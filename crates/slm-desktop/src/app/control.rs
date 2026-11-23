//! External control on the GUI thread: dispatch of queued API requests,
//! the writer lease, and job completion from worker acknowledgements.
//!
//! Every mutation uses the same `send_*` paths as the buttons, so the
//! display interlock (`connect_block`, the worker's re-enumeration before
//! every draw), the confirmed input mode and the worker reset/close path are
//! shared. Nothing here selects a display, confirms a mode or edits the
//! profile. Operations are serial: one at a time, each answered at once with
//! a job receipt that becomes terminal only on the worker's acknowledgement.

use super::*;
use crate::control_api::{self, RESET_MEANING, receipt_json};
use crate::control_lease::{
    HEARTBEAT_SECONDS, Job, JobKind, JobState, LEASE_TIMEOUT, Origin, Refusal, Revocation, Seen,
};
use kohdalab_control_protocol::{is_common, prepare, response as common_response};
use serde_json::{Map, Value, json};
use slm_device::worker::{ConnectFailure, DisconnectReport, ResetOutcome};
use std::time::Instant;

/// Fields added by the transport, the SDK or the envelope helpers; never
/// operation parameters.
const ENVELOPE: [&str; 11] = [
    "token",
    "method",
    "action",
    "control_protocol",
    "_control_method",
    "client",
    "client_id",
    "request_id",
    "lease",
    "run_id",
    "id",
];
const MAX_LABEL: usize = 80;
const MAX_COMPONENTS: usize = 64;

type Answer = Result<Value, Refusal>;

fn native_answer(answer: Answer) -> Value {
    match answer {
        Ok(result) => json!({"ok": true, "result": result}),
        Err((code, message)) => control_api::error(code, &message),
    }
}

fn params(native: &Value) -> Map<String, Value> {
    native
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| !ENVELOPE.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Strict parameters: anything not listed is refused, so a typo can never
/// silently fall back to a default.
fn only(native: &Value, allowed: &[&str]) -> Result<(), Refusal> {
    match params(native)
        .keys()
        .find(|k| !allowed.contains(&k.as_str()))
    {
        Some(k) => Err(("validation", format!("Unknown parameter {k:?}"))),
        None => Ok(()),
    }
}

fn home_redact(text: &str) -> String {
    control_api::redact(text, slm_device::config::home_dir().as_deref())
}

fn label_param(value: &Value) -> Result<String, Refusal> {
    let text = value.as_str().map(str::trim).unwrap_or_default();
    if text.is_empty() || text.chars().count() > MAX_LABEL || text.chars().any(char::is_control) {
        return Err((
            "validation",
            format!("label must be 1–{MAX_LABEL} printable characters"),
        ));
    }
    Ok(text.to_string())
}

/// First input key that the typed spec does not know (`spec.wave.perod`).
fn unknown_field(input: &Value, canonical: &Value, path: &str) -> Option<String> {
    match (input, canonical) {
        (Value::Object(a), Value::Object(b)) => a.iter().find_map(|(k, v)| match b.get(k) {
            None => Some(format!("{path}.{k}")),
            Some(c) => unknown_field(v, c, &format!("{path}.{k}")),
        }),
        (Value::Array(a), Value::Array(b)) => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| unknown_field(x, y, &format!("{path}[{i}]"))),
        _ => None,
    }
}

/// A `PatternSpec` in its saved JSON form, with no unknown fields, valid for
/// the GUI's current generation profile.
pub(crate) fn parse_spec(
    value: &Value,
    profile: &slm_core::GenerationProfile,
) -> Result<PatternSpec, String> {
    let spec: PatternSpec =
        serde_json::from_value(value.clone()).map_err(|e| format!("Invalid PatternSpec: {e}"))?;
    let canonical = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
    if let Some(path) = unknown_field(value, &canonical, "spec") {
        return Err(format!("Unknown PatternSpec field {path}"));
    }
    if spec.components().len() > MAX_COMPONENTS {
        return Err(format!("At most {MAX_COMPONENTS} Multi-Wave components"));
    }
    spec.validate(profile)?;
    Ok(spec)
}

