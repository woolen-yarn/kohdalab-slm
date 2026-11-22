//! Simulated output used by `--demo`, tests and macOS. It exercises the same
//! worker path (identity checks, encoding, state) without any display.

use crate::backend::{BackendKind, OutputBackend, OutputTarget, Verification};
use crate::display::{Connector, DisplayInfo, Rect};
use crate::encoding::{InputMode, decode_bgrx};
use slm_core::CodeFrame;
use std::sync::{Arc, Mutex};

/// Shared knobs and observations for the demo backend.
#[derive(Debug)]
pub struct DemoState {
    pub displays: Vec<DisplayInfo>,
    pub remote_session: bool,
    pub fail_next_show: Option<String>,
    pub pending_invalidation: Option<String>,
    pub open: bool,
    pub last_frame: Option<CodeFrame>,
    pub shows: usize,
    pub mode: Option<InputMode>,
}

#[derive(Clone, Debug)]
pub struct DemoControl(pub Arc<Mutex<DemoState>>);

impl Default for DemoControl {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(DemoState {
            displays: demo_displays(),
            remote_session: false,
            fail_next_show: None,
            pending_invalidation: None,
            open: false,
            last_frame: None,
            shows: 0,
            mode: None,
        })))
    }
}

impl DemoControl {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, DemoState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Simulate Windows renumbering, unplugging or duplicating the SLM.
    pub fn simulate_display_change(&self, displays: Vec<DisplayInfo>, reason: &str) {
        let mut state = self.lock();
        state.displays = displays;
        if state.open {
            state.pending_invalidation = Some(reason.into());
        }
    }
}

pub fn demo_displays() -> Vec<DisplayInfo> {
    let base =
        |gdi: &str, name: &str, path: &str, x: i32, w: u32, h: u32, primary: bool, connector| {
            DisplayInfo {
                gdi_name: gdi.into(),
                friendly_name: name.into(),
                monitor_device_path: path.into(),
                adapter: "demo".into(),
                target_id: 0,
                connector,
                desktop: Rect {
                    x,
                    y: 0,
                    width: w,
                    height: h,
                },
                monitor_rect: Some(Rect {
                    x,
                    y: 0,
                    width: w,
                    height: h,
                }),
                signal_active: Some((w, h)),
                refresh_hz: Some(60.),
                bits_per_pixel: Some(32),
                primary,
                clone_count: 1,
                rotated: false,
                advanced_color: Some(false),
                rgb_output: Some(true),
                bits_per_channel: Some(8),
                simulated: true,
            }
        };
    vec![
        base(
            r"\\.\DISPLAY1",
            "Control monitor (demo)",
            r"demo:\\control",
            0,
            2560,
            1440,
            true,
            Connector::DisplayPort,
        ),
        base(
            r"\\.\DISPLAY2",
            "SLM-200 (demo)",
            r"demo:\\slm-200",
            2560,
            1920,
            1200,
            false,
            Connector::Hdmi,
        ),
    ]
}

pub struct DemoBackend {
    control: DemoControl,
    target: Option<OutputTarget>,
}

impl DemoBackend {
    pub fn new(control: DemoControl) -> Self {
        Self {
            control,
            target: None,
        }
    }
}

impl OutputBackend for DemoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Demo
    }
    fn list_displays(&mut self) -> Result<Vec<DisplayInfo>, String> {
        Ok(self.control.lock().displays.clone())
    }
    fn remote_session(&self) -> bool {
        self.control.lock().remote_session
    }
    fn open(&mut self, target: &OutputTarget) -> Result<(), String> {
        let mut state = self.control.lock();
        state.open = true;
        state.mode = Some(target.mode);
        state.pending_invalidation = None;
        self.target = Some(target.clone());
        Ok(())
    }
    fn show(&mut self, bgrx: &[u8]) -> Result<Verification, String> {
        let target = self.target.as_ref().ok_or("Demo output is not open")?;
        let mut state = self.control.lock();
        if let Some(reason) = state.fail_next_show.take() {
            state.last_frame = None;
            return Err(reason);
        }
        let frame = decode_bgrx(
            bgrx,
            target.display.desktop.width,
            target.display.desktop.height,
            target.mode,
        )?;
        state.last_frame = Some(frame);
        state.shows += 1;
        Ok(Verification::Simulated)
    }
    fn close(&mut self) -> Result<(), String> {
        let mut state = self.control.lock();
        state.open = false;
        self.target = None;
        Ok(())
    }
    fn take_invalidation(&mut self) -> Option<String> {
        self.control.lock().pending_invalidation.take()
    }
}
