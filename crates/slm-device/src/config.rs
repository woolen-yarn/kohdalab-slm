//! `~/.kohdalab/slm/config/config.json`: schema version, device/optical
//! profile, backend, PC-specific output display and the input-mode
//! confirmation. Opening an experiment never changes the output display.

use crate::backend::BackendKind;
use crate::display::DisplaySelection;
use crate::encoding::InputMode;
use crate::profile::DeviceProfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModeConfirmation {
    pub mode: InputMode,
    pub confirmed_unix_s: u64,
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
    pub schema_version: u32,
    pub profile: DeviceProfile,
    pub output: OutputConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            profile: DeviceProfile::santec_slm200_candidate(),
            output: OutputConfig {
                backend: BackendKind::available()[0],
                display: None,
                confirmed_mode: None,
            },
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
                        "{}: {e}; using defaults (the file is not overwritten until you save)",
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
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or("missing schema_version")?;
        if version != u64::from(CONFIG_SCHEMA_VERSION) {
            return Err(format!("config schema {version} is not supported"));
        }
        let config: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        config.profile.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.profile.validate()?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        slm_core::experiment::write_atomic(path, json.as_bytes())
    }

    pub fn confirmed_mode(&self) -> Option<InputMode> {
        self.output.confirmed_mode.as_ref().map(|c| c.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