impl App {
    /// Answer queued requests, then enforce lease expiry. Called once per
    /// frame after worker events, so answers reflect this frame's state.
    pub fn process_control_api(&mut self) {
        // A listener that ended (permanent accept error) has already withdrawn
        // its descriptor; drop it so no active indicator remains. No client
        // can reach the app any more: that is transport loss for a lease.
        if self
            .control_api
            .as_ref()
            .is_some_and(|api| !api.is_active())
        {
            self.control_api = None;
            self.push_log(
                Level::Warn,
                "External control stopped: GUI API listener ended.",
            );
            if self.control.lease.is_some() {
                self.push_log(
                    Level::Warn,
                    "External control lease ended with the listener; stopping output.",
                );
                self.end_lease(Revocation::ListenerEnded, Origin::ListenerEnded);
            }
            return;
        }
        let requests: Vec<_> = self
            .control_api
            .as_ref()
            .map(|api| api.requests.try_iter().take(8).collect())
            .unwrap_or_default();
        for request in requests {
            let value = if request.deadline < Instant::now() {
                control_api::expired(&request.body)
            } else {
                self.answer_control(&request.body)
            };
            let _ = request.reply.send(value);
        }
        if self.control.expired(Instant::now()) {
            self.push_log(
                Level::Warn,
                format!(
                    "External control lease expired (no heartbeat for {} s); stopping output.",
                    LEASE_TIMEOUT.as_secs()
                ),
            );
            self.end_lease(Revocation::Expired, Origin::LeaseExpired);
        }
    }

    /// One authenticated request, answered in the common envelope.
    pub(crate) fn answer_control(&mut self, body: &Value) -> Value {
        let native = match prepare(body) {
            Ok(native) => native,
            Err(error) => return error,
        };
        if !is_common(body) {
            return control_api::unsupported();
        }
        let method = native
            .get("_control_method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let answer = match method.as_str() {
            "describe" => native_answer(Ok(control_api::describe())),
            "status" => native_answer(Ok(control_api::status(&self.control_snapshot()))),
            "config" => native_answer(Ok(control_api::config(&self.control_snapshot()))),
            "result" => native_answer(self.control_result(&native)),
            m if control_api::MUTATIONS.contains(&m) => self.control_mutation(m, &native),
            _ => control_api::unsupported(),
        };
        common_response(body, answer)
    }

    /// Read-only; any authenticated client may follow a job (the SDK follows
    /// a release job after it has dropped the lease).
    fn control_result(&self, native: &Value) -> Answer {
        let id = native
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or(("validation", "control.result needs job_id".to_string()))?;
        self.control.job(id).map(Job::receipt).ok_or((
            "unknown_job",
            "No such SLM job (receipts of the latest 64 jobs are kept)".into(),
        ))
    }

    fn control_mutation(&mut self, method: &str, native: &Value) -> Value {
        let request_id = match native.get("request_id").and_then(Value::as_str) {
            Some(id) if uuid::Uuid::parse_str(id).is_ok() => id.to_string(),
            _ => {
                return native_answer(Err((
                    "validation",
                    format!("control.{method} needs a UUID request_id"),
                )));
            }
        };
        let client = match native.get("client_id").and_then(Value::as_str) {
            Some(c) if !c.is_empty() && c.len() <= 128 => c.to_string(),
            _ => {
                return native_answer(Err((
                    "validation",
                    format!("control.{method} needs a client_id"),
                )));
            }
        };
        let fingerprint = format!("{method}\n{client}\n{}", Value::Object(params(native)));
        // Heartbeats are idempotent and frequent; they are not recorded.
        let record = method != "heartbeat";
        if record {
            match self.control.lookup(&request_id, &fingerprint) {
                // The original answer, even if the lease has ended since:
                // a client that lost the reply learns what was accepted.
                Seen::Duplicate(mut answer) => {
                    if let Some(result) = answer.get_mut("result").and_then(Value::as_object_mut) {
                        result.insert("duplicate".into(), json!(true));
                    }
                    return answer;
                }
                Seen::Conflict => {
                    return native_answer(Err((
                        "request_id_conflict",
                        "This request_id was already used for a different request; nothing was executed".into(),
                    )));
                }
                Seen::New => {}
            }
        }
        let lease = native.get("lease").and_then(Value::as_str);
        let answer = if method == "acquire" {
            self.api_acquire(&client, lease, native)
        } else {
            self.control
                .check_lease(&client, lease)
                .and_then(|()| match method {
                    "heartbeat" => self.api_heartbeat(native),
                    "release" => self.api_release(native),
                    "connect" => self.api_connect(native),
                    "apply" => self.api_apply(native),
                    "reset" => self.api_reset(native),
                    "disconnect" => self.api_disconnect(native),
                    "stop" => self.api_stop(native),
                    _ => Err(("unsupported", format!("control.{method}"))),
                })
        };
        let value = native_answer(answer);
        if record && value["ok"] == true {
            self.control
                .remember(&request_id, fingerprint, value.clone());
        }
        value
    }

