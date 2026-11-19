//! Input Pattern preview: renders the current input with the same core code
//! that produces the sent frame (`PatternSpec::render`,
//! `experiment::render_item`) and shows it with the preview-only grayscale
//! map below (PNG export keeps `heatmap::code_rgb`).
//!
//! It runs on its own thread and never touches the device worker. Requests
//! carry a key derived from the full input; only the newest request is
//! rendered, and the GUI shows an image only when its key matches the current
//! input, so an edit or a tab switch never shows a stale pattern.

use slm_core::experiment::{ExperimentItem, ItemSource, render_item};
use slm_core::{CodeFrame, GenerationProfile, PatternSpec, PeriodUnit};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Debug, serde::Serialize)]
pub enum PreviewSource {
    Spec {
        spec: PatternSpec,
        profile: GenerationProfile,
    },
    Item {
        item: ExperimentItem,
        asset: Option<PathBuf>,
    },
}

impl PreviewSource {
    pub fn save_to(&self, path: &std::path::Path, png: bool) -> Result<(), String> {
        let (rendered, max_code) = self.rendered()?;
        if !png {
            let csv = slm_core::csv::write_santec_csv(rendered.code());
            return slm_core::experiment::write_atomic(path, csv.as_bytes());
        }
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let staged = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        slm_core::heatmap::write_heatmap(
            &rendered,
            slm_core::heatmap::HeatmapKind::Code,
            max_code,
            staged.path(),
        )?;
        staged.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }

    pub fn rendered(&self) -> Result<(slm_core::Rendered, u16), String> {
        match self {
            Self::Spec { spec, profile } => Ok((spec.render(profile)?, profile.max_code)),
            Self::Item { item, asset } => {
                Ok((render_item(item, asset.as_deref())?, item.profile.max_code))
            }
        }
    }

    /// Stable key over everything that affects the rendered codes.
    pub fn key(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        json.hash(&mut hash);
        format!("{:016x}-{}", hash.finish(), json.len())
    }
}

#[derive(Clone, Debug)]
pub struct PreviewImage {
    pub width: u32,
    pub height: u32,
    /// RGB, row-major, row 0 = SLM row 0 (top), column 0 = SLM column 0 (left).
    pub rgb: Vec<u8>,
    /// `CodeFrame::digest` of the rendered frame (same as a send would carry).
    pub digest: String,
    pub min_code: u16,
    pub max_code_used: u16,
    /// Color scale upper bound (device max code, Santec 1023).
    pub scale_max: u16,
    /// Coordinate system of the source pattern (see [`PreviewAxes`]).
    pub axes: PreviewAxes,
}

/// Preview-only grayscale: code 0 is black, the device max code is white,
/// linear and fixed in between (independent of the codes in use).
pub fn gray(code: u16, max_code: u16) -> [u8; 3] {
    let level = (f64::from(code.min(max_code)) * 255. / f64::from(max_code.max(1))).round() as u8;
    [level; 3]
}

pub fn gray_rgb(frame: &CodeFrame, max_code: u16) -> Vec<u8> {
    frame.data.iter().flat_map(|&c| gray(c, max_code)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisUnit {
    /// SLM pixels.
    Px,
    /// Sample plane, `pixel pitch × magnification` per SLM pixel.
    SampleUm,
}

impl AxisUnit {
    pub fn label(self) -> &'static str {
        match self {
            AxisUnit::Px => "px",
            AxisUnit::SampleUm => "µm",
        }
    }
}

/// Display coordinates of the preview image. The origin is always the exact
/// image center (pixel index `((W-1)/2, (H-1)/2)`); axes follow the image
/// columns/rows (+X right, +Y down) and never move or rotate with the
/// pattern's Center X/Y, rotation or kind. Units are px or sample-plane µm.
/// Values are given at pixel centers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewAxes {
    /// Pixel index (column, row) whose center is coordinate 0.
    pub origin_px: [f64; 2],
    pub unit_per_px: f64,
    pub unit: AxisUnit,
    pub width: u32,
    pub height: u32,
}

