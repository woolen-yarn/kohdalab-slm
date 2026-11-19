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

use crate::theme::{self, AMBER, ERROR, MUTED, TEAL};
use eframe::egui::{self, RichText};
use slm_core::experiment::{ExperimentDoc, ExperimentItem, SweepOrigin, new_id};
use slm_core::heatmap::HeatmapKind;
use slm_core::sweep::{SweepSpec, SweepTarget};
use slm_core::units::format_number;
use slm_core::{
    AngleUnit, AngleValue, Geometry, PatternSpec, PeriodUnit, PeriodValue, PhaseUnit, PhaseValue,
    WaveComponent, Waveform,
};
use slm_device::backend::BackendKind;
use slm_device::demo::{DemoControl, demo_displays};
use slm_device::display::{DisplayInfo, OutputRequirements, check_output};
use slm_device::worker::{DeviceState, default_factory};
use slm_device::{
    AppConfig, ApplyItem, Command, DeviceWorker, Event, FrameSource, InputMode, ModeConfirmation,
    ShowReceipt,
};
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod ui;

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
        app.refresh_displays();
        app
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

    fn device_busy(&self) -> bool {
        self.wavelength_busy || !matches!(self.device_state, DeviceState::Disconnected)
    }

    fn request_wavelength(&mut self, write: bool) {
        if self.device_busy() {
            return;
        }
        let Some(usb_device) = self.wavelength_device_number() else {
            return;
        };
        let requested_nm = if write {
            let Some(nm) = self.wavelength_target() else {
                return;
            };
            Some(nm)
        } else {
            None
        };
        let command = Command::Wavelength {
            backend: self.config.output.backend,
            usb_device,
            requested_nm,
        };
        match self.worker.send(command) {
            Ok(()) => {
                self.wavelength_busy = true;
                self.wavelength_status = Some((
                    if write {
                        "Writing wavelength…"
                    } else {
                        "Reading wavelength…"
                    }
                    .into(),
                    AMBER,
                ));
            }
            Err(error) => self.wavelength_status = Some((error, ERROR)),
        }
    }

    fn wavelength_target(&self) -> Option<u32> {
        self.wavelength_nm
            .trim()
            .parse()
            .ok()
            .filter(|nm| slm_device::wavelength::SANTEC_WAVELENGTH_NM.contains(nm))
    }

    fn wavelength_device_number(&self) -> Option<u32> {
        self.wavelength_usb_device
            .trim()
            .parse()
            .ok()
            .filter(|n| (1..=255).contains(n))
    }

    pub fn refresh_displays(&mut self) {
        let _ = self.worker.send(Command::ListDisplays {
            backend: self.config.output.backend,
        });
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

    // ----- worker events ------------------------------------------------

    pub fn process_events(&mut self) {
        for event in self.worker.try_events() {
            match event {
                Event::Displays {
                    backend,
                    result,
                    remote_session,
                } => {
                    if backend != self.config.output.backend {
                        continue;
                    }
                    self.remote_session = remote_session;
                    match result {
                        Ok(displays) => {
                            self.displays = displays;
                            self.displays_error = None;
                        }
                        Err(e) => {
                            self.displays.clear();
                            self.displays_error = Some(e);
                        }
                    }
                }
                Event::State(state) => {
                    if !state.is_connected() {
                        self.queued_row = None;
                    }
                    if let DeviceState::Unknown { reason } = &state {
                        if let Some((_, record)) = self.pending.take() {
                            self.status = ApplyStatus::Failed {
                                record,
                                error: reason.clone(),
                                output_known: false,
                            };
                        }
                        self.push_log(Level::Error, format!("Output unknown: {reason}"));
                    }
                    self.device_state = state;
                }
                Event::Applied {
                    request,
                    label,
                    receipt,
                    ..
                } => {
                    let record = match self.pending.take() {
                        Some((r, record)) if r == request => record,
                        other => {
                            self.pending = other;
                            continue;
                        }
                    };
                    self.push_log(
                        Level::Info,
                        format!("Applied {label}: {}", receipt.verification.label()),
                    );
                    self.status = ApplyStatus::Applied(record, receipt);
                }
                Event::ApplyFailed {
                    request,
                    label,
                    error,
                    output_known,
                    ..
                } => {
                    let record = match self.pending.take() {
                        Some((r, record)) if r == request => record,
                        other => {
                            self.pending = other;
                            AppliedRecord {
                                label: label.clone(),
                                item_id: None,
                                summary: String::new(),
                                spec: None,
                            }
                        }
                    };
                    self.push_log(
                        Level::Error,
                        format!(
                            "{label} not applied: {error}{}",
                            if output_known {
                                ""
                            } else {
                                " Output state unknown."
                            }
                        ),
                    );
                    self.status = ApplyStatus::Failed {
                        record,
                        error,
                        output_known,
                    };
                }
                Event::Wavelength(result) => {
                    self.wavelength_busy = false;
                    let (text, color, level) = match result {
                        Ok(nm) => {
                            self.wavelength_nm = nm.to_string();
                            let value = Some(f64::from(nm));
                            self.config_dirty |= self.config.profile.device_wavelength_nm != value;
                            self.config.profile.device_wavelength_nm = value;
                            let source = if self.config.output.backend == BackendKind::Demo {
                                "Simulated readback"
                            } else {
                                "USB readback"
                            };
                            (format!("{source}: {nm} nm"), TEAL, Level::Info)
                        }
                        Err(error) => (error, ERROR, Level::Error),
                    };
                    self.push_log(level, text.clone());
                    self.wavelength_status = Some((text, color));
                }
                Event::Log(text) => {
                    let level = if text.contains("refused") || text.contains("failed") {
                        Level::Warn
                    } else {
                        Level::Info
                    };
                    self.push_log(level, text);
                }
            }
        }
    }

    // ----- actions --------------------------------------------------------

    pub fn connect(&mut self) {
        let Some(display) = self.config.output.display.clone() else {
            self.push_log(Level::Warn, "Select an output display first.");
            return;
        };
        self.push_log(Level::Info, format!("Connecting to {}…", display.label()));
        let _ = self.worker.send(Command::Connect {
            backend: self.config.output.backend,
            selection: Some(display),
            mode: self.config.confirmed_mode(),
            profile: self.config.profile.clone(),
        });
        self.device_state = DeviceState::Connecting;
    }

    pub fn disconnect(&mut self) {
        self.queued_row = None;
        let request = self.worker.request_id();
        let _ = self.worker.send(Command::Disconnect { request });
        self.push_log(
            Level::Info,
            "Disconnect requested: reset pattern first, then close.",
        );
    }

    pub fn reset_pattern(&mut self) {
        self.queued_row = None;
        let request = self.worker.request_id();
        let record = AppliedRecord {
            label: format!("Reset pattern (code {})", self.config.profile.reset_code),
            item_id: None,
            summary: format!("Uniform raw code {}", self.config.profile.reset_code),
            spec: Some(PatternSpec::Uniform {
                code: self.config.profile.reset_code,
            }),
        };
        self.status = ApplyStatus::Applying(record.clone());
        self.pending = Some((request, record));
        let _ = self.worker.send(Command::Reset { request });
    }

    pub fn apply_spec(&mut self, label: String, spec: PatternSpec) {
        let profile = self.config.profile.generation;
        let record = AppliedRecord {
            label: label.clone(),
            item_id: None,
            summary: spec.summary(&profile.optics),
            spec: Some(spec.clone()),
        };
        self.send_apply(
            record,
            ApplyItem {
                item_id: None,
                label,
                source: FrameSource::Generate { spec, profile },
            },
        );
    }

    fn send_apply(&mut self, record: AppliedRecord, item: ApplyItem) {
        if self.pending.is_some() {
            self.push_log(
                Level::Warn,
                "A pattern is still being applied; wait for the result.",
            );
            return;
        }
        self.queued_row = None;
        let request = self.worker.request_id();
        self.status = ApplyStatus::Applying(record.clone());
        self.pending = Some((request, record));
        let _ = self.worker.send(Command::Apply { request, item });
    }

    pub fn apply_experiment_item(&mut self, index: usize) {
        if self.apply_block().is_some() {
            return;
        }
        let Some(item) = self.experiment.experiment.items.get(index).cloned() else {
            return;
        };
        self.set_selection(Some(index));
        self.reveal_row = Some(index);
        self.sweep_preview_row = true;
        let label = format!("#{} {}", index + 1, item.label);
        let record = AppliedRecord {
            label: item.label.clone(),
            item_id: Some(item.id.clone()),
            summary: item.summary(),
            spec: match &item.source {
                slm_core::experiment::ItemSource::Generated { spec } => Some(spec.clone()),
                _ => None,
            },
        };
        let asset = self.experiment.asset_path(&item);
        self.send_apply(
            record,
            ApplyItem {
                item_id: Some(item.id.clone()),
                label,
                source: FrameSource::Experiment { item, asset },
            },
        );
    }

    /// Index of the experiment item the worker reports as shown.
    pub fn showing_index(&self) -> Option<usize> {
        let DeviceState::Connected { showing, .. } = &self.device_state else {
            return None;
        };
        let id = showing.item_id.as_ref()?;
        self.experiment
            .experiment
            .items
            .iter()
            .position(|i| &i.id == id)
    }

    fn adjacent_row(&self, forward: bool) -> Option<usize> {
        let len = self.experiment.experiment.items.len();
        let index = match (self.selected.or_else(|| self.showing_index()), forward) {
            (Some(i), true) => i.checked_add(1)?,
            (Some(i), false) => i.checked_sub(1)?,
            (None, true) => 0,
            (None, false) => len.checked_sub(1)?,
        };
        (index < len).then_some(index)
    }

    fn set_selection(&mut self, index: Option<usize>) {
        self.queued_row = None;
        self.selected = index;
        self.selection_anchor = index;
        self.selected_rows.clear();
        self.selected_rows.extend(index);
    }

    fn select_row(&mut self, index: usize, modifiers: egui::Modifiers) {
        self.queued_row = None;
        if modifiers.shift {
            let anchor = *self.selection_anchor.get_or_insert(index);
            if !modifiers.command {
                self.selected_rows.clear();
            }
            self.selected_rows
                .extend(anchor.min(index)..=anchor.max(index));
            self.selected = Some(index);
        } else if modifiers.command {
            self.selection_anchor = Some(index);
            self.selected = if self.selected_rows.remove(&index) {
                self.selected_rows.first().copied()
            } else {
                self.selected_rows.insert(index);
                Some(index)
            };
        } else {
            self.set_selection(Some(index));
            if self.auto_apply_rows && self.connected() {
                if self.pending.is_some() {
                    self.queued_row = Some(index);
                } else {
                    self.apply_experiment_item(index);
                }
            }
        }
        self.sweep_preview_row = self.selected.is_some();
    }

    fn move_rows(&mut self, dragged_id: &str, gap: usize) {
        self.queued_row = None;
        let items = &mut self.experiment.experiment.items;
        let Some(source) = items.iter().position(|item| item.id == dragged_id) else {
            return;
        };
        let moving = if self.selected_rows.contains(&source) {
            self.selected_rows.clone()
        } else {
            BTreeSet::from([source])
        };
        let start = gap - moving.range(..gap).count();
        if moving.iter().copied().eq(start..start + moving.len()) {
            return;
        }
        let focus = start + moving.range(..source).count();
        let mut block = Vec::with_capacity(moving.len());
        let mut rest = Vec::with_capacity(items.len() - moving.len());
        for (index, item) in std::mem::take(items).into_iter().enumerate() {
            if moving.contains(&index) {
                block.push(item);
            } else {
                rest.push(item);
            }
        }
        rest.splice(start..start, block);
        *items = rest;
        self.selected = Some(focus);
        self.selection_anchor = Some(focus);
        self.selected_rows = (start..start + moving.len()).collect();
        self.reveal_row = Some(focus);
        self.sweep_preview_row = true;
        self.experiment.dirty = true;
    }

    fn remove_rows(&mut self, all: bool) {
        let items = &mut self.experiment.experiment.items;
        let count = items.len();
        let first = self.selected_rows.first().copied();
        let mut index = 0;
        items.retain(|item| {
            let remove = all || self.selected_rows.contains(&index);
            index += 1;
            if remove {
                self.experiment.pending_assets.remove(&item.id);
            }
            !remove
        });
        let remaining = items.len();
        if remaining != count {
            self.experiment.dirty = true;
            let next = remaining
                .checked_sub(1)
                .zip(first)
                .map(|(last, i)| i.min(last));
            self.set_selection(next);
            self.reveal_row = next;
            self.push_log(
                Level::Info,
                format!("Removed {} list items.", count - remaining),
            );
        }
    }

    pub fn add_to_experiment(&mut self, label: String, spec: PatternSpec) {
        let profile = self.config.profile.generation;
        if let Err(e) = spec.validate(&profile) {
            self.push_log(Level::Warn, format!("Not added: {e}"));
            return;
        }
        self.experiment
            .push(ExperimentItem::generated(label, spec, profile));
        let index = self.experiment.experiment.items.len() - 1;
        self.set_selection(Some(index));
        self.reveal_row = Some(index);
        self.push_log(Level::Info, "Added 1 item to the experiment.");
    }

    pub fn uniform_spec(&self) -> PatternSpec {
        PatternSpec::Uniform {
            code: self.uniform_code,
        }
    }
    pub fn single_spec(&self) -> PatternSpec {
        PatternSpec::SingleWave {
            wave: self.single.wave,
            geometry: self.single.geometry,
        }
    }
    pub fn multi_spec(&self) -> PatternSpec {
        PatternSpec::MultiWave {
            components: self.multi.components.clone(),
            geometry: self.multi.geometry,
            stripe_width_px: self.multi.band_override.then_some(self.multi.band_width),
            reverse_band_order: self.multi.reverse_band_order,
        }
    }
    pub fn sweep_spec(&self) -> SweepSpec {
        SweepSpec {
            base: match self.sweep.base {
                SweepBase::Uniform => self.uniform_spec(),
                SweepBase::Single => self.single_spec(),
                SweepBase::Multi => self.multi_spec(),
            },
            target: self.sweep.target,
            start: self.sweep.start,
            end: self.sweep.end,
            step: self.sweep.step,
            include_end: self.sweep.include_end,
        }
    }

    pub fn add_sweep(&mut self) {
        let spec = self.sweep_spec();
        let profile = self.config.profile.generation;
        let expanded = spec.expand().and_then(|items| {
            for (value, s) in &items {
                s.validate(&profile)
                    .map_err(|e| format!("Value {}: {e}", format_number(*value)))?;
            }
            Ok(items)
        });
        let items = match expanded {
            Ok(items) => items,
            Err(e) => {
                self.push_log(Level::Warn, format!("Sweep not added: {e}"));
                return;
            }
        };
        let sweep_id = new_id();
        let count = items.len();
        let first_added = self.experiment.experiment.items.len();
        let target_label = spec.target.label(&spec.base);
        let unit = spec.target.unit(&spec.base).unwrap_or_default();
        for (index, (value, s)) in items.into_iter().enumerate() {
            let mut item = ExperimentItem::generated(
                format!(
                    "{target_label} = {} ({}/{count})",
                    format_number(value),
                    index + 1
                ),
                s,
                profile,
            );
            item.sweep = Some(SweepOrigin {
                sweep_id: sweep_id.clone(),
                index,
                count,
                target: spec.target,
                target_label: target_label.clone(),
                value,
                unit: unit.clone(),
            });
            self.experiment.push(item);
        }
        self.reveal_row = Some(first_added);
        self.push_log(
            Level::Info,
            format!("Added sweep of {count} items ({target_label})."),
        );
    }

    pub fn import_csv(&mut self, path: &Path) {
        match self
            .experiment
            .import_csv(path, self.config.profile.generation)
        {
            Ok(_) => {
                let index = self.experiment.experiment.items.len() - 1;
                self.set_selection(Some(index));
                self.reveal_row = Some(index);
                self.push_log(
                    Level::Info,
                    format!(
                        "Imported {} (copied into the experiment on save).",
                        path.display()
                    ),
                );
            }
            Err(e) => self.push_log(Level::Error, format!("Import failed: {e}")),
        }
    }

    pub fn save_experiment(&mut self, path: &Path) {
        match self.experiment.save(path) {
            Ok(()) => self.push_log(Level::Info, format!("Experiment saved: {}", path.display())),
            Err(e) => self.push_log(Level::Error, format!("Experiment not saved: {e}")),
        }
    }

    /// Loading never changes the output display and never sends a pattern.
    pub fn open_experiment(&mut self, path: &Path) {
        match ExperimentDoc::load(path) {
            Ok(doc) => {
                self.push_log(
                    Level::Info,
                    format!(
                        "Opened {} ({} items). Nothing was sent.",
                        path.display(),
                        doc.experiment.items.len()
                    ),
                );
                self.experiment = doc;
                self.set_selection(None);
                self.reveal_row = None;
            }
            Err(e) => self.push_log(Level::Error, format!("Open failed: {e}")),
        }
    }

    pub fn export_experiment(&mut self, dir: &Path) {
        let kinds: Vec<HeatmapKind> = HeatmapKind::ALL
            .into_iter()
            .filter(|k| self.heatmaps.contains(k))
            .collect();
        match self.experiment.export(dir, &kinds) {
            Ok(files) => self.push_log(
                Level::Info,
                format!("Exported {} files to {}", files.len(), dir.display()),
            ),
            Err(e) => self.push_log(Level::Error, format!("Export failed: {e}")),
        }
    }

    fn save_preview(&mut self, png: bool) {
        let Some((_, source, caption)) = self.current_preview() else {
            return;
        };
        let (name, extension) = if png {
            ("SLM code heatmap", "png")
        } else {
            ("Santec CSV", "csv")
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("pattern.{extension}"))
            .add_filter(name, &[extension])
            .save_file()
        else {
            return;
        };
        let result = source.save_to(&path, png);
        match result {
            Ok(()) => self.push_log(
                Level::Info,
                format!("Saved {caption} to {}", path.display()),
            ),
            Err(error) => self.push_log(
                Level::Error,
                format!("Save {} failed: {error}", path.display()),
            ),
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

    fn connect_block(&self) -> Option<String> {
        if self.wavelength_busy {
            return Some("Wait for the wavelength operation to finish".into());
        }
        if self.pending.is_some() {
            return Some("Waiting for the device".into());
        }
        let raster = self.config.profile.generation.raster;
        check_output(
            self.config.output.display.as_ref(),
            &self.displays,
            &OutputRequirements {
                width: raster.width,
                height: raster.height,
                mode: self.config.confirmed_mode(),
            },
            self.remote_session,
        )
        .err()
        .map(|reasons| slm_device::display::describe(&reasons))
    }

    fn apply_block(&self) -> Option<&'static str> {
        if self.pending.is_some() {
            Some("Applying… wait for the result")
        } else if matches!(self.device_state, DeviceState::Unknown { .. }) {
            Some("Output unknown: disconnect, reselect and reconnect")
        } else if !self.connected() {
            Some("Connect the device to apply")
        } else {
            None
        }
    }
}

impl App {
    /// What the Input Pattern area shows for the current tab: the key, the
    /// source and a caption naming the preview target.
    pub fn current_preview(&self) -> Option<(String, crate::preview::PreviewSource, String)> {
        use crate::preview::PreviewSource;
        let profile = self.config.profile.generation;
        let item_source = |i: usize| {
            let item = self.experiment.experiment.items.get(i)?.clone();
            let caption = format!("List row #{} · {}", i + 1, item.label);
            let asset = self.experiment.asset_path(&item);
            Some((PreviewSource::Item { item, asset }, caption))
        };
        let (source, caption) = match self.tab {
            Tab::Uniform => (
                PreviewSource::Spec {
                    spec: self.uniform_spec(),
                    profile,
                },
                "Uniform form".to_string(),
            ),
            Tab::Single => (
                PreviewSource::Spec {
                    spec: self.single_spec(),
                    profile,
                },
                "Single Wave form".to_string(),
            ),
            Tab::Multi => (
                PreviewSource::Spec {
                    spec: self.multi_spec(),
                    profile,
                },
                "Multi-Wave form".to_string(),
            ),
            Tab::Sweep if self.sweep_preview_row && self.selected.is_some() => {
                item_source(self.selected?)?
            }
            Tab::Sweep => {
                let spec = self.sweep_spec();
                let target = spec.target.label(&spec.base);
                let unit = spec.target.unit(&spec.base).unwrap_or_default();
                match spec.first_pattern().and_then(|(value, first)| {
                    spec.summary().map(|summary| (value, first, summary.count))
                }) {
                    Ok((value, first, count)) => (
                        PreviewSource::Spec {
                            spec: first,
                            profile,
                        },
                        format!(
                            "Sweep first value (1 of {}) · {} = {} {}",
                            count,
                            ui_text(&target),
                            format_number(value),
                            ui_text(&unit)
                        ),
                    ),
                    Err(_) => (
                        PreviewSource::Spec {
                            spec: spec.base,
                            profile,
                        },
                        "Sweep base (sweep values invalid)".to_string(),
                    ),
                }
            }
            Tab::Experiment => item_source(self.selected?)?,
        };
        Some((source.key(), source, caption))
    }

    /// Every sweep value validated against the profile; cached per draft.
    pub fn sweep_validation(&self) -> Result<(), String> {
        let spec = self.sweep_spec();
        let profile = self.config.profile.generation;
        let key = serde_json::to_string(&(&spec, &profile)).unwrap_or_default();
        if let Some((k, result)) = self.sweep_check.borrow().as_ref()
            && *k == key
        {
            return result.clone();
        }
        let result = spec.expand().and_then(|items| {
            items.iter().try_for_each(|(v, s)| {
                s.validate(&profile)
                    .map_err(|e| format!("At {}: {e}", format_number(*v)))
            })
        });
        *self.sweep_check.borrow_mut() = Some((key, result.clone()));
        result
    }

    pub fn preview_ready(&self) -> bool {
        match self.current_preview() {
            Some((key, _, _)) => self.preview.shown.as_deref() == Some(key.as_str()),
            None => true,
        }
    }

    /// Request a render when the input changed; accept only the result for
    /// the latest request. Independent of the device worker.
    fn process_previews(&mut self, ctx: &egui::Context) {
        if let Some((key, source, _)) = self.current_preview()
            && self.preview.requested.as_deref() != Some(key.as_str())
        {
            self.preview.requested = Some(key.clone());
            self.previewer.request(key, source);
        }
        for result in self.previewer.try_results() {
            if self.preview.requested.as_deref() != Some(result.key.as_str()) {
                continue;
            }
            match result.result {
                Ok(image) => {
                    let color = egui::ColorImage::from_rgb(
                        [image.width as usize, image.height as usize],
                        &image.rgb,
                    );
                    let options = egui::TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Linear,
                        wrap_mode: egui::TextureWrapMode::ClampToEdge,
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                    };
                    match &mut self.preview.texture {
                        Some(texture) => texture.set(color, options),
                        None => {
                            self.preview.texture =
                                Some(ctx.load_texture("input-pattern", color, options))
                        }
                    }
                    self.preview.info = Some(crate::preview::PreviewImage {
                        rgb: Vec::new(),
                        ..image
                    });
                    self.preview.shown = Some(result.key);
                    self.preview.error = None;
                }
                Err(e) => {
                    self.preview.shown = None;
                    self.preview.error = Some((result.key, e));
                }
            }
        }
    }

    /// Fit the startup window to the monitor it opened on, once. Explicit
    /// capture sizes are kept exactly.
    fn fit_window_once(&mut self, ctx: &egui::Context) {
        if self.window_fit.done || self.capture.is_some() {
            return;
        }
        self.window_fit.frames += 1;
        let (monitor, inner) = ctx.input(|i| {
            let v = i.viewport();
            (v.monitor_size, v.inner_rect.map(|r| r.size()))
        });
        match monitor {
            Some(monitor) if monitor.x > 0. && monitor.y > 0. => {
                let requested = inner.unwrap_or(egui::Vec2::new(1566., 900.));
                self.apply_window_fit(ctx, crate::layout::fit_window(monitor, requested));
            }
            // No monitor information (headless, some remote sessions): keep
            // the requested size and the normal minimum.
            _ if self.window_fit.frames > 30 => self.window_fit.done = true,
            _ => ctx.request_repaint(),
        }
    }

    pub fn apply_window_fit(&mut self, ctx: &egui::Context, fit: crate::layout::WindowFit) {
        self.window_fit.done = true;
        self.window_fit.limit = Some(fit.usable);
        self.right_collapsed |= fit.collapse_right;
        self.left_collapsed |= fit.collapse_left;
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(fit.min_inner));
        if fit.maximize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        self.push_log(
            Level::Info,
            if fit.maximize {
                format!(
                    "Window fitted to the screen ({:.0} × {:.0} points usable){}.",
                    fit.usable.x,
                    fit.usable.y,
                    match (fit.collapse_left, fit.collapse_right) {
                        (true, _) => "; side panels folded",
                        (false, true) => "; log panel folded",
                        _ => "",
                    }
                )
            } else {
                "Window fits the screen at 1566 × 900.".into()
            },
        );
    }

    /// Capture-only view selection: final tab and expanded cards.
    pub fn set_capture_view(&mut self, view: &str) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        capture.tab = Tab::ALL.into_iter().find(|t| {
            view.to_lowercase()
                .starts_with(&t.label().to_lowercase().replace(' ', "-"))
        });
        if view.split_whitespace().any(|w| w == "expanded") {
            self.device_expanded = true;
        }
        if view.split_whitespace().any(|w| w == "left-collapsed") {
            self.left_collapsed = true;
        }
        if view.split_whitespace().any(|w| w == "right-collapsed") {
            self.right_collapsed = true;
        }
        if view.split_whitespace().any(|w| w == "settings") {
            self.settings_open = true;
        }
    }

    /// Launch check driven frame by frame (demo only).
    fn capture_step(&mut self, ctx: &egui::Context) {
        let Some(mut capture) = self.capture.take() else {
            return;
        };
        capture.frames += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
        match capture.stage {
            0 if !self.displays.is_empty() && capture.frames > 5 => {
                self.connect();
                capture.stage = 1;
            }
            1 if self.connected() => {
                self.tab = Tab::Single;
                self.single.geometry.rotation.value = 30.;
                let spec = self.single_spec();
                self.apply_spec("Single Wave".into(), spec);
                capture.stage = 2;
            }
            2 if matches!(self.status, ApplyStatus::Applied(..)) => {
                let spec = self.single_spec();
                self.add_to_experiment("Single Wave 30°".into(), spec);
                self.sweep.base = SweepBase::Uniform;
                self.sweep.target = SweepTarget::UniformCode;
                self.sweep.start = 0.;
                self.sweep.end = 1023.;
                self.sweep.step = 100.;
                self.sweep.include_end = true;
                self.add_sweep();
                self.apply_experiment_item(0);
                capture.stage = 10;
            }
            10 if self.pending.is_none() => {
                if let Some(index) = self.adjacent_row(true) {
                    self.apply_experiment_item(index);
                }
                capture.stage = 11;
            }
            11 if self.pending.is_none() => {
                self.set_selection(self.showing_index());
                self.sweep_preview_row = false;
                if let Some(tab) = capture.tab {
                    self.tab = tab;
                }
                capture.frames = 0;
                capture.stage = 3;
            }
            3 if capture.frames > 8 && self.preview_ready() => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                capture.stage = 4;
            }
            4 => {
                let image = ctx.input(|i| {
                    i.raw.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = image {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    capture.result = Some(
                        image::RgbaImage::from_raw(
                            image.size[0] as u32,
                            image.size[1] as u32,
                            rgba,
                        )
                        .ok_or_else(|| "Screenshot size mismatch".to_string())
                        .and_then(|img| img.save(&capture.path).map_err(|e| e.to_string())),
                    );
                    match &capture.result {
                        Some(Ok(())) => eprintln!("Captured {}", capture.path.display()),
                        Some(Err(e)) => eprintln!("Capture failed: {e}"),
                        None => {}
                    }
                    capture.stage = 5;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            _ if capture.frames > 600 && capture.stage < 5 => {
                eprintln!("Capture timed out at stage {}", capture.stage);
                capture.stage = 5;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
        self.capture = Some(capture);
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
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(root);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}
