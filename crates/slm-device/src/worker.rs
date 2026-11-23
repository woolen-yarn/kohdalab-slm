//! Single device worker. All output goes through this thread in order.
//!
//! State contract:
//! - `Connected` means the backend confirmed the last frame (`showing`).
//! - A failed draw, a lost display or a display change makes the state
//!   `Unknown`: the output was hidden or may show something else.
//! - Rendering errors happen before anything is sent and leave the state as it
//!   was.
//! - Every send first re-enumerates displays and re-checks the saved identity;
//!   on any doubt the output is closed and the user must reselect.

use crate::backend::{BackendKind, OutputBackend, OutputTarget, ShowReceipt, now_unix_ms};
use crate::demo::{DemoBackend, DemoControl};
use crate::display::{DisplayInfo, DisplaySelection, OutputRequirements, check_output, describe};
use crate::encoding::{InputMode, encode_bgrx};
use crate::profile::DeviceProfile;
use slm_core::experiment::{ExperimentItem, render_item};
use slm_core::{CodeFrame, GenerationProfile, PatternSpec};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum FrameSource {
    Codes(Arc<CodeFrame>),
    Generate {
        spec: PatternSpec,
        profile: GenerationProfile,
    },
    Experiment {
        item: ExperimentItem,
        asset: Option<PathBuf>,
    },
}

#[derive(Clone, Debug)]
pub struct ApplyItem {
    /// Experiment item ID, when applying from the experiment list.
    pub item_id: Option<String>,
    pub label: String,
    pub source: FrameSource,
}