impl PreviewAxes {
    /// Display axes for a raster: origin fixed at the exact image center,
    /// pixel index `((W-1)/2, (H-1)/2)`, for every pattern kind. A sample
    /// scale that is zero or not finite (extreme optics) falls back to px so
    /// drawing never divides by zero.
    fn centered(profile: &GenerationProfile, unit: AxisUnit) -> Self {
        let (w, h) = (profile.raster.width, profile.raster.height);
        let um = profile.optics.sample_um_per_px();
        let (unit, unit_per_px) = match unit {
            AxisUnit::SampleUm if um.is_finite() && um > 0. => (AxisUnit::SampleUm, um),
            _ => (AxisUnit::Px, 1.),
        };
        Self {
            origin_px: [
                (f64::from(w.max(1)) - 1.) / 2.,
                (f64::from(h.max(1)) - 1.) / 2.,
            ],
            unit_per_px,
            unit,
            width: w,
            height: h,
        }
    }

    /// px axes (Uniform patterns, imported CSVs).
    pub fn raster_center(profile: &GenerationProfile) -> Self {
        Self::centered(profile, AxisUnit::Px)
    }

    /// Px when every period is entered in px; sample µm when any component
    /// uses µm or µm⁻¹. The origin never depends on Center X/Y, rotation or
    /// pattern kind; those only change the generated codes.
    pub fn for_spec(spec: &PatternSpec, profile: &GenerationProfile) -> Self {
        let physical = spec
            .components()
            .iter()
            .any(|c| c.period.unit != PeriodUnit::SlmPx);
        Self::centered(
            profile,
            if physical {
                AxisUnit::SampleUm
            } else {
                AxisUnit::Px
            },
        )
    }

    pub fn for_source(source: &PreviewSource) -> Self {
        match source {
            PreviewSource::Spec { spec, profile } => Self::for_spec(spec, profile),
            PreviewSource::Item { item, .. } => match &item.source {
                ItemSource::Generated { spec } => Self::for_spec(spec, &item.profile),
                ItemSource::ImportedCsv { .. } => Self::raster_center(&item.profile),
            },
        }
    }

    /// Coordinate of pixel index `index` (0 = x, 1 = y), at the pixel center.
    pub fn coord(&self, axis: usize, index: f64) -> f64 {
        (index - self.origin_px[axis]) * self.unit_per_px
    }

    /// Fractional pixel index of a coordinate (inverse of [`Self::coord`]).
    pub fn index(&self, axis: usize, coord: f64) -> f64 {
        coord / self.unit_per_px + self.origin_px[axis]
    }

    /// Coordinates of the first and last pixel centers on an axis.
    pub fn range(&self, axis: usize) -> (f64, f64) {
        let n = if axis == 0 { self.width } else { self.height };
        (
            self.coord(axis, 0.),
            self.coord(axis, f64::from(n.max(1) - 1)),
        )
    }

    /// Fraction across the image (0 = left/top edge, 1 = right/bottom edge)
    /// of a coordinate: pixel `i` spans `[i, i+1)`, its center is `i + 0.5`.
    pub fn fraction(&self, axis: usize, coord: f64) -> f64 {
        let n = if axis == 0 { self.width } else { self.height };
        (self.index(axis, coord) + 0.5) / f64::from(n.max(1))
    }
}

/// "Nice" tick values (1, 2, 5 × 10ⁿ steps) within `[lo, hi]`; always
/// multiples of the step, so 0 is included whenever it is in range.
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> (Vec<f64>, f64) {
    if !(lo.is_finite() && hi.is_finite()) {
        return (Vec::new(), 1.);
    }
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let span = (hi - lo).max(f64::EPSILON);
    let raw = span / target.max(1) as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    // The 1/2/5 × 10ⁿ step whose tick count is closest to `target`.
    let step = [1., 2., 5., 10.]
        .into_iter()
        .map(|m| m * magnitude)
        .min_by(|a, b| {
            let da = (span / a - target as f64).abs();
            let db = (span / b - target as f64).abs();
            da.total_cmp(&db)
        })
        .unwrap_or(magnitude);
    if !(step.is_finite() && step > 0.) {
        return (Vec::new(), 1.);
    }
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    if last.saturating_sub(first) > 1000 {
        return (Vec::new(), step);
    }
    let ticks = (first..=last)
        .map(|k| k as f64 * step)
        .map(|v| if v == 0. { 0. } else { v })
        .collect();
    (ticks, step)
}

/// Tick label with just enough decimals for `step`.
pub fn tick_label(value: f64, step: f64) -> String {
    let decimals = (-step.log10().floor()).clamp(0., 12.) as usize;
    format!("{value:.decimals$}")
}

pub struct PreviewResult {
    pub key: String,
    pub result: Result<PreviewImage, String>,
}

pub struct Previewer {
    requests: Sender<(String, PreviewSource)>,
    results: Receiver<PreviewResult>,
}

