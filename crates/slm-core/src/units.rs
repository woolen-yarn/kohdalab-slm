//! Period / wavevector, phase and angle quantities with their entry units.
//!
//! `px` periods are on the SLM plane. `µm` periods and `µm⁻¹` wavevectors are
//! on the sample plane: with pixel pitch `p` [µm/px] and demagnification `M`,
//! `Λ_sample = M · p · Λ_px` and `q = 2π / Λ_sample`.

use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Optical conversion between the SLM pixel grid and the sample plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Optics {
    /// SLM pixel pitch [µm/px].
    pub pixel_pitch_um: f64,
    /// Sample-plane magnification `M` (e.g. 1/75 for a 75× demagnification).
    pub magnification: f64,
}

impl Optics {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.pixel_pitch_um.is_finite() && self.pixel_pitch_um > 0.) {
            return Err("Pixel pitch must be a positive number of µm".into());
        }
        if !(self.magnification.is_finite() && self.magnification > 0.) {
            return Err("Magnification must be positive".into());
        }
        Ok(())
    }
    /// Sample-plane length of one SLM pixel [µm].
    pub fn sample_um_per_px(&self) -> f64 {
        self.magnification * self.pixel_pitch_um
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeriodUnit {
    /// Period on the SLM plane [px].
    #[serde(rename = "px")]
    SlmPx,
    /// Period on the sample plane [µm].
    #[serde(rename = "um")]
    SampleUm,
    /// Wavevector on the sample plane [µm⁻¹].
    #[serde(rename = "per_um")]
    SampleWavevector,
}

impl PeriodUnit {
    pub const ALL: [PeriodUnit; 3] = [
        PeriodUnit::SlmPx,
        PeriodUnit::SampleUm,
        PeriodUnit::SampleWavevector,
    ];
    pub fn label(self) -> &'static str {
        match self {
            PeriodUnit::SlmPx => "Period [px]",
            PeriodUnit::SampleUm => "Period [µm]",
            PeriodUnit::SampleWavevector => "Wavevector [µm⁻¹]",
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            PeriodUnit::SlmPx => "px",
            PeriodUnit::SampleUm => "µm",
            PeriodUnit::SampleWavevector => "µm⁻¹",
        }
    }
}

/// A period or wavevector exactly as entered, with its unit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodValue {
    pub value: f64,
    pub unit: PeriodUnit,
}

