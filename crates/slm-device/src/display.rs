//! Output-display identity and the checks that gate every send.
//!
//! The saved identity is the monitor device path (stable across Windows
//! renumbering) plus the GDI name it had when chosen. Any doubt stops output
//! and asks the user to reselect; nothing is retargeted automatically.

use crate::encoding::InputMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Connector {
    Hdmi,
    Dvi,
    DisplayPort,
    Internal,
    Other(i32),
    Unknown,
}

impl Connector {
    pub fn label(self) -> String {
        match self {
            Connector::Hdmi => "HDMI".into(),
            Connector::Dvi => "DVI".into(),
            Connector::DisplayPort => "DisplayPort".into(),
            Connector::Internal => "Internal".into(),
            Connector::Other(v) => format!("Output type {v}"),
            Connector::Unknown => "Unknown output".into(),
        }
    }
}

/// One active Windows display path, in physical pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayInfo {
    /// GDI source name, e.g. `\\.\DISPLAY3`.
    pub gdi_name: String,
    pub friendly_name: String,
    /// Monitor device path from `DISPLAYCONFIG_TARGET_DEVICE_NAME`.
    pub monitor_device_path: String,
    pub adapter: String,
    pub target_id: u32,
    pub connector: Connector,
    /// Desktop area of the source (position and size).
    pub desktop: Rect,
    /// `GetMonitorInfo` rectangle (physical with per-monitor DPI awareness).
    pub monitor_rect: Option<Rect>,
    /// Active size of the video signal; differs from `desktop` under GPU scaling.
    pub signal_active: Option<(u32, u32)>,
    pub refresh_hz: Option<f64>,
    pub bits_per_pixel: Option<u32>,
    pub primary: bool,
    /// Number of active targets sharing this source (>1 = duplicated/clone).
    pub clone_count: u32,
    pub rotated: bool,
    /// HDR / advanced color; `None` when Windows did not report it.
    pub advanced_color: Option<bool>,
    #[serde(default)]
    pub rgb_output: Option<bool>,
    #[serde(default)]
    pub bits_per_channel: Option<u32>,
    /// Shown in the GUI; demo displays are never physical outputs.
    #[serde(default)]
    pub simulated: bool,
}

impl DisplayInfo {
    /// Windows display number parsed from the GDI name.
    pub fn number(&self) -> Option<u32> {
        let digits: String = self
            .gdi_name
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        digits.parse().ok()
    }
    pub fn label(&self) -> String {
        let refresh = self
            .refresh_hz
            .map_or_else(String::new, |hz| format!(" @ {hz:.2} Hz"));
        format!(
            "{} · {} ({}) · {}×{}{refresh} · at ({}, {}){}{}",
            self.number().map_or_else(|| "?".into(), |n| n.to_string()),
            if self.friendly_name.is_empty() {
                "Unnamed display"
            } else {
                &self.friendly_name
            },
            self.connector.label(),
            self.desktop.width,
            self.desktop.height,
            self.desktop.x,
            self.desktop.y,
            if self.primary { " · Primary" } else { "" },
            if self.simulated { " · Simulated" } else { "" },
        )
    }
    pub fn selection(&self) -> DisplaySelection {
        DisplaySelection {
            monitor_device_path: self.monitor_device_path.clone(),
            gdi_name: self.gdi_name.clone(),
            friendly_name: self.friendly_name.clone(),
            width: self.desktop.width,
            height: self.desktop.height,
        }
    }
}

/// Saved PC-specific output identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySelection {
    pub monitor_device_path: String,
    pub gdi_name: String,
    pub friendly_name: String,
    pub width: u32,
    pub height: u32,
}