    // ----- lease ----------------------------------------------------------

    fn lease_answer(&self) -> Value {
        json!({
            "lease": self.control.lease.as_ref().map(|l| l.token.clone()),
            "heartbeat_seconds": HEARTBEAT_SECONDS,
            "lease_timeout_seconds": LEASE_TIMEOUT.as_secs_f64(),
            "on_release": "stop",
            "on_expiry": "stop",
            "stop_requires_lease": true,
        })
    }

    fn api_acquire(&mut self, client: &str, lease: Option<&str>, native: &Value) -> Answer {
        only(native, &[])?;
        if let Some(active) = &self.control.lease {
            // The holder may re-acquire its own lease (renewal).
            if active.client_id == client && lease == Some(active.token.as_str()) {
                self.control.renew(Instant::now());
                return Ok(self.lease_answer());
            }
            return Err((
                "ownership",
                "Another client holds the SLM lease; wait for its release, or use Stop or Take control in the GUI".into(),
            ));
        }
        if let Some(refusal) = self.takeover_block() {
            return Err(refusal);
        }
        self.control.grant(client, Instant::now());
        self.queued_row = None;
        self.push_log(
            Level::Info,
            "External control: a client acquired the lease. Manual output controls are locked; Stop and Take control stay available.",
        );
        Ok(self.lease_answer())
    }

    /// A new lease is refused while the previous one's work or cleanup is
    /// unfinished, while the GUI is operating the device, and while the
    /// output is unverified (only a backend-confirmed GUI Connect clears it).
    fn takeover_block(&self) -> Option<Refusal> {
        if let Some(job) = self.control.running() {
            return Some((
                "busy",
                format!(
                    "External {} {} is still waiting for the device; acquire after its receipt is terminal",
                    job.kind.name(),
                    job.id
                ),
            ));
        }
        if self.gui_device_busy() {
            return Some((
                "busy",
                "The GUI is operating the device; try again when it is idle".into(),
            ));
        }
        let reason = match &self.device_state {
            DeviceState::Unknown { reason } => Some(reason.clone()),
            _ => self.output_unverified.clone(),
        }?;
        Some((
            "cleanup_unconfirmed",
            format!(
                "SLM output is unverified ({}); in the GUI, Connect (backend-confirmed reset) and Disconnect before external control",
                home_redact(&reason)
            ),
        ))
    }

    fn gui_device_busy(&self) -> bool {
        self.pending.is_some()
            || self.disconnecting
            || self.wavelength_busy
            || matches!(self.device_state, DeviceState::Connecting)
    }

    fn api_heartbeat(&mut self, native: &Value) -> Answer {
        only(native, &[])?;
        self.control.renew(Instant::now());
        Ok(json!({"renewed": true, "lease_timeout_seconds": LEASE_TIMEOUT.as_secs_f64()}))
    }

    /// Release always stops: reset, then close, through the worker. The
    /// lease ends at once; the stop job's receipt reports the cleanup.
    fn api_release(&mut self, native: &Value) -> Answer {
        only(native, &["reason"])?;
        if let Some(reason) = native.get("reason")
            && !reason
                .as_str()
                .is_some_and(|r| !r.is_empty() && r.len() <= 64)
        {
            return Err(("validation", "reason must be a short string".into()));
        }
        self.push_log(
            Level::Info,
            "External control released; stopping output (reset, then close).",
        );
        let id = self.end_lease(Revocation::Released, Origin::Release);
        let job = self.control.job(&id).map(Job::receipt);
        let cleanup = job
            .as_ref()
            .map_or(json!("unknown"), |j| j["cleanup"].clone());
        Ok(json!({
            "released": true,
            "on_release": "stop",
            "job_id": id,
            "cleanup": cleanup,
            "job": job,
        }))
    }

