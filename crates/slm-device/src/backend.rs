//! Output backend interface. A backend only draws what the worker gives it;
//! identity checks and state live in the worker.

use crate::display::DisplayInfo;
use crate::encoding::InputMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    /// In-memory simulated output; never touches a display.
    #[serde(rename = "demo")]
    Demo,
    /// Windows extended-desktop output through HDMI/DVI.
    #[serde(rename = "windows_display")]
    WindowsDisplay,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::Demo => "Demo (simulated)",
            BackendKind::WindowsDisplay => "Windows display (HDMI/DVI)",
        }
    }
    pub fn available() -> &'static [BackendKind] {
        if cfg!(windows) {
            &[BackendKind::WindowsDisplay, BackendKind::Demo]
        } else {
            &[BackendKind::Demo]
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputTarget {
    pub display: DisplayInfo,
    pub mode: InputMode,
}

/// How far a successful show was checked. None of these prove the optical
/// response of the SLM; that is a hardware test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verification {
    /// Demo backend: the encoded buffer decoded back to the same codes.
    Simulated,
    /// Windows: the composed desktop area read back equal to the encoded frame.
    DesktopReadback,
}

impl Verification {
    pub fn label(self) -> &'static str {
        match self {
            Verification::Simulated => "simulated output, encoding round-trip checked",
            Verification::DesktopReadback => {
                "drawn 1:1; desktop readback matched (optical response not verified)"
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowReceipt {
    /// `CodeFrame::digest` of what was submitted.
    pub digest: String,
    pub uniform_code: Option<u16>,
    pub verification: Verification,
    pub display: String,
    pub mode: InputMode,
    pub at_unix_ms: u64,
}

pub trait OutputBackend: Send {
    fn kind(&self) -> BackendKind;
    fn list_displays(&mut self) -> Result<Vec<DisplayInfo>, String>;
    fn remote_session(&self) -> bool;
    /// Prepare (but do not show) the output on `target`.
    fn open(&mut self, target: &OutputTarget) -> Result<(), String>;
    /// Show an encoded top-down BGRX frame sized to the target.
    fn show(&mut self, bgrx: &[u8]) -> Result<Verification, String>;
    /// Remove the output surface. The SLM then shows whatever Windows draws.
    fn close(&mut self) -> Result<(), String>;
    /// A display change or move detected since the last call; output has
    /// already been hidden by the backend when this returns `Some`.
    fn take_invalidation(&mut self) -> Option<String>;
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
