//! Device and optical profiles. Values are experiment-system settings, not
//! manufacturer facts: the Santec candidate comes from the notebook runs and
//! must be checked against the vendor software and the current optics.

use serde::{Deserialize, Serialize};
use slm_core::{GenerationProfile, Optics, PhaseCalibration, Raster};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlmModel {
    #[serde(rename = "santec_slm_200")]
    SantecSlm200,
}

impl SlmModel {
    pub fn label(self) -> &'static str {
        match self {
            SlmModel::SantecSlm200 => "Santec SLM-200",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub model: SlmModel,
    pub generation: GenerationProfile,
    /// Default notebook `(delta_Hpx, delta_Vpx)` for new patterns [px].
    pub default_center_offset_px: [i32; 2],
    /// Uniform raw code shown on connect, before disconnect and on normal exit.
    /// Provisional `0`; to be checked against the vendor software.
    pub reset_code: u16,
    /// Wavelength set on the device, as recorded by the user [nm].
    #[serde(default)]
    pub device_wavelength_nm: Option<f64>,
    /// SDK raw maximum-phase readback; units must be verified against the SDK.
    #[serde(default)]
    pub device_phase_raw: Option<u32>,
    /// Free-text record of the generator calibration and where it came from.
    #[serde(default)]
    pub calibration_note: String,
}

impl DeviceProfile {
    pub fn santec_slm200() -> Self {
        let mut profile = Self::santec_slm200_candidate();
        profile.generation.calibration.mapping = slm_core::PhaseMapping::SantecFullScale;
        profile.calibration_note = String::new();
        profile
    }

    /// Notebook candidate: 1920 × 1200, 8 µm, 1/75, center (−16, 110),
    /// phase0 = 136, period = 888.
    pub fn santec_slm200_candidate() -> Self {
        Self {
            model: SlmModel::SantecSlm200,
            generation: GenerationProfile {
                raster: Raster {
                    width: 1920,
                    height: 1200,
                },
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
            },
            default_center_offset_px: [-16, 110],
            reset_code: 0,
            device_wavelength_nm: None,
            device_phase_raw: None,
            calibration_note: "Notebook candidate (phase0 136, period 888); not yet compared with the vendor software"
                .into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.generation.validate()?;
        if self.generation.max_code != 1023 {
            return Err("Santec SLM-200 requires the 10-bit code range 0–1023".into());
        }
        if self.reset_code > self.generation.max_code {
            return Err(format!(
                "Reset code {} is outside 0–{}",
                self.reset_code, self.generation.max_code
            ));
        }
        if let Some(nm) = self.device_wavelength_nm
            && !(nm.is_finite() && nm > 0.)
        {
            return Err("Device wavelength must be a positive number of nm".into());
        }
        Ok(())
    }
}
