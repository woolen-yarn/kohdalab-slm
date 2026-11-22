//! Heatmap PNG export at native SLM resolution (one image pixel per SLM pixel).
//!
//! Every kind uses the selected [`Colormap`] over a fixed scale that never
//! depends on the values in the frame:
//!
//! - `Phase`: φ over [0, 2π].
//! - `Kerr`: sin φ over [−1, 1] (Kerr-equivalent profile).
//! - `Code`: device code over [0, max_code].
//!
//! Raw-code patterns (Uniform, imported CSV) have only `Code`. Colors are
//! for viewing only: codes, CSVs, frame digests and the encoded output never
//! depend on the colormap.

use crate::frame::{CodeFrame, PhaseFrame, Rendered};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};
use std::path::Path;

/// Color scheme for previews and heatmap PNGs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Colormap {
    /// Black at the scale minimum, white at the maximum, linear in between.
    #[default]
    Grayscale,
    /// matplotlib `rainbow`: purple → blue → green → yellow → red.
    Rainbow,
    /// matplotlib `viridis` (perceptually uniform), 256-entry reference
    /// table via `colorous::VIRIDIS`.
    Viridis,
}

impl Colormap {
    pub const ALL: [Colormap; 3] = [Colormap::Grayscale, Colormap::Rainbow, Colormap::Viridis];

    pub fn label(self) -> &'static str {
        match self {
            Colormap::Grayscale => "Grayscale",
            Colormap::Rainbow => "Rainbow",
            Colormap::Viridis => "Viridis",
        }
    }

    /// Color at fraction `t` of the scale; clamped to [0, 1], non-finite → 0.
    pub fn rgb(self, t: f64) -> Rgb {
        let t = if t.is_finite() { t.clamp(0., 1.) } else { 0. };
        let byte = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
        match self {
            Colormap::Grayscale => [byte(t); 3],
            // matplotlib `_rainbow_data`: |2x − 0.5|, sin(πx), cos(πx/2).
            Colormap::Rainbow => [
                byte((2. * t - 0.5).abs()),
                byte((PI * t).sin()),
                byte((PI * t / 2.).cos()),
            ],
            Colormap::Viridis => {
                let c = colorous::VIRIDIS.eval_continuous(t);
                [c.r, c.g, c.b]
            }
        }
    }
}

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

pub type Rgb = [u8; 3];

/// φ over the fixed scale [0, 2π].
pub fn phase_rgb(phase: &PhaseFrame, colormap: Colormap) -> Vec<u8> {
    phase
        .data
        .iter()
        .flat_map(|&p| colormap.rgb(p / TAU))
        .collect()
}

/// sin φ over the fixed scale [−1, 1].
pub fn kerr_rgb(phase: &PhaseFrame, colormap: Colormap) -> Vec<u8> {
    phase
        .data
        .iter()
        .flat_map(|&p| colormap.rgb((p.sin() + 1.) / 2.))
        .collect()
}

/// Device code over the fixed scale [0, max_code].
pub fn code_rgb(code: &CodeFrame, max_code: u16, colormap: Colormap) -> Vec<u8> {
    let max = f64::from(max_code.max(1));
    code.data
        .iter()
        .flat_map(|&c| colormap.rgb(f64::from(c) / max))
        .collect()
}

pub fn write_heatmap(
    rendered: &Rendered,
    kind: HeatmapKind,
    max_code: u16,
    colormap: Colormap,
    path: &Path,
) -> Result<(), String> {
    let code = rendered.code();
    let rgb = match (kind, rendered.phase()) {
        (HeatmapKind::Code, _) => code_rgb(code, max_code, colormap),
        (HeatmapKind::Phase, Some(phase)) => phase_rgb(phase, colormap),
        (HeatmapKind::Kerr, Some(phase)) => kerr_rgb(phase, colormap),
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
    fn colormap_endpoints_and_reference_values() {
        assert_eq!(Colormap::default(), Colormap::Grayscale);
        assert_eq!(Colormap::Grayscale.rgb(0.), [0, 0, 0]);
        assert_eq!(Colormap::Grayscale.rgb(1.), [255, 255, 255]);
        assert_eq!(Colormap::Grayscale.rgb(512. / 1023.), [128, 128, 128]);
        // matplotlib viridis: #440154, #21918c (0.5), #fde725.
        assert_eq!(Colormap::Viridis.rgb(0.), [0x44, 0x01, 0x54]);
        assert_eq!(Colormap::Viridis.rgb(1.), [0xfd, 0xe7, 0x25]);
        let mid = Colormap::Viridis.rgb(0.5);
        for (got, want) in mid.iter().zip([0x21u8, 0x91, 0x8c]) {
            assert!(got.abs_diff(want) <= 1, "{mid:?}");
        }
        // matplotlib rainbow: (0.5, 0, 1) → (1, 0, 0).
        assert_eq!(Colormap::Rainbow.rgb(0.), [128, 0, 255]);
        assert_eq!(Colormap::Rainbow.rgb(1.), [255, 0, 0]);
        for map in Colormap::ALL {
            assert_eq!(map.rgb(-1.), map.rgb(0.));
            assert_eq!(map.rgb(2.), map.rgb(1.));
            assert_eq!(map.rgb(f64::NAN), map.rgb(0.));
        }
        let json = serde_json::to_string(&Colormap::Viridis).unwrap();
        assert_eq!(json, "\"viridis\"");
    }

    #[test]
    fn scales_are_fixed_per_kind() {
        let code = CodeFrame {
            width: 3,
            height: 1,
            data: vec![0, 512, 1023],
        };
        for map in Colormap::ALL {
            let rgb = code_rgb(&code, 1023, map);
            assert_eq!(&rgb[..3], &map.rgb(0.));
            assert_eq!(&rgb[6..], &map.rgb(1.));
        }
        let phase = PhaseFrame {
            width: 2,
            height: 1,
            data: vec![0., PI],
        };
        assert_eq!(
            phase_rgb(&phase, Colormap::Grayscale),
            vec![0, 0, 0, 128, 128, 128]
        );
        // sin 0 = 0 → middle of [−1, 1]; sin(π/2) = 1 → top.
        let kerr = PhaseFrame {
            width: 2,
            height: 1,
            data: vec![0., PI / 2.],
        };
        assert_eq!(
            kerr_rgb(&kerr, Colormap::Grayscale),
            vec![128, 128, 128, 255, 255, 255]
        );
    }

    #[test]
    fn writes_png_and_rejects_phase_for_raw_codes() {
        let dir = tempfile::tempdir().unwrap();
        let raw = Rendered::Code(CodeFrame::uniform(4, 2, 10));
        let path = dir.path().join("code.png");
        for map in Colormap::ALL {
            write_heatmap(&raw, HeatmapKind::Code, 1023, map, &path).unwrap();
            let img = image::open(&path).unwrap().to_rgb8();
            assert_eq!(img.dimensions(), (4, 2));
            assert_eq!(img.get_pixel(0, 0).0, map.rgb(10. / 1023.));
        }
        assert!(
            write_heatmap(
                &raw,
                HeatmapKind::Phase,
                1023,
                Colormap::Grayscale,
                &dir.path().join("p.png")
            )
            .is_err()
        );
        assert_eq!(available(&raw), &[HeatmapKind::Code]);
    }
}
