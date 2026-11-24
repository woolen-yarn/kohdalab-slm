//! KohdaLab SLM desktop window.
//!
//! Left: Config, Device (output display, input-mode confirmation, connect,
//! Reset Pattern). Center: Uniform / Single Wave / Multi-Wave / Sweep /
//! Experiment tabs. Right (collapsible): applied condition and log.
//!
//! Status words follow the worker contract: `Applying` while a request is in
//! flight, `Applied` only after the backend confirmed the draw, `Output
//! unknown` after any failure that may have left the SLM in another state.
//! Switching tabs or loading files never sends anything.

use crate::theme;
use eframe::egui;
use slm_core::experiment::ExperimentDoc;
use slm_core::heatmap::HeatmapKind;
use slm_core::sweep::SweepTarget;
use slm_core::{
    AngleValue, Geometry, PatternSpec, PeriodValue, PhaseValue, WaveComponent, Waveform,
};
use slm_device::backend::BackendKind;
use slm_device::demo::{DemoControl, demo_displays};
use slm_device::display::DisplayInfo;
use slm_device::worker::{DeviceState, default_factory};
use slm_device::{AppConfig, DeviceWorker, InputMode, ModeConfirmation, ShowReceipt};
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

mod control;
mod device;
mod experiment;
mod preview_window;
mod ui;
#[cfg(test)]
pub(crate) use ui::{EXTERNAL_CONTROL, EXTERNAL_LEASED};

const MAX_LOG: usize = 500;