pub fn render(source: &PreviewSource) -> Result<PreviewImage, String> {
    let (rendered, scale_max) = source.rendered()?;
    let code = rendered.code();
    Ok(PreviewImage {
        width: code.width,
        height: code.height,
        rgb: gray_rgb(code, scale_max),
        axes: PreviewAxes::for_source(source),
        digest: code.digest(),
        min_code: code.data.iter().copied().min().unwrap_or(0),
        max_code_used: code.max_code(),
        scale_max,
    })
}

impl Previewer {
    pub fn spawn(notify: Box<dyn Fn() + Send>) -> Self {
        let (requests, request_rx) = channel::<(String, PreviewSource)>();
        let (result_tx, results) = channel();
        std::thread::Builder::new()
            .name("slm-preview".into())
            .spawn(move || {
                while let Ok(mut next) = request_rx.recv() {
                    // Skip superseded requests; only the newest input matters.
                    while let Ok(newer) = request_rx.try_recv() {
                        next = newer;
                    }
                    let (key, source) = next;
                    let result = render(&source);
                    if result_tx.send(PreviewResult { key, result }).is_err() {
                        return;
                    }
                    notify();
                }
            })
            .expect("spawn preview thread");
        Self { requests, results }
    }

    pub fn request(&self, key: String, source: PreviewSource) {
        let _ = self.requests.send((key, source));
    }

    pub fn try_results(&self) -> Vec<PreviewResult> {
        self.results.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slm_device::DeviceProfile;

    #[test]
    fn preview_matches_send_path_and_keys_change_with_input() {
        let profile = DeviceProfile::santec_slm200_candidate().generation;
        let a = PreviewSource::Spec {
            spec: PatternSpec::Uniform { code: 512 },
            profile,
        };
        let b = PreviewSource::Spec {
            spec: PatternSpec::Uniform { code: 513 },
            profile,
        };
        assert_ne!(a.key(), b.key());
        assert_eq!(a.key(), a.clone().key());
        let image = render(&a).unwrap();
        let sent = PatternSpec::Uniform { code: 512 }.render(&profile).unwrap();
        assert_eq!(image.digest, sent.code().digest());
        assert_eq!((image.width, image.height), (1920, 1200));
        assert_eq!(
            (image.min_code, image.max_code_used, image.scale_max),
            (512, 512, 1023)
        );
        assert_eq!(image.rgb.len(), 1920 * 1200 * 3);
    }

    use slm_core::experiment::{ExperimentDoc, ExperimentItem};
    use slm_core::{AngleValue, Geometry, PeriodValue, PhaseValue, WaveComponent, Waveform};

    #[test]
    fn saves_the_same_codes_as_csv_and_a_native_size_png() {
        let mut profile = slm_device::DeviceProfile::santec_slm200_candidate().generation;
        profile.raster.width = 8;
        profile.raster.height = 4;
        let spec = PatternSpec::Uniform { code: 512 };
        let source = PreviewSource::Spec {
            spec: spec.clone(),
            profile,
        };
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("pattern.csv");
        let png = dir.path().join("pattern.png");
        source.save_to(&csv, false).unwrap();
        source.save_to(&png, true).unwrap();
        let saved = slm_core::csv::read_santec_csv(
            &std::fs::read_to_string(csv).unwrap(),
            8,
            4,
            profile.max_code,
        )
        .unwrap();
        let rendered = spec.render(&profile).unwrap();
        assert_eq!(saved, *rendered.code());
        let image = image::open(png).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (8, 4));
        assert_eq!(
            image.into_raw(),
            slm_core::heatmap::code_rgb(&saved, profile.max_code)
        );
    }

    fn wave(period: PeriodValue) -> WaveComponent {
        WaveComponent {
            period,
            waveform: Waveform::Ramp,
            initial_phase: PhaseValue::zero(),
        }
    }

    fn single(period: PeriodValue, deg: f64) -> PatternSpec {
        PatternSpec::SingleWave {
            wave: wave(period),
            geometry: Geometry {
                center_offset_px: [-16, 110],
                rotation: AngleValue::degrees(deg),
            },
        }
    }

    fn um(value: f64) -> PeriodValue {
        PeriodValue {
            value,
            unit: PeriodUnit::SampleUm,
        }
    }

    const CENTER: [f64; 2] = [959.5, 599.5];

