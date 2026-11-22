//! Pattern specifications and rendering.
//!
//! Coordinates follow the notebook (`SLM.ipynb`): for pixel column `i` and row
//! `j`, `x = i - W/2 + dx`, `y = j - H/2 + dy` (integer division, offsets in
//! px). The rotation applied to the raster is the pattern angle relative to the
//! sample reference axis minus the SLM mounting angle. The wave coordinate is
//! `u = x cos θ + y sin θ` and the band coordinate is `v = -x sin θ + y cos θ`.
//!
//! Notebook compatibility uses
//! `round(phase0 + φ/(2π) · period) mod period`, with NumPy's ties-to-even
//! rounding and floating-point remainder semantics.

use crate::frame::{CodeFrame, PhaseFrame, Rendered};
use crate::units::{AngleUnit, AngleValue, Optics, PeriodValue, PhaseValue};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// Pixel raster of the SLM panel (landscape for Santec SLM-200).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
}

/// How generated phase is converted to digital codes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseMapping {
    /// Old configurations and experiments omit this field.
    #[default]
    Notebook,
    /// Santec SLM-200 manual, Figure 42, with the device calibrated for 2π.
    SantecFullScale,
}

/// Generator-side phase→code mapping.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhaseCalibration {
    #[serde(default)]
    pub mapping: PhaseMapping,
    /// Code offset added before wrapping (`phase0`, notebook `phase0=136`).
    pub phase0: f64,
    /// Number of codes per 2π (`period`, notebook `period=888`).
    pub code_period: u32,
}

/// Everything needed to regenerate a pattern reproducibly. Experiments store a
/// copy per item so a later profile change or another PC regenerates the same
/// codes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerationProfile {
    pub raster: Raster,
    /// Largest valid device code (Santec SLM-200: 1023, 10 bit).
    pub max_code: u16,
    pub optics: Optics,
    pub calibration: PhaseCalibration,
    /// Angle from the sample reference axis to SLM +X [deg]; subtracted from
    /// the pattern angle when rasterizing.
    pub mount_angle_deg: f64,
    /// Multi-wave band width used when a pattern has no override [px].
    pub default_stripe_width_px: u32,
}