    /// Revoke the lease and stop the output (release, expiry, listener end).
    fn end_lease(&mut self, why: Revocation, origin: Origin) -> String {
        self.control.revoke(why);
        self.start_close(JobKind::Stop, origin)
    }

    /// The worker's reset-then-close path, at most one at a time. Queued
    /// behind an operation already sent, never instead of it.
    fn start_close(&mut self, kind: JobKind, origin: Origin) -> String {
        if let Some(job) = self.control.running_close() {
            return job.id.clone();
        }
        let reset_code = self.config.profile.reset_code;
        let request = self.send_disconnect();
        let id = self
            .control
            .start(
                kind,
                origin,
                request.unwrap_or(0),
                json!({"reset_code": reset_code, "reset_meaning": RESET_MEANING}),
            )
            .id
            .clone();
        if request.is_none() {
            self.control.finish(
                0,
                &[kind],
                JobState::Error,
                "unknown",
                json!({"error": "Device worker stopped; output state unknown"}),
            );
        }
        id
    }

    // ----- operations -----------------------------------------------------

    fn api_serial(&self) -> Result<(), Refusal> {
        if let Some(job) = self.control.running() {
            return Err((
                "busy",
                format!(
                    "{} {} is still running; wait for its receipt",
                    job.kind.name(),
                    job.id
                ),
            ));
        }
        if self.gui_device_busy() {
            return Err((
                "busy",
                "The device is busy; wait for the current operation".into(),
            ));
        }
        Ok(())
    }

    fn api_connect(&mut self, native: &Value) -> Answer {
        only(native, &[])?;
        self.api_serial()?;
        if let Some(reason) = &self.output_unverified {
            return Err((
                "cleanup_unconfirmed",
                format!(
                    "Output is unverified ({}); reconnect in the GUI first",
                    home_redact(reason)
                ),
            ));
        }
        match &self.device_state {
            DeviceState::Disconnected => {}
            DeviceState::Unknown { .. } => {
                return Err((
                    "cleanup_unconfirmed",
                    "Output state is unknown; disconnect first".into(),
                ));
            }
            _ => return Err(("state", "Already connected; disconnect first".into())),
        }
        let Some(display) = self.config.output.display.clone() else {
            return Err((
                "safety",
                "No output display is selected in the GUI; the API cannot choose one".into(),
            ));
        };
        if let Some(block) = self.connect_block() {
            return Err((
                "safety",
                format!(
                    "Display interlock, nothing was sent: {}",
                    home_redact(&block)
                ),
            ));
        }
        let mode = self.config.confirmed_mode();
        let reset_code = self.config.profile.reset_code;
        let request = self
            .send_connect()
            .ok_or(("device", "Device worker stopped".to_string()))?;
        self.push_log(Level::Info, "External control: connect requested.");
        let job = self.control.start(
            JobKind::Connect,
            Origin::Request,
            request,
            json!({
                "display": {"friendly_name": display.friendly_name, "gdi_name": display.gdi_name},
                "mode": mode,
                "reset_code": reset_code,
                "reset_meaning": RESET_MEANING,
            }),
        );
        Ok(job.receipt())
    }

    fn api_apply(&mut self, native: &Value) -> Answer {
        only(native, &["spec", "item_id", "label"])?;
        self.api_serial()?;
        if let Some(block) = self.device_apply_block() {
            let code = if matches!(self.device_state, DeviceState::Unknown { .. }) {
                "cleanup_unconfirmed"
            } else {
                "not_connected"
            };
            return Err((code, format!("{block}; nothing was sent")));
        }
        let label = native.get("label").map(label_param).transpose()?;
        let (request, details) = match (native.get("spec"), native.get("item_id")) {
            (Some(spec), None) => {
                let profile = self.config.profile.generation;
                let spec = parse_spec(spec, &profile).map_err(|e| ("validation", e))?;
                let label = label.unwrap_or_else(|| format!("Python · {}", spec.kind_label()));
                let details = json!({
                    "source": "spec",
                    "label": label,
                    "pattern_kind": spec.kind_label(),
                    "summary": spec.summary(&profile.optics),
                    "spec": spec,
                });
                (self.send_spec(label, spec), details)
            }
            (None, Some(id)) => {
                if label.is_some() {
                    return Err((
                        "validation",
                        "label applies only to spec; a row keeps its own label".into(),
                    ));
                }
                let id = id.as_str().unwrap_or_default();
                let Some(index) = self
                    .experiment
                    .experiment
                    .items
                    .iter()
                    .position(|i| i.id == id)
                else {
                    return Err(("validation", "No Patterns row has this item_id".into()));
                };
                let item = &self.experiment.experiment.items[index];
                let details = json!({
                    "source": "item",
                    "item_id": item.id,
                    "row": index + 1,
                    "label": home_redact(&item.label),
                    "summary": item.summary(),
                });
                (self.send_experiment_item(index), details)
            }
            _ => {
                return Err((
                    "validation",
                    "Give exactly one of spec (a PatternSpec) or item_id (a Patterns row)".into(),
                ));
            }
        };
        let request = request.ok_or(("busy", "Not sent: the device is busy".to_string()))?;
        self.push_log(Level::Info, "External control: apply requested.");
        Ok(self
            .control
            .start(JobKind::Apply, Origin::Request, request, details)
            .receipt())
    }

