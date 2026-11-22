//! `~/.kohdalab/slm/config/config.json`: schema version, device/optical
//! profile, backend, PC-specific output display and the input-mode
//! confirmation. Opening an experiment never changes the output display.

use crate::backend::BackendKind;
use crate::display::DisplaySelection;
use crate::encoding::InputMode;
use crate::profile::DeviceProfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModeConfirmation {
    pub mode: InputMode,
    pub confirmed_unix_s: u64,
    /// Device path from the active display at the time of confirmation.
    #[serde(default)]
    pub monitor_device_path: String,
    /// What was checked (device setting, Table 6/7, test patterns).
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputConfig {
    pub backend: BackendKind,
    pub display: Option<DisplaySelection>,
    /// Output is blocked until the user records the verified input mode.
    pub confirmed_mode: Option<ModeConfirmation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "application_id")]
    pub application: String,
    pub schema_version: u32,
    pub profile: DeviceProfile,
    pub output: OutputConfig,
    /// Preview and heatmap PNG colors only; never affects codes or output.
    /// Configs written before this field, or naming a map this build does not
    /// know, load as grayscale instead of rejecting the whole config.
    #[serde(default, deserialize_with = "lenient_colormap")]
    pub preview_colormap: slm_core::heatmap::Colormap,
}

fn application_id() -> String {
    "kohdalab-slm".into()
}

fn lenient_colormap<'de, D>(deserializer: D) -> Result<slm_core::heatmap::Colormap, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            application: application_id(),
            schema_version: CONFIG_SCHEMA_VERSION,
            profile: DeviceProfile::santec_slm200(),
            output: OutputConfig {
                backend: BackendKind::available()[0],
                display: None,
                confirmed_mode: None,
            },
            preview_colormap: slm_core::heatmap::Colormap::Grayscale,
        }
    }
}

pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn default_config_path() -> Option<PathBuf> {
    Some(
        home_dir()?
            .join(".kohdalab")
            .join("slm")
            .join("config")
            .join("config.json"),
    )
}