/// Whether the active proportional font draws `⁻` (Segoe UI does; egui's
/// bundled font does not). Checked once fonts are loaded.
static SUPERSCRIPT_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Unit text for display: `µm⁻¹` falls back to `µm^-1` without the glyph.
pub fn ui_text(text: &str) -> String {
    if SUPERSCRIPT_OK.load(std::sync::atomic::Ordering::Relaxed) {
        text.to_string()
    } else {
        text.replace("⁻¹", "^-1")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Uniform,
    Single,
    Multi,
    Sweep,
    Experiment,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Uniform,
        Tab::Single,
        Tab::Multi,
        Tab::Sweep,
        Tab::Experiment,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Uniform => "Uniform",
            Tab::Single => "Single Wave",
            Tab::Multi => "Multi-Wave",
            Tab::Sweep => "Sweep",
            Tab::Experiment => "Experiment",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedRecord {
    pub label: String,
    pub item_id: Option<String>,
    pub summary: String,
    pub spec: Option<PatternSpec>,
}

#[derive(Clone, Debug)]
pub enum ApplyStatus {
    Idle,
    Applying(AppliedRecord),
    Applied(AppliedRecord, ShowReceipt),
    Failed {
        record: AppliedRecord,
        error: String,
        output_known: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct WaveDraft {
    pub wave: WaveComponent,
    pub geometry: Geometry,
}

#[derive(Clone, Debug)]
pub struct MultiDraft {
    pub components: Vec<WaveComponent>,
    pub geometry: Geometry,
    pub band_override: bool,
    pub reverse_band_order: bool,
    pub band_width: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepBase {
    Uniform,
    Single,
    Multi,
}

#[derive(Clone, Debug)]
pub struct SweepDraft {
    pub base: SweepBase,
    pub target: SweepTarget,
    pub start: f64,
    pub end: f64,
    pub step: f64,
    pub include_end: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Discard {
    New,
    Open,
}

pub struct AppOptions {
    pub config_path: PathBuf,
    pub demo: bool,
    /// Keeps a demo-only config directory alive for the session.
    pub tempdir: Option<tempfile::TempDir>,
    /// Demo launch check: connect, apply one pattern, save a screenshot of
    /// this window to the path, then close.
    pub capture: Option<PathBuf>,
    /// `--gui-api`: start the loopback control adapter (reads, plus leased
    /// output operations). Off by default; nothing is bound or written
    /// unless this is set.
    pub gui_api: bool,
}

#[derive(Debug, Default)]
struct Capture {
    path: PathBuf,
    tab: Option<Tab>,
    frames: u32,
    stage: u8,
    result: Option<Result<(), String>>,
}

pub struct App {
    pub config: AppConfig,
    pub config_path: PathBuf,
    pub config_dirty: bool,
    pub demo: bool,
    _tempdir: Option<tempfile::TempDir>,
    pub worker: DeviceWorker,
    pub demo_control: DemoControl,
    pub device_state: DeviceState,
    pub pending: Option<(u64, AppliedRecord)>,
    pub status: ApplyStatus,
    pub displays: Vec<DisplayInfo>,
    pub displays_error: Option<String>,
    pub remote_session: bool,
    pub tab: Tab,
    pub uniform_code: u16,
    pub single: WaveDraft,
    pub multi: MultiDraft,
    pub sweep: SweepDraft,
    pub experiment: ExperimentDoc,
    pub selected: Option<usize>,
    selected_rows: BTreeSet<usize>,
    selection_anchor: Option<usize>,
    reveal_row: Option<usize>,
    confirm_remove_all: bool,
    auto_apply_rows: bool,
    queued_row: Option<usize>,
    pub heatmaps: HashSet<HeatmapKind>,
    pub log: VecDeque<LogLine>,
    pub right_collapsed: bool,
    unit_error: Option<String>,
    mode_choice: InputMode,
    mode_note: String,
    mode_ack: bool,
    confirm_discard: Option<Discard>,
    shut_down: bool,
    capture: Option<Capture>,
    glyphs_checked: bool,
    pub left_collapsed: bool,
    pub device_expanded: bool,
    pub settings_open: bool,
    wavelength_nm: String,
    wavelength_usb_device: String,
    wavelength_busy: bool,
    wavelength_status: Option<(String, egui::Color32)>,
    pub config_path_text: String,
    previewer: crate::preview::Previewer,
    pub preview: PreviewState,
    /// Sweep tab: preview the explicitly clicked list row instead of the
    /// first sweep value. Preview only; never sends.
    pub sweep_preview_row: bool,
    /// Startup fit to the monitor (normal and demo launches, not captures).
    pub window_fit: WindowFitState,
    /// Full per-value validation of the sweep draft, keyed by the draft and
    /// profile so it is recomputed only after an edit.
    sweep_check: std::cell::RefCell<Option<(String, Result<(), String>)>>,
    /// External control (`--gui-api`); `None` in standalone use.
    pub control_api: Option<crate::control_api::Server>,
    /// Writer lease, job receipts and request-ID record for the API.
    pub control: crate::control_lease::Engine,
    /// Disconnect sent; cleared when the worker reports `Disconnected`.
    pub disconnecting: bool,
    /// Why the output became unknown; cleared only by a confirmed draw.
    pub output_unverified: Option<String>,
}

#[derive(Default)]
pub struct WindowFitState {
    pub done: bool,
    pub frames: u32,
    /// Usable screen area once known; caps the minimum window size.
    pub limit: Option<egui::Vec2>,
}

/// Input Pattern preview bookkeeping. `shown` is the key of `texture`; it is
/// drawn only while it equals the key of the current input.
#[derive(Default)]
pub struct PreviewState {
    pub requested: Option<String>,
    pub shown: Option<String>,
    pub texture: Option<egui::TextureHandle>,
    pub info: Option<crate::preview::PreviewImage>,
    pub error: Option<(String, String)>,
}

fn utc_clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}Z", day / 3600, day % 3600 / 60, day % 60)
}

fn default_wave(profile: &slm_device::DeviceProfile) -> WaveComponent {
    let _ = profile;
    WaveComponent {
        period: PeriodValue::px(200.),
        waveform: Waveform::Ramp,
        initial_phase: PhaseValue::zero(),
    }
}

fn default_geometry(profile: &slm_device::DeviceProfile) -> Geometry {
    Geometry {
        center_offset_px: profile.default_center_offset_px,
        rotation: AngleValue::degrees(0.),
    }
}

impl App {
    pub fn new(ctx: &egui::Context, options: AppOptions) -> Self {
        theme::configure(ctx);
        let (mut config, warning) = if options.demo {
            (AppConfig::default(), None)
        } else {
            AppConfig::load_or_default(&options.config_path)
        };
        if options.demo {
            config.output.backend = BackendKind::Demo;
            config.output.display = Some(demo_displays()[1].selection());
            config.output.confirmed_mode = Some(ModeConfirmation {
                mode: InputMode::Hz60,
                confirmed_unix_s: 0,
                note: "Demo: simulated confirmation, no hardware".into(),
            });
        }
        let demo_control = DemoControl::default();
        let repaint = ctx.clone();
        let worker = DeviceWorker::spawn(
            default_factory(demo_control.clone()),
            Box::new(move || repaint.request_repaint()),
        );
        let profile = config.profile.clone();
        let mut app = Self {
            config_path: options.config_path,
            config_dirty: false,
            demo: options.demo,
            _tempdir: options.tempdir,
            worker,
            demo_control,
            device_state: DeviceState::Disconnected,
            pending: None,
            status: ApplyStatus::Idle,
            displays: Vec::new(),
            displays_error: None,
            remote_session: false,
            tab: Tab::Uniform,
            uniform_code: 0,
            single: WaveDraft {
                wave: default_wave(&profile),
                geometry: default_geometry(&profile),
            },
            multi: MultiDraft {
                components: vec![
                    default_wave(&profile),
                    WaveComponent {
                        period: PeriodValue::px(100.),
                        ..default_wave(&profile)
                    },
                ],
                geometry: default_geometry(&profile),
                band_override: false,
                reverse_band_order: false,
                band_width: profile.generation.default_stripe_width_px,
            },
            sweep: SweepDraft {
                base: SweepBase::Uniform,
                target: SweepTarget::UniformCode,
                start: 0.,
                end: 1023.,
                step: 10.,
                include_end: false,
            },
            experiment: ExperimentDoc::new("Untitled"),
            selected: None,
            selected_rows: BTreeSet::new(),
            selection_anchor: None,
            reveal_row: None,
            confirm_remove_all: false,
            auto_apply_rows: options.demo,
            queued_row: None,
            heatmaps: HeatmapKind::ALL.into_iter().collect(),
            log: VecDeque::new(),
            right_collapsed: false,
            unit_error: None,
            mode_choice: InputMode::Hz60,
            mode_note: String::new(),
            mode_ack: false,
            confirm_discard: None,
            shut_down: false,
            glyphs_checked: false,
            left_collapsed: false,
            device_expanded: false,
            settings_open: false,
            wavelength_nm: profile
                .device_wavelength_nm
                .filter(|nm| nm.fract() == 0. && (450. ..=1600.).contains(nm))
                .map_or_else(|| "795".into(), |nm| nm.to_string()),
            wavelength_usb_device: "1".into(),
            wavelength_busy: false,
            wavelength_status: None,
            config_path_text: String::new(),
            previewer: {
                let repaint = ctx.clone();
                crate::preview::Previewer::spawn(Box::new(move || repaint.request_repaint()))
            },
            preview: PreviewState::default(),
            sweep_preview_row: false,
            window_fit: WindowFitState::default(),
            sweep_check: std::cell::RefCell::new(None),
            control_api: None,
            control: Default::default(),
            disconnecting: false,
            output_unverified: None,
            capture: options.capture.map(|path| Capture {
                path,
                ..Default::default()
            }),
            config,
        };
        app.config_path_text = ui::tilde(&app.config_path);
        if let Some(warning) = warning {
            app.push_log(Level::Warn, warning);
        }
        app.push_log(
            Level::Info,
            if app.demo {
                "Demo session: simulated output only; configuration is temporary.".to_string()
            } else {
                format!("Config: {}", app.config_path.display())
            },
        );
        if options.gui_api {
            let wake = ctx.clone();
            match crate::control_api::Server::start(None, move || wake.request_repaint()) {
                Ok(server) => {
                    app.control_api = Some(server);
                    app.push_log(
                        Level::Info,
                        "External control: GUI API on loopback. A leased client can connect, apply, reset and stop; Stop and Take control stay in this window.",
                    );
                }
                Err(e) => app.push_log(Level::Warn, format!("GUI API not started: {e}")),
            }
        }
        app.refresh_displays();
        app
    }

    /// External control is active (state, Stop and Take control in the tab row).
    pub fn external_control(&self) -> bool {
        self.control_api
            .as_ref()
            .is_some_and(crate::control_api::Server::is_active)
    }

    pub(crate) fn control_snapshot(&self) -> crate::control_api::Snapshot {
        use crate::control_api::LastApply;
        crate::control_api::Snapshot {
            demo: self.demo,
            config: self.config.clone(),
            config_saved: !self.config_dirty,
            device: self.device_state.clone(),
            last_apply: match &self.status {
                ApplyStatus::Idle => LastApply::Idle,
                ApplyStatus::Applying(r) => LastApply::Applying {
                    label: r.label.clone(),
                },
                ApplyStatus::Applied(r, _) => LastApply::Applied {
                    label: r.label.clone(),
                },
                ApplyStatus::Failed {
                    record,
                    error,
                    output_known,
                } => LastApply::Failed {
                    label: record.label.clone(),
                    error: error.clone(),
                    output_known: *output_known,
                },
            },
            wavelength_busy: self.wavelength_busy,
            disconnecting: self.disconnecting,
            output_unverified: self.output_unverified.clone(),
            home: slm_device::config::home_dir(),
            lease_owned: self.control.lease.is_some(),
            operation: self
                .control
                .running()
                .map(|job| (job.id.clone(), job.kind.name())),
            closing: self.control.running_close().is_some(),
            last_revocation: self.control.last_revocation.map(|r| r.name()),
        }
    }

    pub fn push_log(&mut self, level: Level, text: impl Into<String>) {
        self.log.push_back(LogLine {
            time: utc_clock(),
            level,
            text: text.into(),
        });
        while self.log.len() > MAX_LOG {
            self.log.pop_front();
        }
    }

    pub fn connected(&self) -> bool {
        self.device_state.is_connected()
    }

    pub fn save_config(&mut self) {
        match self.config.save(&self.config_path) {
            Ok(()) => {
                self.config_dirty = false;
                self.push_log(
                    Level::Info,
                    format!("Config saved: {}", self.config_path.display()),
                );
            }
            Err(e) => self.push_log(Level::Error, format!("Config not saved: {e}")),
        }
    }

    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        match self.worker.shutdown(Duration::from_secs(5)) {
            Ok(summary) => eprintln!("KohdaLab SLM: {summary}"),
            Err(e) => eprintln!("KohdaLab SLM: {e}"),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.glyphs_checked {
            let ok = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(12.), "⁻¹"));
            SUPERSCRIPT_OK.store(ok, std::sync::atomic::Ordering::Relaxed);
            self.glyphs_checked = true;
        }
        self.fit_window_once(ctx);
        self.process_events();
        // After worker events, so status reflects this frame's device state.
        self.process_control_api();
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.wavelength_busy {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                return;
            }
            self.shutdown();
            return;
        }
        if self.auto_apply_rows
            && self.apply_block().is_none()
            && let Some(index) = self.queued_row.take()
        {
            self.apply_experiment_item(index);
        }
        self.process_previews(ctx);
        self.capture_step(ctx);
        if self.pending.is_some() || matches!(self.device_state, DeviceState::Connecting) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // Lease expiry is checked per frame; keep frames coming while leased.
        if self.control.lease.is_some() || self.control.running().is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(root);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}