    fn api_reset(&mut self, native: &Value) -> Answer {
        only(native, &[])?;
        self.api_serial()?;
        if let Some(block) = self.device_apply_block() {
            return Err(("not_connected", format!("{block}; nothing was sent")));
        }
        let reset_code = self.config.profile.reset_code;
        let request = self
            .send_reset()
            .ok_or(("busy", "Not sent: the device is busy".to_string()))?;
        self.push_log(Level::Info, "External control: reset pattern requested.");
        Ok(self
            .control
            .start(
                JobKind::Reset,
                Origin::Request,
                request,
                json!({"reset_code": reset_code, "reset_meaning": RESET_MEANING}),
            )
            .receipt())
    }

    fn api_disconnect(&mut self, native: &Value) -> Answer {
        only(native, &[])?;
        self.api_serial()?;
        let id = self.start_close(JobKind::Disconnect, Origin::Request);
        Ok(self.control.job(&id).map(Job::receipt).unwrap_or_default())
    }

    /// Stop is accepted while another operation runs (it is queued behind
    /// it in the worker) and keeps the lease; Release ends it.
    fn api_stop(&mut self, native: &Value) -> Answer {
        only(native, &[])?;
        self.push_log(
            Level::Info,
            "External control: stop requested (reset, then close).",
        );
        let id = self.start_close(JobKind::Stop, Origin::Request);
        Ok(self.control.job(&id).map(Job::receipt).unwrap_or_default())
    }

    // ----- GUI ------------------------------------------------------------

