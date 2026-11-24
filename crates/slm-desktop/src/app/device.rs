//! Device workflow: wavelength and display requests, worker events, and the
//! connect / disconnect / reset / apply actions with their block reasons.

use super::{App, AppliedRecord, ApplyStatus, Level};
use crate::theme::{AMBER, ERROR, TEAL};
use slm_core::PatternSpec;
use slm_device::backend::BackendKind;
use slm_device::display::{OutputRequirements, check_output};
use slm_device::worker::DeviceState;
use slm_device::{ApplyItem, Command, Event, FrameSource};

impl App {
    /// Device, display, mode and profile edits are locked: output is open,
    /// a wavelength operation runs, or external control holds the device.
    pub(super) fn device_busy(&self) -> bool {
        self.wavelength_busy
            || !matches!(self.device_state, DeviceState::Disconnected)
            || self.lease_block().is_some()
    }

    pub(super) fn request_wavelength(&mut self, write: bool) {
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

    pub(super) fn wavelength_target(&self) -> Option<u32> {
        self.wavelength_nm
            .trim()
            .parse()
            .ok()
            .filter(|nm| slm_device::wavelength::SANTEC_WAVELENGTH_NM.contains(nm))
    }

    pub(super) fn wavelength_device_number(&self) -> Option<u32> {
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
                        self.output_unverified = Some(reason.clone());
                    }
                    match &state {
                        // Disconnect ends here (also when it was a no-op
                        // after a refused connect).
                        DeviceState::Disconnected => self.disconnecting = false,
                        // Connect and apply emit `Connected` only after the
                        // backend confirmed the frame: the one positive event
                        // that clears an unknown output.
                        DeviceState::Connected { .. } => self.output_unverified = None,
                        _ => {}
                    }
                    self.device_state = state;
                }
                Event::Applied {
                    request,
                    label,
                    receipt,
                    ..
                } => {
                    self.finish_apply_job(request, Ok(&receipt));
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
                    if !output_known && self.output_unverified.is_none() {
                        self.output_unverified = Some(error.clone());
                    }
                    self.finish_apply_job(request, Err((error.as_str(), output_known)));
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
                Event::ConnectDone { request, result } => {
                    self.finish_connect_job(request, &result);
                }
                Event::DisconnectDone { request, report } => {
                    use slm_device::worker::ResetOutcome;
                    // A reset that was not confirmed before closing leaves
                    // the output unverified, exactly like a failed draw.
                    let unverified = match (&report.reset, &report.close_error) {
                        (ResetOutcome::Failed(e), _) => {
                            Some(format!("Reset before closing failed: {e}"))
                        }
                        (_, Some(e)) => Some(format!("Closing the output failed: {e}")),
                        _ => None,
                    };
                    if let Some(reason) = unverified
                        && self.output_unverified.is_none()
                    {
                        self.push_log(Level::Error, format!("Output unknown: {reason}"));
                        self.output_unverified = Some(reason);
                    }
                    self.disconnecting = false;
                    self.finish_close_job(request, &report);
                }
            }
        }
    }

    // ----- actions --------------------------------------------------------

    /// Manual mutations are refused while external control holds the lease
    /// (or its operation/cleanup is still pending). Stop and Take control
    /// in the External control row stay available.
    fn manual_refused(&mut self) -> bool {
        match self.lease_block() {
            Some(reason) => {
                self.push_log(Level::Warn, format!("Not done: {reason}."));
                true
            }
            None => false,
        }
    }

    pub fn connect(&mut self) {
        if !self.manual_refused() {
            self.send_connect();
        }
    }

    /// Connect to the GUI's selected display with the confirmed input mode
    /// (the worker re-checks both before anything is drawn). Returns the
    /// worker request acknowledged by `ConnectDone`.
    pub(super) fn send_connect(&mut self) -> Option<u64> {
        let Some(display) = self.config.output.display.clone() else {
            self.push_log(Level::Warn, "Select an output display first.");
            return None;
        };
        self.push_log(Level::Info, format!("Connecting to {}…", display.label()));
        let request = self.worker.request_id();
        self.worker
            .send(Command::Connect {
                request,
                backend: self.config.output.backend,
                selection: Some(display),
                mode: self.config.confirmed_mode(),
                profile: self.config.profile.clone(),
            })
            .ok()?;
        self.device_state = DeviceState::Connecting;
        Some(request)
    }

    pub fn disconnect(&mut self) {
        if !self.manual_refused() {
            self.send_disconnect();
        }
    }

    /// The worker's reset-then-close path; acknowledged by `DisconnectDone`.
    pub(super) fn send_disconnect(&mut self) -> Option<u64> {
        self.queued_row = None;
        let request = self.worker.request_id();
        // A disconnect from `Disconnected` emits no state event, so only an
        // open (or opening/unknown) output is tracked as disconnecting.
        let sent = self.worker.send(Command::Disconnect { request }).is_ok();
        if sent {
            self.disconnecting |= !matches!(self.device_state, DeviceState::Disconnected);
        }
        self.push_log(
            Level::Info,
            "Disconnect requested: reset pattern first, then close.",
        );
        sent.then_some(request)
    }

    pub fn reset_pattern(&mut self) {
        if !self.manual_refused() {
            self.send_reset();
        }
    }

    /// Show the uniform reset code; acknowledged by `Applied`/`ApplyFailed`.
    pub(super) fn send_reset(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
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
        if self.worker.send(Command::Reset { request }).is_err() {
            self.pending = None;
            return None;
        }
        Some(request)
    }

    pub fn apply_spec(&mut self, label: String, spec: PatternSpec) {
        if !self.manual_refused() {
            self.send_spec(label, spec);
        }
    }

    pub(super) fn send_spec(&mut self, label: String, spec: PatternSpec) -> Option<u64> {
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
        )
    }

    /// Returns the worker request acknowledged by `Applied`/`ApplyFailed`.
    fn send_apply(&mut self, record: AppliedRecord, item: ApplyItem) -> Option<u64> {
        if self.pending.is_some() {
            self.push_log(
                Level::Warn,
                "A pattern is still being applied; wait for the result.",
            );
            return None;
        }
        self.queued_row = None;
        let request = self.worker.request_id();
        self.status = ApplyStatus::Applying(record.clone());
        self.pending = Some((request, record));
        if self.worker.send(Command::Apply { request, item }).is_err() {
            self.pending = None;
            return None;
        }
        Some(request)
    }

    pub fn apply_experiment_item(&mut self, index: usize) {
        if self.apply_block().is_some() {
            return;
        }
        self.send_experiment_item(index);
    }

    pub(super) fn send_experiment_item(&mut self, index: usize) -> Option<u64> {
        let item = self.experiment.experiment.items.get(index).cloned()?;
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
        )
    }

    pub(super) fn connect_block(&self) -> Option<String> {
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

    /// Why the manual Apply controls are disabled.
    pub(super) fn apply_block(&self) -> Option<&'static str> {
        self.lease_block().or_else(|| self.device_apply_block())
    }

    /// Why no pattern can be sent at all (manual or external).
    pub(super) fn device_apply_block(&self) -> Option<&'static str> {
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