impl GenerationProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.raster.width == 0 || self.raster.height == 0 {
            return Err("Raster size must be positive".into());
        }
        self.optics.validate()?;
        if self.calibration.mapping == PhaseMapping::Notebook {
            if self.calibration.code_period == 0 {
                return Err("Code period must be at least 1".into());
            }
            if u32::from(self.max_code) + 1 < self.calibration.code_period {
                return Err(format!(
                    "Code period {} exceeds the device code range 0–{}",
                    self.calibration.code_period, self.max_code
                ));
            }
        }
        if !self.calibration.phase0.is_finite() {
            return Err("phase0 must be finite".into());
        }
        if !self.mount_angle_deg.is_finite() {
            return Err("Mounting angle must be finite".into());
        }
        if self.default_stripe_width_px == 0 {
            return Err("Default band width must be at least 1 px".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Waveform {
    /// Linear phase ramp (blazed).
    #[serde(rename = "ramp")]
    Ramp,
    /// 0/π binary grating.
    #[serde(rename = "binary")]
    Binary,
}

impl Waveform {
    pub fn label(self) -> &'static str {
        match self {
            Waveform::Ramp => "Phase ramp",
            Waveform::Binary => "Binary 0/π",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaveComponent {
    pub period: PeriodValue,
    pub waveform: Waveform,
    pub initial_phase: PhaseValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// Notebook `(delta_Hpx, delta_Vpx)` [px].
    pub center_offset_px: [i32; 2],
    /// Pattern angle relative to the sample reference axis.
    pub rotation: AngleValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PatternSpec {
    /// Every pixel receives the same raw device code (no phase conversion).
    Uniform { code: u16 },
    SingleWave {
        wave: WaveComponent,
        geometry: Geometry,
    },
    /// Components alternate band by band along the band coordinate, in entry
    /// order. `stripe_width_px` overrides the profile default.
    /// `reverse_band_order` counts bands along `−v` instead of `v`; at 90° this
    /// reproduces the notebook's `phi=90` branch (bands ordered by +X).
    MultiWave {
        components: Vec<WaveComponent>,
        geometry: Geometry,
        stripe_width_px: Option<u32>,
        #[serde(default)]
        reverse_band_order: bool,
    },
}

impl PatternSpec {
    pub fn kind_label(&self) -> &'static str {
        match self {
            PatternSpec::Uniform { .. } => "Uniform",
            PatternSpec::SingleWave { .. } => "Single Wave",
            PatternSpec::MultiWave { .. } => "Multi-Wave",
        }
    }
    pub fn components(&self) -> &[WaveComponent] {
        match self {
            PatternSpec::Uniform { .. } => &[],
            PatternSpec::SingleWave { wave, .. } => std::slice::from_ref(wave),
            PatternSpec::MultiWave { components, .. } => components,
        }
    }
    pub fn components_mut(&mut self) -> &mut [WaveComponent] {
        match self {
            PatternSpec::Uniform { .. } => &mut [],
            PatternSpec::SingleWave { wave, .. } => std::slice::from_mut(wave),
            PatternSpec::MultiWave { components, .. } => components,
        }
    }
    pub fn geometry_mut(&mut self) -> Option<&mut Geometry> {
        match self {
            PatternSpec::Uniform { .. } => None,
            PatternSpec::SingleWave { geometry, .. } | PatternSpec::MultiWave { geometry, .. } => {
                Some(geometry)
            }
        }
    }
    pub fn geometry(&self) -> Option<&Geometry> {
        match self {
            PatternSpec::Uniform { .. } => None,
            PatternSpec::SingleWave { geometry, .. } | PatternSpec::MultiWave { geometry, .. } => {
                Some(geometry)
            }
        }
    }

    /// One-line description of the condition with its entry units.
    pub fn summary(&self, optics: &Optics) -> String {
        let wave = |w: &WaveComponent| {
            format!(
                "{} {} {} ({} px), φ0 {}{}",
                w.waveform.label(),
                crate::units::format_number(w.period.value),
                w.period.unit.suffix(),
                w.period.display_in(crate::units::PeriodUnit::SlmPx, optics),
                crate::units::format_number(w.initial_phase.value),
                w.initial_phase.unit.suffix(),
            )
        };
        let geometry = |g: &Geometry| {
            format!(
                "rot {}{}, center ({}, {}) px",
                crate::units::format_number(g.rotation.value),
                g.rotation.unit.suffix(),
                g.center_offset_px[0],
                g.center_offset_px[1]
            )
        };
        match self {
            PatternSpec::Uniform { code } => format!("Uniform code {code}"),
            PatternSpec::SingleWave {
                wave: w,
                geometry: g,
            } => {
                format!("Single: {}; {}", wave(w), geometry(g))
            }
            PatternSpec::MultiWave {
                components,
                geometry: g,
                stripe_width_px,
                reverse_band_order,
            } => format!(
                "Multi ×{} (band {}{}): {}; {}",
                components.len(),
                stripe_width_px.map_or_else(|| "default".into(), |w| format!("{w} px")),
                if *reverse_band_order {
                    ", reversed order"
                } else {
                    ""
                },
                components.iter().map(wave).collect::<Vec<_>>().join(" | "),
                geometry(g)
            ),
        }
    }

    pub fn validate(&self, profile: &GenerationProfile) -> Result<(), String> {
        profile.validate()?;
        match self {
            PatternSpec::Uniform { code } => {
                if *code > profile.max_code {
                    return Err(format!(
                        "Contrast level {code} is outside 0–{}",
                        profile.max_code
                    ));
                }
            }
            PatternSpec::SingleWave { wave, geometry } => {
                validate_wave(wave, &profile.optics)?;
                geometry.rotation.as_degrees()?;
            }
            PatternSpec::MultiWave {
                components,
                geometry,
                stripe_width_px,
                ..
            } => {
                if components.is_empty() {
                    return Err("Multi-Wave needs at least one component".into());
                }
                for (i, wave) in components.iter().enumerate() {
                    validate_wave(wave, &profile.optics)
                        .map_err(|e| format!("Component {}: {e}", i + 1))?;
                }
                if *stripe_width_px == Some(0) {
                    return Err("Band width must be at least 1 px".into());
                }
                geometry.rotation.as_degrees()?;
            }
        }
        Ok(())
    }

    /// Render the full raster. Uniform patterns produce raw codes only.
    pub fn render(&self, profile: &GenerationProfile) -> Result<Rendered, String> {
        self.validate(profile)?;
        let Raster { width, height } = profile.raster;
        match self {
            PatternSpec::Uniform { code } => {
                Ok(Rendered::Code(CodeFrame::uniform(width, height, *code)))
            }
            PatternSpec::SingleWave { wave, geometry } => {
                let (c, s) = rotation(geometry, profile, false)?;
                let wave = PreparedWave::new(wave, &profile.optics)?;
                let phase = raster_phase(profile, geometry, |x, y| {
                    let u = x * c + y * s;
                    wave.phase(u)
                })?;
                let code = phase_to_code(&phase, profile);
                Ok(Rendered::Phase { phase, code })
            }
            PatternSpec::MultiWave {
                components,
                geometry,
                stripe_width_px,
                reverse_band_order,
            } => {
                let (c, s) = rotation(geometry, profile, true)?;
                let reverse = *reverse_band_order;
                let waves = components
                    .iter()
                    .map(|w| PreparedWave::new(w, &profile.optics))
                    .collect::<Result<Vec<_>, _>>()?;
                let width_px =
                    f64::from(stripe_width_px.unwrap_or(profile.default_stripe_width_px));
                let n = waves.len() as f64;
                let phase = raster_phase(profile, geometry, |x, y| {
                    let u = x * c + y * s;
                    // Band coordinate; reversed is its exact negation, so
                    // the order flips without a discontinuity in angle.
                    let v = if reverse {
                        x * s - y * c
                    } else {
                        -x * s + y * c
                    };
                    let band = np_remainder(np_floor_divide(v, width_px), n) as usize;
                    waves[band.min(waves.len() - 1)].phase(u)
                })?;
                let code = phase_to_code(&phase, profile);
                Ok(Rendered::Phase { phase, code })
            }
        }
    }
}

fn validate_wave(wave: &WaveComponent, optics: &Optics) -> Result<(), String> {
    wave.period.validate()?;
    wave.period.period_px(optics)?;
    wave.initial_phase.radians()?;
    Ok(())
}

/// `(cos θ, sin θ)` of the rasterization angle. With `snap`, exact multiples of
/// 90° use exact values, matching the notebook `multi_wave` branches that use
/// `X`/`Y` directly. Single waves use floating-point trigonometry as the
/// notebook's `np.cos(np.radians(phi))` does.
fn rotation(
    geometry: &Geometry,
    profile: &GenerationProfile,
    snap: bool,
) -> Result<(f64, f64), String> {
    let radians = match geometry.rotation.unit {
        AngleUnit::Degree => {
            let degrees = geometry.rotation.value - profile.mount_angle_deg;
            if !degrees.is_finite() {
                return Err("Rotation angle must be finite".into());
            }
            if snap && degrees.rem_euclid(90.) == 0. {
                return Ok(match (degrees / 90.).rem_euclid(4.) as i64 {
                    0 => (1., 0.),
                    1 => (0., 1.),
                    2 => (-1., 0.),
                    _ => (0., -1.),
                });
            }
            degrees.to_radians()
        }
        AngleUnit::Radian => geometry.rotation.value - profile.mount_angle_deg.to_radians(),
    };
    if !radians.is_finite() {
        return Err("Rotation angle must be finite".into());
    }
    Ok((radians.cos(), radians.sin()))
}

struct PreparedWave {
    period_px: Option<f64>,
    waveform: Waveform,
    initial: f64,
}

impl PreparedWave {
    fn new(wave: &WaveComponent, optics: &Optics) -> Result<Self, String> {
        Ok(Self {
            period_px: wave.period.period_px(optics)?,
            waveform: wave.waveform,
            initial: wave.initial_phase.radians()?,
        })
    }
    /// Wrapped phase at wave coordinate `u`.
    fn phase(&self, u: f64) -> f64 {
        let raw = match (self.period_px, self.waveform) {
            (None, _) => 0.,
            // Notebook: (X_rot / period_px) * 2 * np.pi, then % (2π).
            (Some(p), Waveform::Ramp) => np_remainder((u / p) * TAU, TAU),
            // Notebook: where((X_rot // (period_px / 2)) % 2 == 0, 0, π).
            (Some(p), Waveform::Binary) => {
                let parity = np_remainder(np_floor_divide(u, p / 2.), 2.);
                if !parity.is_finite() {
                    return f64::NAN;
                }
                if parity == 0. {
                    0.
                } else {
                    std::f64::consts::PI
                }
            }
        };
        if self.initial == 0. {
            raw
        } else {
            np_remainder(raw + self.initial, TAU)
        }
    }
}

fn raster_phase(
    profile: &GenerationProfile,
    geometry: &Geometry,
    phase_at: impl Fn(f64, f64) -> f64,
) -> Result<PhaseFrame, String> {
    let Raster { width, height } = profile.raster;
    let x0 = -(i64::from(width) / 2) + i64::from(geometry.center_offset_px[0]);
    let y0 = -(i64::from(height) / 2) + i64::from(geometry.center_offset_px[1]);
    let mut data = Vec::with_capacity(width as usize * height as usize);
    for j in 0..i64::from(height) {
        let y = (j + y0) as f64;
        for i in 0..i64::from(width) {
            let phase = phase_at((i + x0) as f64, y);
            if !phase.is_finite() {
                return Err("Pattern parameters produced a non-finite phase. Check the wave period and phase offset.".into());
            }
            data.push(phase);
        }
    }
    Ok(PhaseFrame {
        width,
        height,
        data,
    })
}

/// Convert generated radians to the selected calibration's raw device codes.
pub fn phase_to_code(phase: &PhaseFrame, profile: &GenerationProfile) -> CodeFrame {
    let period = f64::from(profile.calibration.code_period);
    let phase0 = profile.calibration.phase0;
    let max_code = f64::from(profile.max_code);
    let data = match profile.calibration.mapping {
        PhaseMapping::Notebook => phase
            .data
            .iter()
            .map(|&p| np_remainder((phase0 + (p / TAU) * period).round_ties_even(), period) as u16)
            .collect(),
        PhaseMapping::SantecFullScale => phase
            .data
            .iter()
            .map(|&p| (p.rem_euclid(TAU) / TAU * max_code).round() as u16)
            .collect(),
    };
    CodeFrame {
        width: phase.width,
        height: phase.height,
        data,
    }
}

/// NumPy `np.remainder` for floats (result takes the divisor's sign).
pub fn np_remainder(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0. {
        if (b < 0.) != (m < 0.) { m + b } else { m }
    } else {
        0f64.copysign(b)
    }
}

/// NumPy `np.floor_divide` for floats (`npy_divmod`).
pub fn np_floor_divide(a: f64, b: f64) -> f64 {
    let m = a % b;
    let mut div = (a - m) / b;
    if m != 0. && (b < 0.) != (m < 0.) {
        div -= 1.;
    }
    if div != 0. {
        let mut floordiv = div.floor();
        if div - floordiv > 0.5 {
            floordiv += 1.;
        }
        floordiv
    } else {
        0f64.copysign(a / b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{PeriodUnit, PhaseUnit};

    pub(crate) fn profile(width: u32, height: u32) -> GenerationProfile {
        GenerationProfile {
            raster: Raster { width, height },
            max_code: 1023,
            optics: Optics {
                pixel_pitch_um: 8.,
                magnification: 1. / 75.,
            },
            calibration: PhaseCalibration {
                mapping: Default::default(),
                phase0: 136.,
                code_period: 888,
            },
            mount_angle_deg: 0.,
            default_stripe_width_px: 1,
        }
    }

    #[test]
    fn extremely_small_period_rejects_non_finite_phase() {
        for waveform in [Waveform::Ramp, Waveform::Binary] {
            let pattern = PatternSpec::SingleWave {
                wave: wave(f64::from_bits(1), waveform),
                geometry: geometry(0.),
            };
            assert!(pattern.render(&profile(4, 2)).is_err());
        }
    }

    #[test]
    fn santec_full_scale_maps_phase_to_ten_bit_codes() {
        let mut profile = profile(8, 1);
        profile.calibration.code_period = 0;
        assert!(profile.validate().is_err());
        profile.calibration.mapping = PhaseMapping::SantecFullScale;
        assert!(profile.validate().is_ok());
        let phase = PhaseFrame {
            width: 8,
            height: 1,
            data: [0., 0.25, 0.5, 0.75, 0.9999, 1., -0.25, 1.25]
                .map(|cycle| cycle * TAU)
                .into(),
        };
        assert_eq!(
            phase_to_code(&phase, &profile).data,
            [0, 256, 512, 767, 1023, 0, 767, 256]
        );
    }

    fn wave(period_px: f64, waveform: Waveform) -> WaveComponent {
        WaveComponent {
            period: PeriodValue::px(period_px),
            waveform,
            initial_phase: PhaseValue::zero(),
        }
    }

    fn geometry(deg: f64) -> Geometry {
        Geometry {
            center_offset_px: [0, 0],
            rotation: AngleValue::degrees(deg),
        }
    }

    #[test]
    fn numpy_remainder_and_floor_divide() {
        assert_eq!(np_remainder(-1., 3.), 2.);
        assert_eq!(np_floor_divide(-1., 2.), -1.);
        assert_eq!(np_floor_divide(3., 2.), 1.);
        assert_eq!(np_floor_divide(-4., 2.), -2.);
        assert_eq!((2.5f64).round_ties_even(), 2.);
        assert_eq!((3.5f64).round_ties_even(), 4.);
    }

    #[test]
    fn uniform_is_raw_code_and_range_checked() {
        let p = profile(4, 3);
        let r = PatternSpec::Uniform { code: 1023 }.render(&p).unwrap();
        assert!(r.phase().is_none());
        assert_eq!(r.code().uniform_code(), Some(1023));
        assert!(PatternSpec::Uniform { code: 1024 }.render(&p).is_err());
    }

    #[test]
    fn zero_wavevector_is_coordinate_independent() {
        let p = profile(16, 8);
        for waveform in [Waveform::Ramp, Waveform::Binary] {
            let spec = PatternSpec::SingleWave {
                wave: WaveComponent {
                    period: PeriodValue {
                        value: 0.,
                        unit: PeriodUnit::SampleWavevector,
                    },
                    waveform,
                    initial_phase: PhaseValue {
                        value: 0.5,
                        unit: PhaseUnit::Pi,
                    },
                },
                geometry: geometry(30.),
            };
            let r = spec.render(&p).unwrap();
            // φ = π/2 everywhere → round(136 + 0.25·888) = 358.
            assert_eq!(r.code().uniform_code(), Some(358));
        }
    }

    #[test]
    fn ramp_matches_hand_computed_notebook_values() {
        let p = profile(8, 2);
        let r = PatternSpec::SingleWave {
            wave: wave(4., Waveform::Ramp),
            geometry: geometry(0.),
        }
        .render(&p)
        .unwrap();
        // x = i - 4 → -4..3; phase/(2π) = (x/4) mod 1.
        let expected: Vec<u16> = (-4..4)
            .map(|x: i32| {
                let frac = (f64::from(x) / 4.).rem_euclid(1.);
                ((136. + frac * 888.).round_ties_even() % 888.) as u16
            })
            .collect();
        assert_eq!(&r.code().data[..8], &expected[..]);
    }

    #[test]
    fn binary_grating_is_two_level() {
        let p = profile(12, 1);
        let r = PatternSpec::SingleWave {
            wave: wave(4., Waveform::Binary),
            geometry: geometry(0.),
        }
        .render(&p)
        .unwrap();
        // x = -6..5, floor(x/2) % 2: -6,-5→-3→1(π); -4,-3→-2→0; -2,-1→-1→1; 0,1→0; 2,3→1; 4,5→2→0
        let pi = 136 + 444; // (136 + 888/2) mod 888
        assert_eq!(
            r.code().data,
            vec![pi, pi, 136, 136, pi, pi, 136, 136, pi, pi, 136, 136]
        );
    }

    #[test]
    fn multi_wave_bands_alternate_by_row_at_zero_degrees() {
        let p = profile(6, 6);
        let comps = vec![wave(2., Waveform::Ramp), wave(3., Waveform::Ramp)];
        for width in [1u32, 2] {
            let r = PatternSpec::MultiWave {
                components: comps.clone(),
                geometry: geometry(0.),
                stripe_width_px: Some(width),
                reverse_band_order: false,
            }
            .render(&p)
            .unwrap();
            for j in 0..6i32 {
                let y = j - 3;
                let band = (y.div_euclid(width as i32)).rem_euclid(2) as usize;
                for i in 0..6i32 {
                    let x = f64::from(i - 3);
                    let period = [2., 3.][band];
                    let ph = np_remainder((x / period) * TAU, TAU);
                    let code =
                        np_remainder((136. + ph / TAU * 888.).round_ties_even(), 888.) as u16;
                    assert_eq!(
                        r.code().get(i as u32, j as u32),
                        code,
                        "w={width} i={i} j={j}"
                    );
                }
            }
        }
    }

    #[test]
    fn reverse_band_order_is_saved_and_defaults_off() {
        let spec = PatternSpec::MultiWave {
            components: vec![wave(2., Waveform::Ramp), wave(3., Waveform::Ramp)],
            geometry: geometry(90.),
            stripe_width_px: Some(2),
            reverse_band_order: true,
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"reverse_band_order\":true"), "{json}");
        assert_eq!(serde_json::from_str::<PatternSpec>(&json).unwrap(), spec);
        let old = json.replace(",\"reverse_band_order\":true", "");
        let PatternSpec::MultiWave {
            reverse_band_order, ..
        } = serde_json::from_str::<PatternSpec>(&old).unwrap()
        else {
            panic!("multi-wave expected")
        };
        assert!(!reverse_band_order);
        // Reversal changes the band layout but not the per-band phase law.
        let p = profile(8, 4);
        let normal = PatternSpec::MultiWave {
            components: vec![wave(2., Waveform::Ramp), wave(3., Waveform::Ramp)],
            geometry: geometry(90.),
            stripe_width_px: Some(2),
            reverse_band_order: false,
        };
        assert_ne!(
            normal.render(&p).unwrap().code(),
            spec.render(&p).unwrap().code()
        );
    }

    #[test]
    fn mount_angle_is_subtracted() {
        let mut p = profile(10, 10);
        let a = PatternSpec::SingleWave {
            wave: wave(5., Waveform::Ramp),
            geometry: geometry(0.),
        }
        .render(&p)
        .unwrap();
        p.mount_angle_deg = 30.;
        let b = PatternSpec::SingleWave {
            wave: wave(5., Waveform::Ramp),
            geometry: geometry(30.),
        }
        .render(&p)
        .unwrap();
        assert_eq!(a.code(), b.code());
    }
}