    /// Why manual device controls are locked, if they are.
    pub(crate) fn lease_block(&self) -> Option<&'static str> {
        if self.control.lease.is_some() {
            Some("External control holds the SLM; use Stop or Take control")
        } else if self.control.running().is_some() {
            Some("Waiting for the device to acknowledge the external operation")
        } else {
            None
        }
    }

    /// Manual Stop: always available. Ends any lease, then reset and close.
    pub fn gui_stop(&mut self) {
        let revoked = self.control.revoke(Revocation::GuiStop);
        self.push_log(
            Level::Warn,
            if revoked {
                "Stop: external control lease revoked; output reset, then closed."
            } else {
                "Stop: output reset, then closed."
            },
        );
        self.start_close(JobKind::Stop, Origin::GuiStop);
    }

    /// Take control: end the lease and leave the output as it is. An
    /// operation already sent still finishes and gets its receipt.
    pub fn gui_take_control(&mut self) {
        if self.control.revoke(Revocation::GuiTakeControl) {
            self.push_log(
                Level::Warn,
                "Take control: external control lease revoked. The output is unchanged.",
            );
        }
    }

    // ----- worker acknowledgements ---------------------------------------

    pub(super) fn finish_apply_job(
        &mut self,
        request: u64,
        result: Result<&slm_device::ShowReceipt, (&str, bool)>,
    ) {
        let (state, cleanup, native) = match result {
            Ok(receipt) => (
                JobState::Completed,
                "not_required",
                json!({"receipt": receipt_json(receipt)}),
            ),
            Err((error, known)) => (
                JobState::Error,
                if known { "not_required" } else { "unknown" },
                json!({"error": home_redact(error), "output_known": known}),
            ),
        };
        self.control.finish(
            request,
            &[JobKind::Apply, JobKind::Reset],
            state,
            cleanup,
            native,
        );
    }

    pub(super) fn finish_connect_job(
        &mut self,
        request: u64,
        result: &Result<slm_device::ShowReceipt, ConnectFailure>,
    ) {
        let (state, cleanup, native) = match result {
            Ok(receipt) => (
                JobState::Completed,
                "not_required",
                json!({"receipt": receipt_json(receipt)}),
            ),
            Err(f) => (
                JobState::Error,
                if f.output_known {
                    "not_required"
                } else {
                    "unknown"
                },
                json!({"error": home_redact(&f.error), "output_known": f.output_known}),
            ),
        };
        self.control
            .finish(request, &[JobKind::Connect], state, cleanup, native);
    }

    /// `verified` means only: the backend confirmed the uniform reset frame
    /// and the output window closed. It is not an optical measurement.
    pub(super) fn finish_close_job(&mut self, request: u64, report: &DisconnectReport) {
        let nothing_open = report.reset == ResetOutcome::NotOpen { was_unknown: false };
        let (state, cleanup) = if self.output_unverified.is_some() {
            (JobState::Error, "unknown")
        } else if report.reset_confirmed() {
            (JobState::Completed, "verified")
        } else if nothing_open && report.close_error.is_none() {
            (JobState::Completed, "not_required")
        } else {
            (JobState::Error, "unknown")
        };
        let reset = match &report.reset {
            ResetOutcome::Confirmed(r) => {
                json!({"outcome": "confirmed", "receipt": receipt_json(r)})
            }
            ResetOutcome::Failed(e) => json!({"outcome": "failed", "error": home_redact(e)}),
            ResetOutcome::SkippedUnknown => json!({"outcome": "skipped_output_unknown"}),
            ResetOutcome::NotOpen { was_unknown } => {
                json!({"outcome": "not_open", "previous_output_unknown": was_unknown})
            }
        };
        self.control.finish(
            request,
            &[JobKind::Disconnect, JobKind::Stop],
            state,
            cleanup,
            json!({
                "summary": home_redact(&report.summary),
                "reset": reset,
                "reset_code": report.reset_code,
                "output_closed": report.close_error.is_none(),
                "close_error": report.close_error.as_deref().map(home_redact),
                "optical_state": "not_verified",
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_parsing_is_strict_and_validated() {
        let profile = slm_device::DeviceProfile::santec_slm200_candidate().generation;
        let ok = json!({"kind": "uniform", "code": 512});
        assert_eq!(
            parse_spec(&ok, &profile).unwrap(),
            PatternSpec::Uniform { code: 512 }
        );
        let single = json!({
            "kind": "single_wave",
            "wave": {"period": {"value": 100.0, "unit": "px"}, "waveform": "ramp",
                     "initial_phase": {"value": 0.0, "unit": "deg"}},
            "geometry": {"center_offset_px": [-16, 110], "rotation": {"value": 15.0, "unit": "deg"}},
        });
        assert!(parse_spec(&single, &profile).is_ok());
        for (bad, why) in [
            (json!({"kind": "uniform", "code": 5000}), "outside"),
            (
                json!({"kind": "uniform", "code": 1, "colour": 2}),
                "spec.colour",
            ),
            (
                json!({"kind": "raw_frame", "code": 1}),
                "Invalid PatternSpec",
            ),
            (json!({"kind": "uniform"}), "Invalid PatternSpec"),
            (
                json!({"kind": "uniform", "code": -1}),
                "Invalid PatternSpec",
            ),
            (json!("uniform"), "Invalid PatternSpec"),
        ] {
            let e = parse_spec(&bad, &profile).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
        let mut typo = single.clone();
        typo["wave"]["perod"] = json!(1);
        assert!(
            parse_spec(&typo, &profile)
                .unwrap_err()
                .contains("spec.wave.perod")
        );
    }

    #[test]
    fn parameters_are_strict_and_labels_bounded() {
        let native = json!({"token": "t", "client_id": "c", "request_id": "r", "spec": 1});
        assert!(only(&native, &["spec"]).is_ok());
        assert_eq!(only(&native, &[]).unwrap_err().0, "validation");
        assert!(label_param(&json!("Run 1")).is_ok());
        for bad in [json!(""), json!("a\nb"), json!(3), json!("x".repeat(81))] {
            assert!(label_param(&bad).is_err(), "{bad}");
        }
    }
}