    #[test]
    fn origin_is_image_center_for_every_pattern() {
        let profile = DeviceProfile::santec_slm200_candidate().generation;
        let spec = single(PeriodValue::px(100.), 0.);
        let axes = PreviewAxes::for_spec(&spec, &profile);
        assert_eq!(axes.origin_px, CENTER);
        assert_eq!((axes.unit, axes.unit_per_px), (AxisUnit::Px, 1.));
        assert_eq!(axes.range(0), (-959.5, 959.5));
        assert_eq!(axes.range(1), (-599.5, 599.5));
        assert_eq!(axes.coord(0, 0.), -959.5);
        assert_eq!(axes.index(0, 0.), 959.5);
        assert_eq!(axes.fraction(0, 0.), 0.5);
        assert_eq!(axes.fraction(1, 0.), 0.5);
        assert_eq!(axes.fraction(0, -959.5), 0.5 / 1920.);
        // Rotation and pattern kind never move the axes.
        assert_eq!(
            PreviewAxes::for_spec(&single(PeriodValue::px(100.), 37.), &profile),
            axes
        );
        let multi = PatternSpec::MultiWave {
            components: vec![wave(PeriodValue::px(50.)), wave(PeriodValue::px(80.))],
            geometry: Geometry {
                center_offset_px: [300, -200],
                rotation: AngleValue::degrees(90.),
            },
            stripe_width_px: Some(2),
            reverse_band_order: true,
        };
        assert_eq!(PreviewAxes::for_spec(&multi, &profile), axes);
        assert_eq!(
            PreviewAxes::for_spec(&PatternSpec::Uniform { code: 7 }, &profile),
            axes
        );
    }

    #[test]
    fn center_input_changes_codes_but_not_the_origin() {
        let profile = DeviceProfile::santec_slm200_candidate().generation;
        let at = |offset: [i32; 2]| PatternSpec::SingleWave {
            wave: wave(PeriodValue::px(100.)),
            geometry: Geometry {
                center_offset_px: offset,
                rotation: AngleValue::degrees(0.),
            },
        };
        let a = render(&PreviewSource::Spec {
            spec: at([-16, 110]),
            profile,
        })
        .unwrap();
        let b = render(&PreviewSource::Spec {
            spec: at([40, -25]),
            profile,
        })
        .unwrap();
        assert_ne!(a.digest, b.digest, "Center X/Y still drives generation");
        assert_eq!(a.axes, b.axes, "but the display origin stays put");
        assert_eq!(a.axes.origin_px, CENTER);
        // The renderer's own origin moved: the ramp phase is 0 (code 136)
        // at x = 0, i.e. column 960 - dx.
        let code = at([-16, 110]).render(&profile).unwrap().into_code();
        assert_eq!(code.get(976, 0), 136);
        let code = at([40, -25]).render(&profile).unwrap().into_code();
        assert_eq!(code.get(920, 0), 136);
    }

    #[test]
    fn unit_follows_period_entry_with_the_same_origin() {
        let profile = DeviceProfile::santec_slm200_candidate().generation;
        let px = render(&PreviewSource::Spec {
            spec: single(PeriodValue::px(75.), 0.),
            profile,
        })
        .unwrap();
        // 75 px × 8 µm / 75 = 8 µm: the same physical period entered in µm.
        let um_spec = single(um(8.), 0.);
        let um_axes = PreviewAxes::for_spec(&um_spec, &profile);
        assert_eq!(um_axes.origin_px, px.axes.origin_px);
        assert_eq!(
            (px.axes.unit, um_axes.unit),
            (AxisUnit::Px, AxisUnit::SampleUm)
        );
        assert!((um_axes.unit_per_px - 8. / 75.).abs() < 1e-15);
        assert!((um_axes.coord(0, 0.) - (-959.5 * 8. / 75.)).abs() < 1e-9);
        let q = PeriodValue {
            value: 0.5,
            unit: PeriodUnit::SampleWavevector,
        };
        assert_eq!(
            PreviewAxes::for_spec(&single(q, 0.), &profile).unit,
            AxisUnit::SampleUm
        );
        let multi = |second: PeriodValue| PatternSpec::MultiWave {
            components: vec![wave(PeriodValue::px(50.)), wave(second)],
            geometry: Geometry {
                center_offset_px: [3, -4],
                rotation: AngleValue::degrees(0.),
            },
            stripe_width_px: None,
            reverse_band_order: false,
        };
        assert_eq!(
            PreviewAxes::for_spec(&multi(PeriodValue::px(80.)), &profile).unit,
            AxisUnit::Px
        );
        let mixed = PreviewAxes::for_spec(&multi(um(8.)), &profile);
        assert_eq!((mixed.unit, mixed.origin_px), (AxisUnit::SampleUm, CENTER));
    }