impl DisplaySelection {
    pub fn label(&self) -> String {
        format!(
            "{} ({}, {}×{})",
            if self.friendly_name.is_empty() {
                "Unnamed display"
            } else {
                &self.friendly_name
            },
            self.gdi_name,
            self.width,
            self.height
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputRequirements {
    pub width: u32,
    pub height: u32,
    /// Confirmed input mode; `None` blocks output.
    pub mode: Option<InputMode>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockReason {
    NoSelection,
    ModeNotConfirmed,
    UnsupportedMode(InputMode),
    RemoteSession,
    NotFound,
    Ambiguous(usize),
    NoIdentity,
    NumberChanged {
        was: String,
        now: String,
    },
    Duplicated(u32),
    Primary,
    Resolution {
        expected: (u32, u32),
        found: (u32, u32),
    },
    MonitorRectMismatch,
    GpuScaling {
        desktop: (u32, u32),
        signal: (u32, u32),
    },
    Rotated,
    Refresh {
        expected: f64,
        found: Option<f64>,
    },
    BitDepth(Option<u32>),
    AdvancedColor,
    NonRgbOutput,
    OutputBitDepth(u32),
    UnknownDisplayProperty(&'static str),
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockReason::NoSelection => write!(
                f,
                "No output display selected. Choose the SLM in Output Display."
            ),
            BlockReason::ModeNotConfirmed => write!(
                f,
                "SLM input mode is not confirmed. Check the mode on the device (60 Hz, Table 6), then confirm it in Device."
            ),
            BlockReason::UnsupportedMode(mode) => {
                write!(
                    f,
                    "{}",
                    mode.unsupported_reason()
                        .unwrap_or("Unsupported input mode.")
                )
            }
            BlockReason::RemoteSession => write!(
                f,
                "Remote Desktop session: the physical display is not visible. Reconnect from the physical console."
            ),
            BlockReason::NotFound => write!(
                f,
                "The selected display is no longer connected. Reselect the output display."
            ),
            BlockReason::Ambiguous(n) => {
                write!(
                    f,
                    "{n} displays match the saved identity. Reselect the output display."
                )
            }
            BlockReason::NoIdentity => {
                write!(
                    f,
                    "Windows reported no monitor device path, so the display cannot be identified safely."
                )
            }
            BlockReason::NumberChanged { was, now } => write!(
                f,
                "Display number changed ({was} → {now}). Reselect the output display to confirm it."
            ),
            BlockReason::Duplicated(n) => write!(
                f,
                "The display is duplicated on {n} outputs. Use Extend in Windows display settings."
            ),
            BlockReason::Primary => write!(
                f,
                "The selected display is the primary display. Keep the controls on another screen and make the SLM a secondary display."
            ),
            BlockReason::Resolution { expected, found } => write!(
                f,
                "Display mode is {}×{}; the SLM needs {}×{}.",
                found.0, found.1, expected.0, expected.1
            ),
            BlockReason::MonitorRectMismatch => write!(
                f,
                "Monitor area differs from the display mode (DPI virtualization or scaling). Output would not be 1:1."
            ),
            BlockReason::GpuScaling { desktop, signal } => write!(
                f,
                "GPU scaling: desktop {}×{} is sent as {}×{}. Set the native mode without scaling.",
                desktop.0, desktop.1, signal.0, signal.1
            ),
            BlockReason::Rotated => {
                write!(f, "The display is rotated. Set orientation to Landscape.")
            }
            BlockReason::Refresh { expected, found } => match found {
                Some(hz) => write!(
                    f,
                    "Refresh rate is {hz:.2} Hz; the confirmed mode needs {expected:.0} Hz."
                ),
                None => write!(
                    f,
                    "Windows did not report the refresh rate; {expected:.0} Hz is required."
                ),
            },
            BlockReason::BitDepth(bits) => match bits {
                Some(b) => write!(
                    f,
                    "Desktop color depth is {b} bit; 32-bit color is required for exact codes."
                ),
                None => write!(f, "Windows did not report the desktop color depth."),
            },
            BlockReason::NonRgbOutput => write!(
                f,
                "The display output is not RGB. Select 8-bit RGB in the graphics settings."
            ),
            BlockReason::OutputBitDepth(bits) => write!(
                f,
                "The display output is {bits} bits per channel; select 8-bit RGB in the graphics settings."
            ),
            BlockReason::UnknownDisplayProperty(property) => {
                write!(
                    f,
                    "Windows did not report {property}; exact SLM output cannot be verified."
                )
            }
            BlockReason::AdvancedColor => write!(
                f,
                "HDR / advanced color is on for this display; pixel values would be remapped. Turn HDR off."
            ),
        }
    }
}

pub fn describe(reasons: &[BlockReason]) -> String {
    reasons
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Find the saved display and check that output would be exact and 1:1.
pub fn check_output(
    selection: Option<&DisplaySelection>,
    displays: &[DisplayInfo],
    requirements: &OutputRequirements,
    remote_session: bool,
) -> Result<DisplayInfo, Vec<BlockReason>> {
    let mut reasons = Vec::new();
    if remote_session {
        reasons.push(BlockReason::RemoteSession);
    }
    match requirements.mode {
        None => reasons.push(BlockReason::ModeNotConfirmed),
        Some(mode) if !mode.supported() => reasons.push(BlockReason::UnsupportedMode(mode)),
        Some(_) => {}
    }
    let Some(selection) = selection else {
        reasons.push(BlockReason::NoSelection);
        return Err(reasons);
    };
    if selection.monitor_device_path.is_empty() {
        reasons.push(BlockReason::NoIdentity);
        return Err(reasons);
    }
    let matches: Vec<&DisplayInfo> = displays
        .iter()
        .filter(|d| d.monitor_device_path == selection.monitor_device_path)
        .collect();
    let display = match matches.as_slice() {
        [] => {
            reasons.push(BlockReason::NotFound);
            return Err(reasons);
        }
        [one] => *one,
        many => {
            reasons.push(BlockReason::Ambiguous(many.len()));
            return Err(reasons);
        }
    };
    if display.gdi_name != selection.gdi_name {
        reasons.push(BlockReason::NumberChanged {
            was: selection.gdi_name.clone(),
            now: display.gdi_name.clone(),
        });
    }
    if display.clone_count > 1 {
        reasons.push(BlockReason::Duplicated(display.clone_count));
    }
    if display.primary {
        reasons.push(BlockReason::Primary);
    }
    let desktop = (display.desktop.width, display.desktop.height);
    let expected = (requirements.width, requirements.height);
    if desktop != expected {
        reasons.push(BlockReason::Resolution {
            expected,
            found: desktop,
        });
    }
    if let Some(rect) = display.monitor_rect
        && rect != display.desktop
    {
        reasons.push(BlockReason::MonitorRectMismatch);
    }
    if let Some(signal) = display.signal_active
        && signal != desktop
    {
        reasons.push(BlockReason::GpuScaling { desktop, signal });
    }
    if display.rotated {
        reasons.push(BlockReason::Rotated);
    }
    if let Some(mode) = requirements.mode {
        let expected = mode.video_refresh_hz();
        match display.refresh_hz {
            Some(hz) if (hz - expected).abs() < 0.5 => {}
            found => reasons.push(BlockReason::Refresh { expected, found }),
        }
    }
    if display.bits_per_pixel != Some(32) {
        reasons.push(BlockReason::BitDepth(display.bits_per_pixel));
    }
    if display.advanced_color == Some(true) {
        reasons.push(BlockReason::AdvancedColor);
    }
    if display.rgb_output == Some(false) {
        reasons.push(BlockReason::NonRgbOutput);
    }
    if let Some(bits) = display.bits_per_channel
        && bits != 8
    {
        reasons.push(BlockReason::OutputBitDepth(bits));
    }
    if !display.simulated {
        for (known, property) in [
            (display.monitor_rect.is_some(), "the monitor bounds"),
            (display.rgb_output.is_some(), "the output color format"),
            (display.bits_per_channel.is_some(), "the output color depth"),
            (
                display.signal_active.is_some(),
                "the active video resolution",
            ),
            (
                display.advanced_color.is_some(),
                "the HDR / advanced color state",
            ),
        ] {
            if !known {
                reasons.push(BlockReason::UnknownDisplayProperty(property));
            }
        }
    }
    if reasons.is_empty() {
        Ok(display.clone())
    } else {
        Err(reasons)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn slm(gdi: &str, x: i32) -> DisplayInfo {
        DisplayInfo {
            gdi_name: gdi.into(),
            friendly_name: "SLM-200".into(),
            monitor_device_path: r"\\?\DISPLAY#SAN0001#5&1&UID4352#{e6f07b5f}".into(),
            adapter: "0:1234".into(),
            target_id: 4352,
            connector: Connector::Hdmi,
            desktop: Rect {
                x,
                y: 0,
                width: 1920,
                height: 1200,
            },
            monitor_rect: Some(Rect {
                x,
                y: 0,
                width: 1920,
                height: 1200,
            }),
            signal_active: Some((1920, 1200)),
            refresh_hz: Some(60.),
            bits_per_pixel: Some(32),
            primary: false,
            clone_count: 1,
            rotated: false,
            advanced_color: Some(false),
            rgb_output: Some(true),
            bits_per_channel: Some(8),
            simulated: false,
        }
    }

    fn req() -> OutputRequirements {
        OutputRequirements {
            width: 1920,
            height: 1200,
            mode: Some(InputMode::Hz60),
        }
    }

    #[test]
    fn real_output_requires_reported_properties_and_eight_bit_rgb() {
        let mut display = slm(r"\.\DISPLAY2", 1920);
        let selection = display.selection();
        display.signal_active = None;
        display.rgb_output = Some(false);
        display.bits_per_channel = Some(10);
        let reasons = check_output(Some(&selection), &[display], &req(), false).unwrap_err();
        assert!(reasons.contains(&BlockReason::UnknownDisplayProperty(
            "the active video resolution"
        )));
        assert!(reasons.contains(&BlockReason::NonRgbOutput));
        assert!(reasons.contains(&BlockReason::OutputBitDepth(10)));
    }

    #[test]
    fn accepts_exact_secondary_display() {
        let d = slm(r"\\.\DISPLAY2", 2560);
        assert_eq!(d.number(), Some(2));
        let sel = d.selection();
        assert_eq!(
            check_output(Some(&sel), std::slice::from_ref(&d), &req(), false).unwrap(),
            d
        );
    }

    #[test]
    fn blocks_every_unsafe_condition() {
        let d = slm(r"\\.\DISPLAY2", 2560);
        let sel = d.selection();
        let check = |displays: &[DisplayInfo]| {
            check_output(Some(&sel), displays, &req(), false).unwrap_err()
        };
        assert_eq!(check(&[]), vec![BlockReason::NotFound]);
        assert_eq!(
            check(&[d.clone(), d.clone()]),
            vec![BlockReason::Ambiguous(2)]
        );
        let renumbered = slm(r"\\.\DISPLAY3", 2560);
        assert!(matches!(
            check(&[renumbered])[0],
            BlockReason::NumberChanged { .. }
        ));
        let mut dup = d.clone();
        dup.clone_count = 2;
        assert_eq!(check(&[dup]), vec![BlockReason::Duplicated(2)]);
        let mut primary = d.clone();
        primary.primary = true;
        assert_eq!(check(&[primary]), vec![BlockReason::Primary]);
        let mut mode = d.clone();
        mode.desktop.width = 1920;
        mode.desktop.height = 1080;
        mode.monitor_rect = Some(mode.desktop);
        mode.signal_active = Some((1920, 1080));
        assert!(matches!(check(&[mode])[0], BlockReason::Resolution { .. }));
        let mut scaled = d.clone();
        scaled.signal_active = Some((3840, 2160));
        assert!(matches!(
            check(&[scaled])[0],
            BlockReason::GpuScaling { .. }
        ));
        let mut dpi = d.clone();
        dpi.monitor_rect = Some(Rect {
            x: 2560,
            y: 0,
            width: 1280,
            height: 800,
        });
        assert_eq!(check(&[dpi]), vec![BlockReason::MonitorRectMismatch]);
        let mut hz = d.clone();
        hz.refresh_hz = Some(59.0);
        assert!(matches!(check(&[hz])[0], BlockReason::Refresh { .. }));
        let mut hdr = d.clone();
        hdr.advanced_color = Some(true);
        assert_eq!(check(&[hdr]), vec![BlockReason::AdvancedColor]);
        let mut depth = d.clone();
        depth.bits_per_pixel = Some(16);
        assert_eq!(check(&[depth]), vec![BlockReason::BitDepth(Some(16))]);
        let mut rotated = d.clone();
        rotated.rotated = true;
        assert_eq!(check(&[rotated]), vec![BlockReason::Rotated]);
        let r = check_output(Some(&sel), std::slice::from_ref(&d), &req(), true).unwrap_err();
        assert_eq!(r, vec![BlockReason::RemoteSession]);
        let unconfirmed = OutputRequirements {
            mode: None,
            ..req()
        };
        assert_eq!(
            check_output(Some(&sel), std::slice::from_ref(&d), &unconfirmed, false).unwrap_err(),
            vec![BlockReason::ModeNotConfirmed]
        );
        let hz120 = OutputRequirements {
            mode: Some(InputMode::Hz120),
            ..req()
        };
        assert_eq!(
            check_output(Some(&sel), std::slice::from_ref(&d), &hz120, false).unwrap_err(),
            vec![BlockReason::UnsupportedMode(InputMode::Hz120)]
        );
        assert_eq!(
            check_output(None, &[d], &req(), false).unwrap_err(),
            vec![BlockReason::NoSelection]
        );
    }
}