pub enum Command {
    ListDisplays {
        backend: BackendKind,
    },
    /// Acknowledged by exactly one [`Event::ConnectDone`] with `request`.
    Connect {
        request: u64,
        backend: BackendKind,
        selection: Option<DisplaySelection>,
        mode: Option<InputMode>,
        profile: DeviceProfile,
    },
    Apply {
        request: u64,
        item: ApplyItem,
    },
    Reset {
        request: u64,
    },
    Wavelength {
        backend: BackendKind,
        usb_device: u32,
        requested_nm: Option<u32>,
    },
    /// Acknowledged by exactly one [`Event::DisconnectDone`] with `request`.
    Disconnect {
        request: u64,
    },
    /// Reset, close and stop the thread; replies with a summary.
    Shutdown {
        reply: Sender<Result<String, String>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Showing {
    pub item_id: Option<String>,
    pub label: String,
    pub receipt: ShowReceipt,
}

/// `Connected` carries the display and receipt; the enum is sent rarely, so
/// its size is not a concern.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum DeviceState {
    Disconnected,
    Connecting,
    Connected {
        backend: BackendKind,
        display: DisplayInfo,
        mode: InputMode,
        showing: Showing,
    },
    /// Output state is not known; reconnect after reselecting the display.
    Unknown {
        reason: String,
    },
}

impl DeviceState {
    pub fn is_connected(&self) -> bool {
        matches!(self, DeviceState::Connected { .. })
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Displays {
        backend: BackendKind,
        result: Result<Vec<DisplayInfo>, String>,
        remote_session: bool,
    },
    State(DeviceState),
    Applied {
        request: u64,
        item_id: Option<String>,
        label: String,
        receipt: ShowReceipt,
    },
    ApplyFailed {
        request: u64,
        item_id: Option<String>,
        label: String,
        error: String,
        /// `false` when the failure left the SLM output unknown.
        output_known: bool,
    },
    Wavelength(Result<u32, String>),
    Log(String),
    /// Worker acknowledgement of one `Connect`. `Ok` carries the receipt of
    /// the backend-confirmed reset frame. `Err` says whether the output is
    /// still known (`true`: this attempt left nothing on screen).
    ConnectDone {
        request: u64,
        result: Result<ShowReceipt, ConnectFailure>,
    },
    /// Worker acknowledgement of one `Disconnect` (also emitted on shutdown
    /// with request 0) after the output was closed or found already closed.
    DisconnectDone {
        request: u64,
        report: DisconnectReport,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectFailure {
    pub error: String,
    pub output_known: bool,
}

/// What the reset-before-close step of a disconnect actually did.
#[derive(Clone, Debug, PartialEq)]
pub enum ResetOutcome {
    /// The backend confirmed the uniform reset frame before closing.
    Confirmed(ShowReceipt),
    /// The reset draw failed; the output was hidden and closed unverified.
    Failed(String),
    /// A session was open but its output was not trusted; no reset was sent.
    SkippedUnknown,
    /// No output was open. `was_unknown`: the previous output was unknown.
    NotOpen { was_unknown: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisconnectReport {
    pub reset: ResetOutcome,
    /// Uniform raw device code of the reset frame (profile `reset_code`).
    pub reset_code: Option<u16>,
    pub close_error: Option<String>,
    pub summary: String,
}

impl DisconnectReport {
    /// The backend confirmed the reset frame and the output closed without
    /// error. A backend receipt, not an optical measurement.
    pub fn reset_confirmed(&self) -> bool {
        matches!(self.reset, ResetOutcome::Confirmed(_)) && self.close_error.is_none()
    }
}

pub type BackendFactory = Box<dyn Fn(BackendKind) -> Result<Box<dyn OutputBackend>, String> + Send>;

pub fn default_factory(demo: DemoControl) -> BackendFactory {
    Box::new(move |kind| match kind {
        BackendKind::Demo => Ok(Box::new(DemoBackend::new(demo.clone())) as Box<dyn OutputBackend>),
        #[cfg(windows)]
        BackendKind::WindowsDisplay => {
            Ok(Box::new(crate::windows::WindowsBackend::new()) as Box<dyn OutputBackend>)
        }
        #[cfg(not(windows))]
        BackendKind::WindowsDisplay => {
            Err("Windows display output is only available on Windows".into())
        }
    })
}

struct Session {
    backend: Box<dyn OutputBackend>,
    selection: DisplaySelection,
    mode: InputMode,
    profile: DeviceProfile,
    display: DisplayInfo,
}

struct Worker {
    factory: BackendFactory,
    events: Sender<Event>,
    notify: Box<dyn Fn() + Send>,
    session: Option<Session>,
    state: DeviceState,
    demo_wavelength_nm: u32,
}

impl Worker {
    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
        (self.notify)();
    }
    fn log(&self, message: impl Into<String>) {
        self.emit(Event::Log(message.into()));
    }
    fn set_state(&mut self, state: DeviceState) {
        if self.state != state {
            self.state = state.clone();
            self.emit(Event::State(state));
        }
    }

    fn list_displays(&mut self, kind: BackendKind) {
        // Reuse the open backend so enumeration matches what output uses.
        let result = match &mut self.session {
            Some(session) if session.backend.kind() == kind => session
                .backend
                .list_displays()
                .map(|d| (d, session.backend.remote_session())),
            _ => (self.factory)(kind).and_then(|mut backend| {
                let remote = backend.remote_session();
                backend.list_displays().map(|d| (d, remote))
            }),
        };
        let (result, remote_session) = match result {
            Ok((displays, remote)) => (Ok(displays), remote),
            Err(e) => (Err(e), false),
        };
        self.emit(Event::Displays {
            backend: kind,
            result,
            remote_session,
        });
    }

    fn reset_frame(profile: &DeviceProfile) -> CodeFrame {
        let raster = profile.generation.raster;
        CodeFrame::uniform(raster.width, raster.height, profile.reset_code)
    }

    fn connect(
        &mut self,
        request: u64,
        kind: BackendKind,
        selection: Option<DisplaySelection>,
        mode: Option<InputMode>,
        profile: DeviceProfile,
    ) {
        let result = self.connect_inner(kind, selection, mode, profile);
        self.emit(Event::ConnectDone { request, result });
    }

    fn connect_inner(
        &mut self,
        kind: BackendKind,
        selection: Option<DisplaySelection>,
        mode: Option<InputMode>,
        profile: DeviceProfile,
    ) -> Result<ShowReceipt, ConnectFailure> {
        let known = |error: String| ConnectFailure {
            error,
            output_known: true,
        };
        if self.session.is_some() {
            let e = "Connect ignored: disconnect first, then select the display and reconnect.";
            self.log(e);
            return Err(known(e.into()));
        }
        if let DeviceState::Unknown { .. } = self.state {
            let e = "Connect ignored: the output state is unknown; disconnect first.";
            self.log(e);
            return Err(ConnectFailure {
                error: e.into(),
                output_known: false,
            });
        }
        if let Err(e) = profile.validate() {
            self.log(format!("Connect refused: {e}"));
            self.set_state(DeviceState::Disconnected);
            return Err(known(format!("Connect refused: {e}")));
        }
        self.set_state(DeviceState::Connecting);
        let mut backend = match (self.factory)(kind) {
            Ok(b) => b,
            Err(e) => {
                self.log(format!("Connect failed: {e}"));
                self.set_state(DeviceState::Disconnected);
                return Err(known(format!("Connect failed: {e}")));
            }
        };
        let raster = profile.generation.raster;
        let requirements = OutputRequirements {
            width: raster.width,
            height: raster.height,
            mode,
        };
        let checked = backend
            .list_displays()
            .map_err(|e| format!("Enumerate displays: {e}"))
            .and_then(|displays| {
                check_output(
                    selection.as_ref(),
                    &displays,
                    &requirements,
                    backend.remote_session(),
                )
                .map_err(|reasons| describe(&reasons))
            });
        let display = match checked {
            Ok(d) => d,
            Err(e) => {
                self.log(format!("Connect refused, nothing was sent: {e}"));
                self.set_state(DeviceState::Disconnected);
                return Err(known(format!("Connect refused, nothing was sent: {e}")));
            }
        };
        let (Some(selection), Some(mode)) = (selection, mode) else {
            unreachable!("check_output requires a selection and a confirmed mode")
        };
        let target = OutputTarget {
            display: display.clone(),
            mode,
        };
        if let Err(e) = backend.open(&target) {
            let _ = backend.close();
            self.log(format!("Connect failed while creating the output: {e}"));
            self.set_state(DeviceState::Disconnected);
            return Err(known(format!(
                "Connect failed while creating the output: {e}"
            )));
        }
        self.session = Some(Session {
            backend,
            selection,
            mode,
            profile: profile.clone(),
            display: display.clone(),
        });
        let reset = Self::reset_frame(&profile);
        match self.show(&reset) {
            Ok(receipt) => {
                self.log(format!(
                    "Connected to {} ({}); reset pattern code {} shown: {}.",
                    display.label(),
                    mode.short(),
                    profile.reset_code,
                    receipt.verification.label()
                ));
                self.set_state(DeviceState::Connected {
                    backend: kind,
                    display,
                    mode,
                    showing: Showing {
                        item_id: None,
                        label: format!("Reset pattern (code {})", profile.reset_code),
                        receipt: receipt.clone(),
                    },
                });
                Ok(receipt)
            }
            Err(e) => {
                self.log(format!("Initial reset pattern failed: {e}"));
                self.set_state(DeviceState::Unknown { reason: e.clone() });
                Err(ConnectFailure {
                    error: format!("Initial reset pattern failed: {e}"),
                    output_known: false,
                })
            }
        }
    }

    /// Revalidate, encode and draw. On a draw or identity failure the output
    /// is closed and `Err` is returned; the caller marks the state unknown.
    fn show(&mut self, frame: &CodeFrame) -> Result<ShowReceipt, String> {
        let session = self.session.as_mut().ok_or("Not connected")?;
        let raster = session.profile.generation.raster;
        if (frame.width, frame.height) != (raster.width, raster.height) {
            return Err(format!(
                "Frame is {}×{}; the SLM needs {}×{}",
                frame.width, frame.height, raster.width, raster.height
            ));
        }
        let encoded = encode_bgrx(frame, session.mode, session.profile.generation.max_code)?;
        let requirements = OutputRequirements {
            width: raster.width,
            height: raster.height,
            mode: Some(session.mode),
        };
        let recheck = session
            .backend
            .list_displays()
            .map_err(|e| format!("Enumerate displays: {e}"))
            .and_then(|displays| {
                check_output(
                    Some(&session.selection),
                    &displays,
                    &requirements,
                    session.backend.remote_session(),
                )
                .map_err(|reasons| describe(&reasons))
            })
            .and_then(|display| {
                if display.desktop == session.display.desktop {
                    Ok(display)
                } else {
                    Err("The display moved in the Windows layout since Connect. Reconnect.".into())
                }
            });
        if let Err(e) = recheck {
            let _ = session.backend.close();
            self.session = None;
            return Err(format!("Output stopped and hidden: {e}"));
        }
        match session.backend.show(&encoded) {
            Ok(verification) => Ok(ShowReceipt {
                digest: frame.digest(),
                uniform_code: frame.uniform_code(),
                verification,
                display: session.display.label(),
                mode: session.mode,
                at_unix_ms: now_unix_ms(),
            }),
            Err(e) => {
                let _ = session.backend.close();
                self.session = None;
                Err(format!("Draw failed; output hidden: {e}"))
            }
        }
    }

    fn apply(&mut self, request: u64, item: ApplyItem) {
        let fail = |worker: &mut Worker, error: String, output_known: bool| {
            worker.emit(Event::ApplyFailed {
                request,
                item_id: item.item_id.clone(),
                label: item.label.clone(),
                error,
                output_known,
            });
        };
        if !self.state.is_connected() || self.session.is_none() {
            let reason = match &self.state {
                DeviceState::Unknown { .. } => {
                    "Output state is unknown; disconnect, reselect and reconnect first"
                }
                _ => "Not connected; nothing was sent",
            };
            fail(
                self,
                reason.into(),
                !matches!(self.state, DeviceState::Unknown { .. }),
            );
            return;
        }
        let frame = match &item.source {
            FrameSource::Codes(frame) => Ok((**frame).clone()),
            FrameSource::Generate { spec, profile } => spec.render(profile).map(|r| r.into_code()),
            FrameSource::Experiment { item, asset } => {
                render_item(item, asset.as_deref()).map(|r| r.into_code())
            }
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(e) => {
                fail(
                    self,
                    format!("Not sent (previous pattern still shown): {e}"),
                    true,
                );
                return;
            }
        };
        match self.show(&frame) {
            Ok(receipt) => {
                if let DeviceState::Connected {
                    backend,
                    display,
                    mode,
                    ..
                } = self.state.clone()
                {
                    self.set_state(DeviceState::Connected {
                        backend,
                        display,
                        mode,
                        showing: Showing {
                            item_id: item.item_id.clone(),
                            label: item.label.clone(),
                            receipt: receipt.clone(),
                        },
                    });
                }
                self.emit(Event::Applied {
                    request,
                    item_id: item.item_id,
                    label: item.label,
                    receipt,
                });
            }
            Err(e) => {
                // A size or encoding error is caught before drawing and
                // leaves the session open and the previous frame shown.
                if self.session.is_some() {
                    fail(
                        self,
                        format!("Not sent (previous pattern still shown): {e}"),
                        true,
                    );
                } else {
                    self.set_state(DeviceState::Unknown { reason: e.clone() });
                    fail(self, e, false);
                }
            }
        }
    }

    fn reset(&mut self, request: u64) {
        let Some(session) = &self.session else {
            self.emit(Event::ApplyFailed {
                request,
                item_id: None,
                label: "Reset pattern".into(),
                error: "Not connected; nothing was sent".into(),
                output_known: !matches!(self.state, DeviceState::Unknown { .. }),
            });
            return;
        };
        let code = session.profile.reset_code;
        let frame = Arc::new(Self::reset_frame(&session.profile));
        self.apply(
            request,
            ApplyItem {
                item_id: None,
                label: format!("Reset pattern (code {code})"),
                source: FrameSource::Codes(frame),
            },
        );
    }

    /// Reset (when the output is trusted), then close. Emits the
    /// `DisconnectDone` acknowledgement for `request`; returns the summary.
    fn disconnect(&mut self, request: u64) -> String {
        let report = self.disconnect_inner();
        let summary = report.summary.clone();
        self.emit(Event::DisconnectDone { request, report });
        summary
    }

    fn disconnect_inner(&mut self) -> DisconnectReport {
        let (reset, reset_code, summary) = match &self.session {
            None => {
                let was_unknown = matches!(self.state, DeviceState::Unknown { .. });
                self.set_state(DeviceState::Disconnected);
                let summary = if was_unknown {
                    "Disconnected. The SLM output state was unknown and is still unverified."
                } else {
                    "Already disconnected."
                };
                return DisconnectReport {
                    reset: ResetOutcome::NotOpen { was_unknown },
                    reset_code: None,
                    close_error: None,
                    summary: summary.into(),
                };
            }
            Some(session) => {
                let profile = session.profile.clone();
                if self.state.is_connected() {
                    match self.show(&Self::reset_frame(&profile)) {
                        Ok(receipt) => (
                            ResetOutcome::Confirmed(receipt),
                            Some(profile.reset_code),
                            format!(
                                "Reset pattern code {} sent before closing.",
                                profile.reset_code
                            ),
                        ),
                        Err(e) => (
                            ResetOutcome::Failed(e.clone()),
                            Some(profile.reset_code),
                            format!("Reset before closing failed: {e}."),
                        ),
                    }
                } else {
                    (
                        ResetOutcome::SkippedUnknown,
                        Some(profile.reset_code),
                        "Output state was unknown; no reset was sent.".into(),
                    )
                }
            }
        };
        let mut close_error = None;
        if let Some(mut session) = self.session.take()
            && let Err(e) = session.backend.close()
        {
            self.log(format!("Close output: {e}"));
            close_error = Some(e);
        }
        self.set_state(DeviceState::Disconnected);
        DisconnectReport {
            reset,
            reset_code,
            close_error,
            summary: format!(
                "{summary} Output closed; what the SLM shows after closing is not verified."
            ),
        }
    }

    fn poll(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        if let Some(reason) = session.backend.take_invalidation() {
            let _ = session.backend.close();
            self.session = None;
            let reason = format!("{reason} Output hidden; reselect the display and reconnect.");
            self.log(reason.clone());
            self.set_state(DeviceState::Unknown { reason });
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            match commands.recv_timeout(Duration::from_millis(200)) {
                Ok(Command::ListDisplays { backend }) => self.list_displays(backend),
                Ok(Command::Connect {
                    request,
                    backend,
                    selection,
                    mode,
                    profile,
                }) => self.connect(request, backend, selection, mode, profile),
                Ok(Command::Apply { request, item }) => self.apply(request, item),
                Ok(Command::Reset { request }) => self.reset(request),
                Ok(Command::Wavelength {
                    backend,
                    usb_device,
                    requested_nm,
                }) => {
                    let result = self.wavelength(backend, usb_device, requested_nm);
                    self.emit(Event::Wavelength(result));
                }
                Ok(Command::Disconnect { request }) => {
                    let summary = self.disconnect(request);
                    self.log(summary);
                }
                Ok(Command::Shutdown { reply }) => {
                    let summary = self.disconnect(0);
                    let _ = reply.send(Ok(summary));
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = self.disconnect(0);
                    return;
                }
            }
            self.poll();
        }
    }

    fn wavelength(
        &mut self,
        backend: BackendKind,
        usb_device: u32,
        requested_nm: Option<u32>,
    ) -> Result<u32, String> {
        if !matches!(self.state, DeviceState::Disconnected) {
            return Err("Disconnect video output before changing the wavelength".into());
        }
        if usb_device == 0
            || requested_nm.is_some_and(|nm| !crate::wavelength::SANTEC_WAVELENGTH_NM.contains(&nm))
        {
            return Err("Enter a USB device number and a wavelength from 450 to 1600 nm".into());
        }
        match backend {
            BackendKind::Demo => {
                if let Some(nm) = requested_nm {
                    self.demo_wavelength_nm = nm;
                }
                Ok(self.demo_wavelength_nm)
            }
            BackendKind::WindowsDisplay => {
                crate::wavelength::santec_wavelength(usb_device, requested_nm)
            }
        }
    }
}

pub struct DeviceWorker {
    commands: Sender<Command>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
    next_request: u64,
}

impl DeviceWorker {
    pub fn spawn(factory: BackendFactory, notify: Box<dyn Fn() + Send>) -> Self {
        let (commands, command_rx) = channel();
        let (event_tx, events) = channel();
        let worker = Worker {
            factory,
            events: event_tx,
            notify,
            session: None,
            state: DeviceState::Disconnected,
            demo_wavelength_nm: 795,
        };
        let thread = std::thread::Builder::new()
            .name("slm-device".into())
            .spawn(move || worker.run(command_rx))
            .expect("spawn device worker");
        Self {
            commands,
            events,
            thread: Some(thread),
            next_request: 1,
        }
    }

    pub fn request_id(&mut self) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        id
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "Device worker stopped".to_string())
    }

    pub fn try_events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Blocking wait for the next event (tests and headless smoke runs).
    pub fn wait_event(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Reset and close the output, then stop the worker.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<String, String> {
        let (reply, rx) = channel();
        self.send(Command::Shutdown { reply })?;
        let result = rx.recv_timeout(timeout).map_err(|_| {
            "Device worker did not confirm shutdown in time; output state unknown".to_string()
        })?;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        result
    }
}

impl Drop for DeviceWorker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.shutdown(Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_displays;
    use slm_core::{AngleValue, Geometry, PeriodValue, PhaseValue, WaveComponent, Waveform};

    struct Rig {
        worker: DeviceWorker,
        demo: DemoControl,
    }

    fn rig() -> Rig {
        let demo = DemoControl::default();
        let worker = DeviceWorker::spawn(default_factory(demo.clone()), Box::new(|| {}));
        Rig { worker, demo }
    }

    fn slm_selection() -> DisplaySelection {
        demo_displays()[1].selection()
    }

    impl Rig {
        /// Wait for the first event `f` accepts, within one overall deadline.
        fn until<T>(&self, mut f: impl FnMut(&Event) -> Option<T>) -> T {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                if let Some(event) = self.worker.wait_event(left)
                    && let Some(value) = f(&event)
                {
                    return value;
                }
            }
            panic!("expected event did not arrive within 5 s");
        }
        fn state(&self) -> DeviceState {
            self.until(|e| match e {
                Event::State(DeviceState::Connecting) => None,
                Event::State(s) => Some(s.clone()),
                _ => None,
            })
        }
        fn connect(&self, mode: Option<InputMode>) -> DeviceState {
            self.worker
                .send(Command::Connect {
                    request: 0,
                    backend: BackendKind::Demo,
                    selection: Some(slm_selection()),
                    mode,
                    profile: DeviceProfile::santec_slm200_candidate(),
                })
                .unwrap();
            self.state()
        }
        /// Apply and return its result with the last `State` seen while
        /// waiting (a failure emits `State(Unknown)` before `ApplyFailed`).
        fn apply_with_state(&mut self, source: FrameSource) -> (Event, Option<DeviceState>) {
            let request = self.worker.request_id();
            self.worker
                .send(Command::Apply {
                    request,
                    item: ApplyItem {
                        item_id: Some("item".into()),
                        label: "test".into(),
                        source,
                    },
                })
                .unwrap();
            let mut last_state = None;
            let event = self.until(|e| match e {
                Event::State(s) => {
                    last_state = Some(s.clone());
                    None
                }
                Event::Applied { request: r, .. } | Event::ApplyFailed { request: r, .. }
                    if *r == request =>
                {
                    Some(e.clone())
                }
                _ => None,
            });
            (event, last_state)
        }
        fn apply(&mut self, source: FrameSource) -> Event {
            self.apply_with_state(source).0
        }
    }

    fn wave_spec() -> FrameSource {
        FrameSource::Generate {
            spec: PatternSpec::SingleWave {
                wave: WaveComponent {
                    period: PeriodValue::px(100.),
                    waveform: Waveform::Ramp,
                    initial_phase: PhaseValue::zero(),
                },
                geometry: Geometry {
                    center_offset_px: [-16, 110],
                    rotation: AngleValue::degrees(15.),
                },
            },
            profile: DeviceProfile::santec_slm200_candidate().generation,
        }
    }

    #[test]
    fn unsupported_120hz_sends_nothing() {
        let r = rig();
        assert_eq!(r.connect(Some(InputMode::Hz120)), DeviceState::Disconnected);
        assert_eq!(r.demo.lock().shows, 0);
        assert!(!r.demo.lock().open);
    }

    #[test]
    fn unconfirmed_mode_sends_nothing() {
        let r = rig();
        assert_eq!(r.connect(None), DeviceState::Disconnected);
        assert_eq!(r.demo.lock().shows, 0);
        assert!(!r.demo.lock().open);
    }

    #[test]
    fn connect_shows_reset_then_apply_and_disconnect_resets() {
        let mut r = rig();
        let state = r.connect(Some(InputMode::Hz60));
        let DeviceState::Connected { showing, .. } = state else {
            panic!("not connected: {state:?}")
        };
        assert_eq!(showing.receipt.uniform_code, Some(0));
        assert_eq!(
            r.demo.lock().last_frame.as_ref().unwrap().uniform_code(),
            Some(0)
        );
        let event = r.apply(wave_spec());
        assert!(matches!(event, Event::Applied { .. }), "{event:?}");
        assert_eq!(r.demo.lock().shows, 2);
        assert_ne!(
            r.demo.lock().last_frame.as_ref().unwrap().uniform_code(),
            Some(0)
        );
        // Render error: nothing sent, still connected.
        let bad = FrameSource::Generate {
            spec: PatternSpec::Uniform { code: 2000 },
            profile: DeviceProfile::santec_slm200_candidate().generation,
        };
        let event = r.apply(bad);
        assert!(matches!(
            event,
            Event::ApplyFailed {
                output_known: true,
                ..
            }
        ));
        assert_eq!(r.demo.lock().shows, 2);
        r.worker.send(Command::Disconnect { request: 0 }).unwrap();
        assert_eq!(r.state(), DeviceState::Disconnected);
        let demo = r.demo.lock();
        assert_eq!(demo.shows, 3);
        assert_eq!(demo.last_frame.as_ref().unwrap().uniform_code(), Some(0));
        assert!(!demo.open);
    }

    #[test]
    fn renumbered_or_duplicated_display_stops_output() {
        let mut r = rig();
        assert!(r.connect(Some(InputMode::Hz60)).is_connected());
        let mut displays = demo_displays();
        displays[1].gdi_name = r"\\.\DISPLAY3".into();
        r.demo.lock().displays = displays;
        let event = r.apply(wave_spec());
        assert!(
            matches!(
                event,
                Event::ApplyFailed {
                    output_known: false,
                    ..
                }
            ),
            "{event:?}"
        );
        assert!(!r.demo.lock().open);
        assert_eq!(r.demo.lock().shows, 1);
        // Unknown until disconnected; Apply refuses.
        let event = r.apply(wave_spec());
        assert!(matches!(
            event,
            Event::ApplyFailed {
                output_known: false,
                ..
            }
        ));
    }

    #[test]
    fn display_change_while_idle_marks_unknown() {
        let r = rig();
        assert!(r.connect(Some(InputMode::Hz60)).is_connected());
        let mut displays = demo_displays();
        displays[1].clone_count = 2;
        r.demo
            .simulate_display_change(displays, "Windows display configuration changed.");
        assert!(matches!(r.state(), DeviceState::Unknown { .. }));
        assert!(!r.demo.lock().open);
    }

    #[test]
    fn draw_failure_is_unknown_not_applied() {
        let mut r = rig();
        assert!(r.connect(Some(InputMode::Hz60)).is_connected());
        r.demo.lock().fail_next_show = Some("device lost".into());
        let (event, state) = r.apply_with_state(wave_spec());
        assert!(
            matches!(
                event,
                Event::ApplyFailed {
                    output_known: false,
                    ..
                }
            ),
            "{event:?}"
        );
        assert!(
            matches!(state, Some(DeviceState::Unknown { .. })),
            "{state:?}"
        );
        assert!(!r.demo.lock().open);
    }

    #[test]
    fn remote_session_and_primary_are_refused() {
        let r = rig();
        r.demo.lock().remote_session = true;
        assert_eq!(r.connect(Some(InputMode::Hz60)), DeviceState::Disconnected);
        r.demo.lock().remote_session = false;
        r.worker
            .send(Command::Connect {
                request: 0,
                backend: BackendKind::Demo,
                selection: Some(demo_displays()[0].selection()),
                mode: Some(InputMode::Hz60),
                profile: DeviceProfile::santec_slm200_candidate(),
            })
            .unwrap();
        assert_eq!(r.state(), DeviceState::Disconnected);
        assert_eq!(r.demo.lock().shows, 0);
    }

    fn connect_ack(
        r: &Rig,
        request: u64,
        mode: Option<InputMode>,
    ) -> Result<ShowReceipt, ConnectFailure> {
        r.worker
            .send(Command::Connect {
                request,
                backend: BackendKind::Demo,
                selection: Some(slm_selection()),
                mode,
                profile: DeviceProfile::santec_slm200_candidate(),
            })
            .unwrap();
        r.until(|e| match e {
            Event::ConnectDone { request: q, result } if *q == request => Some(result.clone()),
            _ => None,
        })
    }

    fn disconnect_ack(r: &Rig, request: u64) -> DisconnectReport {
        r.worker.send(Command::Disconnect { request }).unwrap();
        r.until(|e| match e {
            Event::DisconnectDone { request: q, report } if *q == request => Some(report.clone()),
            _ => None,
        })
    }

    #[test]
    fn connect_and_disconnect_are_acknowledged_with_their_request() {
        let r = rig();
        // A refused connect from `Disconnected` changes no state, but is
        // still acknowledged, with the output known (nothing was sent).
        let refused = connect_ack(&r, 7, None).unwrap_err();
        assert!(refused.output_known, "{refused:?}");
        assert_eq!(r.demo.lock().shows, 0);
        let receipt = connect_ack(&r, 8, Some(InputMode::Hz60)).unwrap();
        assert_eq!(receipt.uniform_code, Some(0));
        // A second connect is ignored and acknowledged as a known failure.
        assert!(
            connect_ack(&r, 9, Some(InputMode::Hz60))
                .unwrap_err()
                .output_known
        );
        let report = disconnect_ack(&r, 10);
        assert!(report.reset_confirmed(), "{report:?}");
        assert_eq!(report.reset_code, Some(0));
        assert!(!r.demo.lock().open);
        // Disconnecting again is a no-op, still acknowledged.
        let report = disconnect_ack(&r, 11);
        assert_eq!(report.reset, ResetOutcome::NotOpen { was_unknown: false });
        assert!(!report.reset_confirmed());
    }

    #[test]
    fn failed_reset_before_close_is_reported_not_confirmed() {
        let r = rig();
        connect_ack(&r, 1, Some(InputMode::Hz60)).unwrap();
        r.demo.lock().fail_next_show = Some("device lost".into());
        let report = disconnect_ack(&r, 2);
        assert!(
            matches!(report.reset, ResetOutcome::Failed(_)),
            "{report:?}"
        );
        assert!(!report.reset_confirmed());
        assert!(!r.demo.lock().open);
        // After an unknown output, disconnect reports it was unknown.
        connect_ack(&r, 3, Some(InputMode::Hz60)).unwrap();
        r.demo.lock().pending_invalidation = Some("moved".into());
        assert!(matches!(r.state(), DeviceState::Unknown { .. }));
        assert!(connect_ack(&r, 4, Some(InputMode::Hz60)).is_err_and(|f| !f.output_known));
        let report = disconnect_ack(&r, 5);
        assert_eq!(report.reset, ResetOutcome::NotOpen { was_unknown: true });
    }

    #[test]
    fn shutdown_resets() {
        let mut r = rig();
        assert!(r.connect(Some(InputMode::Hz60)).is_connected());
        let _ = r.apply(wave_spec());
        let summary = r.worker.shutdown(Duration::from_secs(10)).unwrap();
        assert!(summary.contains("Reset pattern code 0 sent"), "{summary}");
        assert_eq!(
            r.demo.lock().last_frame.as_ref().unwrap().uniform_code(),
            Some(0)
        );
    }
}