    #[test]
    fn uniform_and_csv_use_exact_raster_center() {
        let profile = DeviceProfile::santec_slm200_candidate().generation;
        let axes = PreviewAxes::for_spec(&PatternSpec::Uniform { code: 3 }, &profile);
        assert_eq!(axes.origin_px, [959.5, 599.5]);
        assert_eq!(axes.range(0), (-959.5, 959.5));
        assert_eq!(axes.range(1), (-599.5, 599.5));
        assert_eq!(axes.fraction(0, 0.), 0.5);
        assert_eq!(axes.fraction(1, 0.), 0.5);

        // Experiment items: CSV → raster center px; generated → the item's
        // own profile, not the current one.
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("a.csv");
        std::fs::write(
            &csv,
            slm_core::csv::write_santec_csv(&CodeFrame::uniform(1920, 1200, 5)),
        )
        .unwrap();
        let mut doc = ExperimentDoc::new("x");
        doc.import_csv(&csv, profile).unwrap();
        let item = doc.experiment.items[0].clone();
        let source = PreviewSource::Item {
            asset: doc.asset_path(&item),
            item,
        };
        assert_eq!(PreviewAxes::for_source(&source).origin_px, [959.5, 599.5]);
        let mut other = profile;
        other.optics.pixel_pitch_um = 4.;
        let item = ExperimentItem::generated("x".into(), single(um(10.), 0.), other);
        let axes = PreviewAxes::for_source(&PreviewSource::Item { item, asset: None });
        assert!((axes.unit_per_px - 4. / 75.).abs() < 1e-15);
    }

    #[test]
    fn extreme_optics_and_ranges_do_not_panic() {
        let mut profile = DeviceProfile::santec_slm200_candidate().generation;
        profile.optics.magnification = f64::MAX;
        let axes = PreviewAxes::for_spec(&single(um(10.), 0.), &profile);
        assert_eq!((axes.unit, axes.unit_per_px), (AxisUnit::Px, 1.));
        profile.optics.magnification = 1e-320;
        let axes = PreviewAxes::for_spec(&single(um(10.), 0.), &profile);
        let (lo, hi) = axes.range(0);
        assert!(lo.is_finite() && hi.is_finite() && axes.fraction(0, 0.).is_finite());
        let _ = nice_ticks(lo, hi, 8);
        assert!(nice_ticks(f64::NAN, 1., 5).0.is_empty());
        assert!(nice_ticks(0., f64::INFINITY, 5).0.is_empty());
        assert!(nice_ticks(-1e300, 1e300, 5).0.len() <= 1001);
        let _ = tick_label(1., 1e-320);
    }

    #[test]
    fn ticks_include_negative_zero_positive() {
        let (ticks, step) = nice_ticks(-976., 943., 8);
        assert_eq!(step, 200.);
        assert_eq!(ticks.first(), Some(&-800.));
        assert_eq!(ticks.last(), Some(&800.));
        assert!(ticks.contains(&0.));
        assert_eq!(tick_label(-800., step), "-800");
        let (ticks, step) = nice_ticks(-104.1, 100.6, 6);
        assert_eq!(step, 50.);
        assert!(
            ticks.contains(&0.) && ticks.iter().any(|t| *t < 0.) && ticks.iter().any(|t| *t > 0.)
        );
        assert_eq!(tick_label(0.25, 0.05), "0.25");
    }

    #[test]
    fn grayscale_is_black_to_white_linear_and_fixed() {
        assert_eq!(gray(0, 1023), [0, 0, 0]);
        assert_eq!(gray(1023, 1023), [255, 255, 255]);
        assert_eq!(gray(512, 1023), [128, 128, 128]);
        assert_eq!(gray(256, 1023), [64, 64, 64]);
        // Fixed to the device max, not to the codes in use; out of range clamps.
        assert_eq!(gray(887, 1023), [221, 221, 221]);
        assert_eq!(gray(2000, 1023), [255, 255, 255]);
        let frame = CodeFrame {
            width: 3,
            height: 1,
            data: vec![0, 512, 1023],
        };
        assert_eq!(
            gray_rgb(&frame, 1023),
            vec![0, 0, 0, 128, 128, 128, 255, 255, 255]
        );
    }
}
