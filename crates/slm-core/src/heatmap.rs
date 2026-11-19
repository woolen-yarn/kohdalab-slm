//! Heatmap PNG export at native SLM resolution (one image pixel per SLM pixel).
//!
//! - `Phase`: φ in [0, 2π) with a cyclic color map.
//! - `Kerr`: sin φ in [−1, 1] with a diverging map (Kerr-equivalent profile).
//! - `Code`: device code in [0, max_code] with a sequential map.
//!
//! Raw-code patterns (Uniform, imported CSV) have only `Code`.

use crate::frame::{CodeFrame, PhaseFrame, Rendered};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HeatmapKind {
    Phase,
    Kerr,
    Code,
}

impl HeatmapKind {
    pub const ALL: [HeatmapKind; 3] = [HeatmapKind::Phase, HeatmapKind::Kerr, HeatmapKind::Code];
    pub fn suffix(self) -> &'static str {
        match self {
            HeatmapKind::Phase => "phase",
            HeatmapKind::Kerr => "kerr_sin_phase",
            HeatmapKind::Code => "code",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            HeatmapKind::Phase => "Phase",
            HeatmapKind::Kerr => "Kerr sin(φ)",
            HeatmapKind::Code => "SLM code",
        }
    }
}

/// Kinds available for a rendered frame.
pub fn available(rendered: &Rendered) -> &'static [HeatmapKind] {
    match rendered {
        Rendered::Code(_) => &[HeatmapKind::Code],
        Rendered::Phase { .. } => &HeatmapKind::ALL,
    }
}

type Rgb = [u8; 3];

fn lerp_map(stops: &[(f64, Rgb)], t: f64) -> Rgb {
    let t = t.clamp(0., 1.);
    for pair in stops.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0. };
            let mix =
                |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * f).round() as u8;
            return [mix(c0[0], c1[0]), mix(c0[1], c1[1]), mix(c0[2], c1[2])];
        }
    }
    stops.last().map_or([0; 3], |s| s.1)
}

/// Cyclic: equal colors at 0 and 2π (twilight-like).
const CYCLIC: [(f64, Rgb); 5] = [
    (0.00, [226, 217, 226]),
    (0.25, [94, 128, 189]),
    (0.50, [47, 20, 54]),
    (0.75, [175, 85, 72]),
    (1.00, [226, 217, 226]),
];
/// Diverging blue–white–red (coolwarm-like).
const DIVERGING: [(f64, Rgb); 3] = [
    (0.0, [59, 76, 192]),
    (0.5, [221, 221, 221]),
    (1.0, [180, 4, 38]),
];
/// Sequential (viridis-like).
const SEQUENTIAL: [(f64, Rgb); 5] = [
    (0.00, [68, 1, 84]),
    (0.25, [59, 82, 139]),
    (0.50, [33, 145, 140]),
    (0.75, [94, 201, 98]),
    (1.00, [253, 231, 37]),
];

pub fn phase_rgb(phase: &PhaseFrame) -> Vec<u8> {
    phase
        .data
        .iter()
        .flat_map(|&p| lerp_map(&CYCLIC, p / TAU))
        .collect()
}

pub fn kerr_rgb(phase: &PhaseFrame) -> Vec<u8> {
    phase
        .data
        .iter()
        .flat_map(|&p| lerp_map(&DIVERGING, (p.sin() + 1.) / 2.))
        .collect()
}

pub fn code_rgb(code: &CodeFrame, max_code: u16) -> Vec<u8> {
    let max = f64::from(max_code.max(1));
    code.data
        .iter()
        .flat_map(|&c| lerp_map(&SEQUENTIAL, f64::from(c) / max))
        .collect()
}

pub fn write_heatmap(
    rendered: &Rendered,
    kind: HeatmapKind,
    max_code: u16,
    path: &Path,
) -> Result<(), String> {
    let code = rendered.code();
    let rgb = match (kind, rendered.phase()) {
        (HeatmapKind::Code, _) => code_rgb(code, max_code),
        (HeatmapKind::Phase, Some(phase)) => phase_rgb(phase),
        (HeatmapKind::Kerr, Some(phase)) => kerr_rgb(phase),
        (_, None) => {
            return Err(format!(
                "{} heatmap needs a generated phase; raw-code patterns export the code heatmap only",
                kind.label()
            ));
        }
    };
    let image = image::RgbImage::from_raw(code.width, code.height, rgb)
        .ok_or("Heatmap buffer size does not match the frame")?;
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("Write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_map_wraps() {
        assert_eq!(lerp_map(&CYCLIC, 0.), lerp_map(&CYCLIC, 1.));
    }

    #[test]
    fn writes_png_and_rejects_phase_for_raw_codes() {
        let dir = tempfile::tempdir().unwrap();
        let raw = Rendered::Code(CodeFrame::uniform(4, 2, 10));
        let path = dir.path().join("code.png");
        write_heatmap(&raw, HeatmapKind::Code, 1023, &path).unwrap();
        let img = image::open(&path).unwrap();
        assert_eq!((img.width(), img.height()), (4, 2));
        assert!(write_heatmap(&raw, HeatmapKind::Phase, 1023, &dir.path().join("p.png")).is_err());
        assert_eq!(available(&raw), &[HeatmapKind::Code]);
    }
}