impl PeriodValue {
    pub fn px(value: f64) -> Self {
        Self {
            value,
            unit: PeriodUnit::SlmPx,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.value.is_finite() {
            return Err(format!("{} must be a finite number", self.unit.label()));
        }
        match self.unit {
            PeriodUnit::SlmPx | PeriodUnit::SampleUm if self.value <= 0. => {
                Err(format!("{} must be greater than 0", self.unit.label()))
            }
            PeriodUnit::SampleWavevector if self.value < 0. => {
                Err("Wavevector must be 0 or greater".into())
            }
            _ => Ok(()),
        }
    }
    /// Sample-plane wavevector [µm⁻¹]; `0` means a coordinate-independent phase.
    pub fn wavevector_per_um(&self, optics: &Optics) -> Result<f64, String> {
        self.validate()?;
        optics.validate()?;
        Ok(match self.unit {
            PeriodUnit::SlmPx => TAU / (self.value * optics.sample_um_per_px()),
            PeriodUnit::SampleUm => TAU / self.value,
            PeriodUnit::SampleWavevector => self.value,
        })
    }
    /// SLM-plane period [px]. `None` is an infinite period (`q = 0`).
    ///
    /// The `px` entry is returned unchanged, so a notebook period such as
    /// `200 px` reaches the generator bit-for-bit.
    pub fn period_px(&self, optics: &Optics) -> Result<Option<f64>, String> {
        self.validate()?;
        optics.validate()?;
        Ok(match self.unit {
            PeriodUnit::SlmPx => Some(self.value),
            PeriodUnit::SampleUm => Some(self.value / optics.sample_um_per_px()),
            PeriodUnit::SampleWavevector if self.value == 0. => None,
            PeriodUnit::SampleWavevector => Some(TAU / self.value / optics.sample_um_per_px()),
        })
    }
    /// Express the same physical condition in another entry unit.
    pub fn convert(&self, to: PeriodUnit, optics: &Optics) -> Result<PeriodValue, String> {
        if to == self.unit {
            return Ok(*self);
        }
        let q = self.wavevector_per_um(optics)?;
        let value = match to {
            PeriodUnit::SampleWavevector => q,
            _ if q == 0. => {
                return Err(
                    "q = 0 has an infinite period; enter a nonzero wavevector before switching to a period unit"
                        .into(),
                );
            }
            PeriodUnit::SampleUm => TAU / q,
            PeriodUnit::SlmPx => TAU / q / optics.sample_um_per_px(),
        };
        Ok(PeriodValue { value, unit: to })
    }
    /// Value in `unit` for display; `∞` for a period at `q = 0`.
    pub fn display_in(&self, unit: PeriodUnit, optics: &Optics) -> String {
        match self.convert(unit, optics) {
            Ok(v) => format_number(v.value),
            Err(_) if self.validate().is_ok() && optics.validate().is_ok() => "∞".into(),
            Err(_) => "—".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PhaseUnit {
    #[serde(rename = "deg")]
    Degree,
    #[serde(rename = "rad")]
    Radian,
    /// Multiples of π.
    #[serde(rename = "pi")]
    Pi,
}

impl PhaseUnit {
    pub const ALL: [PhaseUnit; 3] = [PhaseUnit::Degree, PhaseUnit::Radian, PhaseUnit::Pi];
    pub fn suffix(self) -> &'static str {
        match self {
            PhaseUnit::Degree => "°",
            PhaseUnit::Radian => "rad",
            PhaseUnit::Pi => "π",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhaseValue {
    pub value: f64,
    pub unit: PhaseUnit,
}

impl PhaseValue {
    pub fn zero() -> Self {
        Self {
            value: 0.,
            unit: PhaseUnit::Degree,
        }
    }
    pub fn radians(&self) -> Result<f64, String> {
        if !self.value.is_finite() {
            return Err("Initial phase must be a finite number".into());
        }
        Ok(match self.unit {
            PhaseUnit::Degree => self.value.to_radians(),
            PhaseUnit::Radian => self.value,
            PhaseUnit::Pi => self.value * PI,
        })
    }
    pub fn convert(&self, to: PhaseUnit) -> Result<PhaseValue, String> {
        if to == self.unit {
            return Ok(*self);
        }
        let rad = self.radians()?;
        let value = match to {
            PhaseUnit::Degree => rad.to_degrees(),
            PhaseUnit::Radian => rad,
            PhaseUnit::Pi => rad / PI,
        };
        Ok(PhaseValue { value, unit: to })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AngleUnit {
    #[serde(rename = "deg")]
    Degree,
    #[serde(rename = "rad")]
    Radian,
}

impl AngleUnit {
    pub const ALL: [AngleUnit; 2] = [AngleUnit::Degree, AngleUnit::Radian];
    pub fn suffix(self) -> &'static str {
        match self {
            AngleUnit::Degree => "°",
            AngleUnit::Radian => "rad",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AngleValue {
    pub value: f64,
    pub unit: AngleUnit,
}

impl AngleValue {
    pub fn degrees(value: f64) -> Self {
        Self {
            value,
            unit: AngleUnit::Degree,
        }
    }
    pub fn as_degrees(&self) -> Result<f64, String> {
        if !self.value.is_finite() {
            return Err("Rotation angle must be a finite number".into());
        }
        Ok(match self.unit {
            AngleUnit::Degree => self.value,
            AngleUnit::Radian => self.value.to_degrees(),
        })
    }
    pub fn convert(&self, to: AngleUnit) -> Result<AngleValue, String> {
        if to == self.unit {
            return Ok(*self);
        }
        let value = match to {
            AngleUnit::Degree => self.as_degrees()?,
            AngleUnit::Radian => self.as_degrees()?.to_radians(),
        };
        Ok(AngleValue { value, unit: to })
    }
}

/// Compact, round-trippable display: up to 10 significant digits.
pub fn format_number(value: f64) -> String {
    if value == 0. {
        return "0".into();
    }
    let magnitude = value.abs().log10().floor() as i32;
    if !(-4..10).contains(&magnitude) {
        return format!("{value:.6e}");
    }
    let decimals = (9 - magnitude).clamp(0, 12) as usize;
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn optics() -> Optics {
        Optics {
            pixel_pitch_um: 8.,
            magnification: 1. / 75.,
        }
    }

    #[test]
    fn period_conversion_keeps_physical_condition() {
        let o = optics();
        let px = PeriodValue::px(75.);
        // 75 px × 8 µm/px / 75 = 8 µm on the sample.
        let um = px.convert(PeriodUnit::SampleUm, &o).unwrap();
        assert!((um.value - 8.).abs() < 1e-12);
        let q = um.convert(PeriodUnit::SampleWavevector, &o).unwrap();
        assert!((q.value - TAU / 8.).abs() < 1e-12);
        let back = q.convert(PeriodUnit::SlmPx, &o).unwrap();
        assert!((back.value - 75.).abs() < 1e-9);
    }

    #[test]
    fn zero_wavevector_is_infinite_period() {
        let o = optics();
        let q0 = PeriodValue {
            value: 0.,
            unit: PeriodUnit::SampleWavevector,
        };
        assert_eq!(q0.period_px(&o).unwrap(), None);
        assert_eq!(q0.display_in(PeriodUnit::SlmPx, &o), "∞");
        assert!(q0.convert(PeriodUnit::SampleUm, &o).is_err());
        assert!(PeriodValue::px(0.).validate().is_err());
        assert!(
            PeriodValue {
                value: -0.1,
                unit: PeriodUnit::SampleWavevector
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn phase_and_angle_units() {
        let p = PhaseValue {
            value: 0.5,
            unit: PhaseUnit::Pi,
        };
        assert!((p.radians().unwrap() - PI / 2.).abs() < 1e-15);
        assert!((p.convert(PhaseUnit::Degree).unwrap().value - 90.).abs() < 1e-12);
        let a = AngleValue {
            value: PI / 2.,
            unit: AngleUnit::Radian,
        };
        assert!((a.as_degrees().unwrap() - 90.).abs() < 1e-12);
    }

    #[test]
    fn number_format() {
        assert_eq!(format_number(0.30000000000000004), "0.3");
        assert_eq!(format_number(1023.), "1023");
        assert_eq!(format_number(0.0666666666666), "0.06666666667");
    }
}