impl AppConfig {
    /// Load, or return defaults with a message when the file is absent or
    /// unreadable. An unreadable file is left untouched.
    pub fn load_or_default(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!("Read {}: {e}; using defaults", path.display())),
            ),
            Ok(text) => match Self::parse_for_this_os(&text) {
                Ok((config, note)) => (config, note),
                Err(e) => (
                    Self::default(),
                    Some(format!(
                        "{}: {e}; using defaults (the original file is protected; choose a new config path to save)",
                        path.display()
                    )),
                ),
            },
        }
    }

    /// Read a config for the GUI's Load button: same rules as startup,
    /// including backend resolution for this OS.
    pub fn load(path: &Path) -> Result<(Self, Option<String>), String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("Read {}: {e}", path.display()))?;
        Self::parse_for_this_os(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parse, then replace a backend this OS cannot run (e.g. a Windows
    /// config opened on macOS) with the first available one. The display
    /// selection is kept; it simply will not match until reselected.
    pub fn parse_for_this_os(text: &str) -> Result<(Self, Option<String>), String> {
        let mut config = Self::parse(text)?;
        let available = BackendKind::available();
        if available.contains(&config.output.backend) {
            return Ok((config, None));
        }
        let note = format!(
            "Backend {} is not available on this OS; using {}.",
            config.output.backend.label(),
            available[0].label()
        );
        config.output.backend = available[0];
        Ok((config, Some(note)))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
        if let Some(app) = value.get("application")
            && app.as_str() != Some("kohdalab-slm")
        {
            return Err("This configuration belongs to another application".into());
        }
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or("missing schema_version")?;
        if version != 1 && version != u64::from(CONFIG_SCHEMA_VERSION) {
            return Err(format!("config schema {version} is not supported"));
        }
        if version >= 2
            && value
                .pointer("/profile/generation/calibration/mapping")
                .is_none()
        {
            return Err("config schema 2 requires profile.generation.calibration.mapping".into());
        }
        let mut config: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        config.schema_version = CONFIG_SCHEMA_VERSION;
        config.profile.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.profile.validate()?;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                Self::parse(&text).map_err(|e| {
                    format!(
                        "Refusing to overwrite incompatible config {}: {e}. Choose a new path.",
                        path.display()
                    )
                })?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "Cannot check existing config {}: {e}",
                    path.display()
                ));
            }
        }
        let mut current = self.clone();
        current.application = application_id();
        current.schema_version = CONFIG_SCHEMA_VERSION;
        let json = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
        slm_core::experiment::write_atomic(path, json.as_bytes())
    }

    pub fn input_mode_confirmation(&self) -> Option<&ModeConfirmation> {
        let display = self.output.display.as_ref()?;
        self.output.confirmed_mode.as_ref().filter(|confirmation| {
            !confirmation.monitor_device_path.is_empty()
                && confirmation.monitor_device_path == display.monitor_device_path
        })
    }

    pub fn confirmed_mode(&self) -> Option<InputMode> {
        self.input_mode_confirmation().map(|c| c.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_preserves_foreign_corrupt_and_future_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let config = AppConfig::default();
        let mut foreign = serde_json::to_value(&config).unwrap();
        foreign["application"] = "kohdalab-iv".into();
        let mut future = serde_json::to_value(&config).unwrap();
        future["schema_version"] = 99.into();
        for original in [
            "{".to_string(),
            foreign.to_string(),
            future.to_string(),
            "{\"items\":[]}".into(),
        ] {
            std::fs::write(&path, &original).unwrap();
            assert!(config.save(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
        // Legacy configs without an application stamp are accepted and upgraded.
        let mut legacy = serde_json::to_value(&config).unwrap();
        legacy.as_object_mut().unwrap().remove("application");
        std::fs::write(&path, legacy.to_string()).unwrap();
        config.save(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("kohdalab-slm")
        );
    }

    #[test]
    fn round_trip_and_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join("config.json");
        let (config, warning) = AppConfig::load_or_default(&path);
        assert!(warning.is_none());
        assert!(config.output.confirmed_mode.is_none());
        assert_eq!(config.profile.reset_code, 0);
        let mut edited = config.clone();
        edited.profile.generation.calibration.phase0 = 140.;
        edited.save(&path).unwrap();
        assert_eq!(AppConfig::load_or_default(&path).0, edited);
        std::fs::write(&path, "{").unwrap();
        let (fallback, warning) = AppConfig::load_or_default(&path);
        assert!(warning.is_some());
        assert_eq!(fallback, AppConfig::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{");
    }

    #[test]
    fn mode_confirmation_is_tied_to_the_display_that_was_checked() {
        let mut config = AppConfig::default();
        let displays = crate::demo::demo_displays();
        config.output.display = Some(displays[1].selection());
        config.output.confirmed_mode = Some(ModeConfirmation {
            mode: InputMode::Hz60,
            confirmed_unix_s: 0,
            monitor_device_path: String::new(),
            note: "Legacy confirmation".into(),
        });
        assert_eq!(config.confirmed_mode(), None);
        config
            .output
            .confirmed_mode
            .as_mut()
            .unwrap()
            .monitor_device_path = displays[1].monitor_device_path.clone();
        assert_eq!(config.confirmed_mode(), Some(InputMode::Hz60));
        config.output.display = Some(displays[0].selection());
        assert_eq!(config.confirmed_mode(), None);
    }

    #[test]
    fn legacy_profiles_keep_notebook_mapping() {
        let current = AppConfig::default();
        assert_eq!(
            current.profile.generation.calibration.mapping,
            slm_core::PhaseMapping::SantecFullScale
        );
        let mut legacy = serde_json::to_value(current).unwrap();
        legacy["profile"]["generation"]["calibration"]
            .as_object_mut()
            .unwrap()
            .remove("mapping");
        assert!(
            AppConfig::parse(&legacy.to_string())
                .unwrap_err()
                .contains("calibration.mapping")
        );
        legacy["schema_version"] = 1.into();
        let loaded = AppConfig::parse(&legacy.to_string()).unwrap();
        assert_eq!(loaded.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(
            loaded.profile.generation.calibration.mapping,
            slm_core::PhaseMapping::Notebook
        );
    }

    #[test]
    fn unavailable_backend_is_resolved_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut config = AppConfig::default();
        // A backend this OS cannot run.
        let foreign = if cfg!(windows) {
            BackendKind::Demo
        } else {
            BackendKind::WindowsDisplay
        };
        config.output.backend = foreign;
        std::fs::write(&path, serde_json::to_string(&config).unwrap()).unwrap();
        let (loaded, note) = AppConfig::load(&path).unwrap();
        if BackendKind::available().contains(&foreign) {
            assert!(note.is_none());
        } else {
            assert_eq!(loaded.output.backend, BackendKind::available()[0]);
            assert!(note.unwrap().contains("not available"));
            assert_eq!(
                AppConfig::load_or_default(&path).0.output.backend,
                BackendKind::available()[0]
            );
        }
    }

    #[test]
    fn preview_colormap_round_trips_and_defaults_to_grayscale() {
        use slm_core::heatmap::Colormap;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut config = AppConfig::default();
        assert_eq!(config.preview_colormap, Colormap::Grayscale);
        config.preview_colormap = Colormap::Viridis;
        config.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"preview_colormap\": \"viridis\""), "{text}");
        assert_eq!(
            AppConfig::load(&path).unwrap().0.preview_colormap,
            Colormap::Viridis
        );
        // An existing config without the field loads as grayscale.
        let mut old = serde_json::to_value(AppConfig::default()).unwrap();
        old.as_object_mut().unwrap().remove("preview_colormap");
        assert_eq!(
            AppConfig::parse(&old.to_string()).unwrap().preview_colormap,
            Colormap::Grayscale
        );
        old["preview_colormap"] = "sepia".into();
        assert_eq!(
            AppConfig::parse(&old.to_string()).unwrap().preview_colormap,
            Colormap::Grayscale
        );
    }
}
