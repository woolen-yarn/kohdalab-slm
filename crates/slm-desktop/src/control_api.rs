//! Opt-in (`--gui-api`), loopback-only GUI control adapter.
//!
//! Same transport as KohdaLab Camera: one authenticated JSON Lines request per
//! connection, queued to the GUI. The server thread never touches the App or
//! the device worker. The GUI thread answers every request (`app/control.rs`):
//! reads from a [`Snapshot`] of state it already holds for that frame, and
//! mutations only under a writer lease (`control_lease.rs`), through the same
//! worker commands, display interlock and reset/close path as the buttons.
use kohdalab_control_protocol::{descriptor, response as common_response};
use serde_json::{Value, json};
use slm_device::AppConfig;
use slm_device::backend::{BackendKind, ShowReceipt};
use slm_device::worker::DeviceState;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const APPLICATION: &str = "kohdalab-slm";
pub const API_VERSION: u64 = 1;
pub const METHODS: [&str; 12] = [
    "control.describe",
    "control.status",
    "control.config",
    "control.result",
    "control.acquire",
    "control.heartbeat",
    "control.release",
    "control.connect",
    "control.apply",
    "control.reset",
    "control.disconnect",
    "control.stop",
];
/// Methods that need a UUID `request_id`; all but `acquire` need the lease.
pub const MUTATIONS: [&str; 8] = [
    "acquire",
    "heartbeat",
    "release",
    "connect",
    "apply",
    "reset",
    "disconnect",
    "stop",
];
/// What the reset frame is, stated wherever a reset is reported.
pub const RESET_MEANING: &str = "uniform raw device code profile.reset_code (provisional 0); backend-confirmed draw only, not an optical measurement and not verified dark";
const LIMIT: usize = 64 * 1024;
const PEERS: usize = 4;
const QUEUE: usize = 16;
const TIMEOUT: Duration = Duration::from_secs(2);

/// `~/.kohdalab/slm/run`. SLM is not in the shared crate's application list,
/// so the app owns this path (same layout as the other suite apps).
pub fn default_run_dir() -> Option<PathBuf> {
    Some(
        slm_device::config::home_dir()?
            .join(".kohdalab")
            .join("slm")
            .join("run"),
    )
}

pub struct Request {
    pub body: Value,
    pub deadline: Instant,
    pub reply: mpsc::SyncSender<Value>,
}

pub struct Server {
    pub requests: mpsc::Receiver<Request>,
    pub descriptor: PathBuf,
    stopped: Arc<AtomicBool>,
    /// Cleared when the listener thread ends for any reason (including a
    /// permanent accept error or a panic); the GUI then drops the server.
    alive: Arc<AtomicBool>,
}

impl Server {
    pub fn start(
        directory: Option<PathBuf>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let dir = directory
            .or_else(default_run_dir)
            .ok_or("Cannot locate the user profile for ~/.kohdalab/slm/run")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = uuid::Uuid::new_v4().to_string();
        let instance = uuid::Uuid::new_v4().to_string();
        let path = dir.join(format!("slm-{}-{instance}.json", std::process::id()));
        // NamedTempFile is created owner-only; the token never leaves it.
        let mut tmp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
        let d = descriptor(
            json!({"protocol":1,"app":APPLICATION,"address":format!("127.0.0.1:{port}"),"token":token,"instance_id":instance,"pid":std::process::id()}),
            APPLICATION,
            API_VERSION,
        );
        serde_json::to_writer(&mut tmp, &d).map_err(|e| e.to_string())?;
        tmp.flush().map_err(|e| e.to_string())?;
        tmp.persist_noclobber(&path).map_err(|e| e.to_string())?;
        let (s, r) = mpsc::sync_channel(QUEUE);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let down = ListenerDown(alive.clone(), path.clone(), wake.clone());
        let peers = Arc::new(AtomicUsize::new(0));
        thread::Builder::new()
            .name("slm-control-listener".into())
            .spawn(move || {
                let _down = down;
                let handle = |stream: TcpStream| {
                    let mut n = peers.load(Ordering::Acquire);
                    while n < PEERS {
                        match peers.compare_exchange(n, n + 1, Ordering::AcqRel, Ordering::Acquire)
                        {
                            Ok(_) => {
                                let s = s.clone();
                                let t = token.clone();
                                let p = peers.clone();
                                let wake = wake.clone();
                                thread::spawn(move || {
                                    let _guard = Permit(p);
                                    let _ = serve(stream, &t, &s, wake.as_ref());
                                });
                                break;
                            }
                            Err(v) => n = v,
                        }
                    }
                };
                let _ = accept_loop(|| listener.accept(), &stop, handle);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            requests: r,
            descriptor: path,
            stopped,
            alive,
        })
    }

    /// The listener is still accepting; `false` after it ended for any reason.
    pub fn is_active(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Tests: end the listener thread the way a permanent failure would.
    #[cfg(test)]
    pub(crate) fn stop_listener(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

/// Accept until stopped or a permanent error. Transient errors are retried;
/// non-loopback peers are dropped.
fn accept_loop<T>(
    mut accept: impl FnMut() -> std::io::Result<(T, std::net::SocketAddr)>,
    stop: &AtomicBool,
    mut handle: impl FnMut(T),
) -> std::io::Result<()> {
    use std::io::ErrorKind::{ConnectionAborted, ConnectionReset, Interrupted, WouldBlock};
    let transient = |e: &std::io::Error| {
        matches!(
            e.kind(),
            WouldBlock | Interrupted | ConnectionAborted | ConnectionReset
        )
    };
    while !stop.load(Ordering::Acquire) {
        match accept() {
            Ok((stream, peer)) if peer.ip().is_loopback() => handle(stream),
            Ok(_) => {}
            Err(e) if transient(&e) => thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Runs when the listener thread ends (normally, on error or on panic): mark
/// the server inactive, withdraw the descriptor so clients stop finding it,
/// and wake the GUI so it clears the indicator.
struct ListenerDown(Arc<AtomicBool>, PathBuf, Arc<dyn Fn() + Send + Sync>);
impl Drop for ListenerDown {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
        let _ = std::fs::remove_file(&self.1);
        (self.2)();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = std::fs::remove_file(&self.descriptor);
    }
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn error(code: &str, msg: &str) -> Value {
    json!({"ok":false,"error_code":code,"error":msg})
}

/// Every reply, including early refusals, is one complete JSON Lines record.
fn write_line(stream: &mut impl Write, value: &Value) -> Result<(), String> {
    let mut line = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

fn serve(
    mut stream: TcpStream,
    token: &str,
    sender: &mpsc::SyncSender<Request>,
    wake: &(dyn Fn() + Send + Sync),
) -> Result<(), String> {
    // On macOS an accepted stream can inherit the listener's nonblocking mode.
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + TIMEOUT;
    let value = answer(&mut stream, token, sender, wake, deadline);
    write_line(&mut stream, &value)
}

fn answer(
    stream: &mut TcpStream,
    token: &str,
    sender: &mpsc::SyncSender<Request>,
    wake: &(dyn Fn() + Send + Sync),
    deadline: Instant,
) -> Value {
    let body = match read(stream, deadline).and_then(|b| {
        serde_json::from_slice::<Value>(&b).map_err(|_| "Request must be JSON".into())
    }) {
        Ok(v) => v,
        Err(e) => return error("invalid_request", &e),
    };
    if !body.is_object() {
        return error("invalid_request", "Request must be a JSON object");
    }
    let transport_error = |code, message| common_response(&body, error(code, message));
    // Authentication precedes queueing: an unauthenticated body never reaches the GUI.
    if body.get("token").and_then(Value::as_str) != Some(token) {
        return transport_error("authentication", "GUI API authentication failed");
    }
    let (reply, rx) = mpsc::sync_channel(1);
    match sender.try_send(Request {
        body: body.clone(),
        deadline,
        reply,
    }) {
        Ok(()) => {
            wake();
            rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| transport_error("timeout", "GUI request timed out"))
        }
        Err(mpsc::TrySendError::Full(_)) => transport_error("busy", "GUI request queue is full"),
        Err(mpsc::TrySendError::Disconnected(_)) => {
            transport_error("unavailable", "GUI API is stopping")
        }
    }
}

fn read(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        stream
            .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
            .map_err(|e| e.to_string())?;
        let mut b = [0; 4096];
        let n = stream.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("Incomplete JSON request".into());
        }
        if let Some(i) = b[..n].iter().position(|c| *c == b'\n') {
            out.extend_from_slice(&b[..i]);
            return if out.len() <= LIMIT {
                Ok(out)
            } else {
                Err("Request exceeds 64 KiB".into())
            };
        }
        out.extend_from_slice(&b[..n]);
        if out.len() > LIMIT {
            return Err("Request exceeds 64 KiB".into());
        }
    }
}

// ----- GUI-side answers --------------------------------------------------

/// The last apply outcome as the GUI shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum LastApply {
    Idle,
    Applying {
        label: String,
    },
    Applied {
        label: String,
    },
    Failed {
        label: String,
        error: String,
        output_known: bool,
    },
}

/// State the GUI already holds for this frame. Built only while requests are
/// queued; owns copies so answering cannot affect the App.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub demo: bool,
    pub config: AppConfig,
    pub config_saved: bool,
    pub device: DeviceState,
    pub last_apply: LastApply,
    pub wavelength_busy: bool,
    /// A disconnect was requested and the worker has not reported
    /// `Disconnected` yet (it may still be drawing the reset frame).
    pub disconnecting: bool,
    /// Sticky: set when the output became unknown, cleared only by a
    /// backend-confirmed draw (connect or apply). It survives disconnect,
    /// because closing an unknown output does not verify what it shows.
    pub output_unverified: Option<String>,
    pub home: Option<PathBuf>,
    /// A client holds the writer lease.
    pub lease_owned: bool,
    /// The lease operation (or lease-end cleanup) still waiting for its
    /// worker acknowledgement: job ID and kind.
    pub operation: Option<(String, &'static str)>,
    /// That operation is a reset-then-close (stop/disconnect).
    pub closing: bool,
    /// Why the last lease ended (`released`, `expired`, …).
    pub last_revocation: Option<&'static str>,
}

/// Free text (worker errors, labels) can name local files; never report the
/// user's home directory.
pub(crate) fn redact(text: &str, home: Option<&Path>) -> String {
    match home.and_then(Path::to_str).filter(|h| h.len() > 1) {
        Some(home) => text.replace(home, "~"),
        None => text.to_string(),
    }
}

pub fn describe() -> Value {
    kohdalab_control_protocol::describe(
        APPLICATION,
        env!("CARGO_PKG_VERSION"),
        API_VERSION,
        &METHODS,
        json!({
            "supports_ownership": true,
            "exclusive_control": true,
            "read_only": false,
            // Only an explicit control.connect opens the output.
            "physical_auto_connect": false,
            // Patterns are shown on the SLM display; the USB wavelength is
            // never written by the API.
            "device_writes": true,
            "wavelength_writes": false,
            "stop_requires_lease": true,
            "heartbeat_seconds": crate::control_lease::HEARTBEAT_SECONDS,
            "lease_timeout_seconds": crate::control_lease::LEASE_TIMEOUT.as_secs_f64(),
            "on_release": "stop",
            "on_expiry": "stop",
            "on_transport_loss": "stop",
            "stop": "worker reset frame then close the output window",
            "reset_pattern": RESET_MEANING,
            "optical_state_verified": false,
            "display_selection": "gui_only",
            "input_mode_confirmation": "gui_only",
            "serial_operations": true,
            "takeover_blocked_while": ["lease_held", "operation_pending", "cleanup_pending", "output_unverified", "gui_busy"],
            "simulation_required_for_tests": true,
        }),
        json!({"gui_owned": true, "external_control": "lease"}),
    )
}

pub(crate) fn receipt_json(receipt: &ShowReceipt) -> Value {
    json!({
        "digest": receipt.digest,
        "uniform_code": receipt.uniform_code,
        "verification": receipt.verification,
        "verification_label": receipt.verification.label(),
        "display": receipt.display,
        "mode": receipt.mode,
        "at_unix_ms": receipt.at_unix_ms,
    })
}

/// Output as far as the backend has confirmed it. Precedence: `unknown`
/// (anything unverified) > `pending` (a connect, draw or disconnect in
/// flight) > `confirmed` (connected, last draw confirmed) or `none`.
fn output_state(s: &Snapshot) -> &'static str {
    let applying = matches!(s.last_apply, LastApply::Applying { .. });
    match &s.device {
        DeviceState::Unknown { .. } => "unknown",
        _ if s.output_unverified.is_some() => "unknown",
        DeviceState::Connecting => "pending",
        _ if applying || s.disconnecting || s.closing => "pending",
        DeviceState::Connected { .. } => "confirmed",
        DeviceState::Disconnected => "none",
    }
}

pub fn status(s: &Snapshot) -> Value {
    let home = s.home.as_deref();
    let applying = matches!(s.last_apply, LastApply::Applying { .. });
    let output = output_state(s);
    let output_known = matches!(output, "confirmed" | "none");
    let (phase, device) = match &s.device {
        _ if s.disconnecting || s.closing => ("stopping", "disconnecting"),
        DeviceState::Disconnected if s.output_unverified.is_some() => ("error", "disconnected"),
        DeviceState::Disconnected => ("disconnected", "disconnected"),
        DeviceState::Connecting => ("initializing", "connecting"),
        DeviceState::Connected { .. } => ("ready", "connected"),
        DeviceState::Unknown { .. } => ("error", "output_unknown"),
    };
    let busy = applying
        || s.disconnecting
        || s.wavelength_busy
        || s.operation.is_some()
        || matches!(s.device, DeviceState::Connecting);
    // The worker's last confirmed frame is `showing` only while nothing newer
    // is pending or unverified; otherwise it is history (`previous_showing`).
    let last_confirmed = match &s.device {
        DeviceState::Connected {
            showing,
            mode,
            backend,
            ..
        } => json!({
            "label": redact(&showing.label, home),
            "item_id": showing.item_id,
            "backend": backend,
            "mode": mode,
            "receipt": receipt_json(&showing.receipt),
        }),
        _ => Value::Null,
    };
    let (showing, previous) = if output == "confirmed" {
        (last_confirmed, Value::Null)
    } else {
        (Value::Null, last_confirmed)
    };
    let last = match &s.last_apply {
        LastApply::Idle => json!({"state": "idle"}),
        LastApply::Applying { label } => {
            json!({"state": "applying", "label": redact(label, home)})
        }
        LastApply::Applied { label } => json!({"state": "applied", "label": redact(label, home)}),
        LastApply::Failed {
            label,
            error,
            output_known,
        } => json!({
            "state": "failed",
            "label": redact(label, home),
            "error": redact(error, home),
            "output_known": output_known,
        }),
    };
    let reason = match (&s.device, &s.output_unverified) {
        (DeviceState::Unknown { reason }, _) | (_, Some(reason)) => json!(redact(reason, home)),
        _ => Value::Null,
    };
    let external = if s.lease_owned {
        "leased"
    } else if s.operation.is_some() {
        "cleanup_pending"
    } else if output == "unknown" {
        "output_unverified"
    } else {
        "available"
    };
    kohdalab_control_protocol::status(
        phase,
        busy,
        // The lease token is a capability and never appears in status.
        json!({
            "owned": s.lease_owned,
            "supported": true,
            "heartbeat_seconds": crate::control_lease::HEARTBEAT_SECONDS,
            "lease_timeout_seconds": crate::control_lease::LEASE_TIMEOUT.as_secs_f64(),
            "last_revocation": s.last_revocation,
        }),
        match output {
            "unknown" => "unknown",
            "pending" => "pending",
            _ => "not_required",
        },
        json!({
            "demo": s.demo,
            "simulated": s.config.output.backend == BackendKind::Demo,
            "device": device,
            "output": output,
            "output_known": output_known,
            "unknown_reason": reason,
            "showing": showing,
            "previous_showing": previous,
            "last_apply": last,
            "wavelength_busy": s.wavelength_busy,
            "external_control": external,
            "operation": s.operation.as_ref().map(|(id, kind)| json!({"job_id": id, "kind": kind})),
            "reset_code": s.config.profile.reset_code,
            "reset_meaning": RESET_MEANING,
        }),
    )
}

/// The GUI's in-memory configuration, sanitized. The file path, the monitor
/// device path and free-text notes are withheld; see `redacted`.
pub fn config(s: &Snapshot) -> Value {
    let c = &s.config;
    let p = &c.profile;
    json!({
        "source": if s.demo { "temporary_demo" } else { "file" },
        "saved": s.config_saved,
        "schema_version": c.schema_version,
        "profile": {
            "model": p.model,
            "model_label": p.model.label(),
            "generation": p.generation,
            "default_center_offset_px": p.default_center_offset_px,
            "reset_code": p.reset_code,
            "device_wavelength_nm": p.device_wavelength_nm,
            "calibration_note_present": !p.calibration_note.trim().is_empty(),
        },
        "output": {
            "backend": c.output.backend,
            "backend_label": c.output.backend.label(),
            "display": c.output.display.as_ref().map(|d| json!({
                "friendly_name": d.friendly_name,
                "gdi_name": d.gdi_name,
                "width": d.width,
                "height": d.height,
            })),
            "confirmed_mode": c.output.confirmed_mode.as_ref().map(|m| json!({
                "mode": m.mode,
                "confirmed_unix_s": m.confirmed_unix_s,
            })),
        },
        "redacted": [
            "config_path",
            "output.display.monitor_device_path",
            "output.confirmed_mode.note",
            "profile.calibration_note",
        ],
    })
}

/// Methods outside [`METHODS`] (and every native, non-`control.*` request).
pub fn unsupported() -> Value {
    error(
        "unsupported",
        "KohdaLab SLM supports only the control.* methods listed by control.describe",
    )
}

/// A request whose deadline passed while queued is answered without work.
pub fn expired(body: &Value) -> Value {
    common_response(
        body,
        error(
            "timeout",
            "Request expired before execution; no new action started",
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use slm_device::ModeConfirmation;
    use slm_device::backend::Verification;
    use slm_device::display::DisplaySelection;
    use slm_device::encoding::InputMode;
    use slm_device::worker::Showing;
    use std::io::{BufRead, BufReader};

    fn snapshot() -> Snapshot {
        let mut config = AppConfig::default();
        config.output.backend = BackendKind::Demo;
        config.output.display = Some(DisplaySelection {
            monitor_device_path: r"\\?\DISPLAY#SNT0001#serial-secret".into(),
            gdi_name: r"\\.\DISPLAY3".into(),
            friendly_name: "SLM-200".into(),
            width: 1920,
            height: 1200,
        });
        config.output.confirmed_mode = Some(ModeConfirmation {
            mode: InputMode::Hz60,
            confirmed_unix_s: 7,
            note: "token=hunter2 checked on bench".into(),
        });
        config.profile.calibration_note = "password: hunter2".into();
        Snapshot {
            demo: false,
            config,
            config_saved: true,
            device: DeviceState::Disconnected,
            last_apply: LastApply::Idle,
            wavelength_busy: false,
            disconnecting: false,
            output_unverified: None,
            home: Some(PathBuf::from("/Users/someone")),
            lease_owned: false,
            operation: None,
            closing: false,
            last_revocation: None,
        }
    }

    fn connected(label: &str) -> DeviceState {
        DeviceState::Connected {
            backend: BackendKind::Demo,
            display: slm_device::demo::demo_displays()[1].clone(),
            mode: InputMode::Hz60,
            showing: Showing {
                item_id: None,
                label: label.into(),
                receipt: ShowReceipt {
                    digest: "abc".into(),
                    uniform_code: Some(0),
                    verification: Verification::Simulated,
                    display: "DEMO".into(),
                    mode: InputMode::Hz60,
                    at_unix_ms: 1,
                },
            },
        }
    }

    fn common(method: &str) -> Value {
        json!({"control_protocol": 1, "method": method, "request_id": "r1"})
    }

    #[test]
    fn describe_advertises_lease_methods_and_conservative_policy() {
        let d = describe();
        assert_eq!(d["application"], APPLICATION);
        assert_eq!(d["methods"], json!(METHODS));
        for m in MUTATIONS {
            assert!(METHODS.contains(&format!("control.{m}").as_str()), "{m}");
        }
        let safety = &d["safety"];
        assert_eq!(safety["supports_ownership"], true);
        assert_eq!(safety["read_only"], false);
        assert_eq!(safety["physical_auto_connect"], false);
        assert_eq!(safety["wavelength_writes"], false);
        assert_eq!(safety["stop_requires_lease"], true);
        assert_eq!(safety["on_release"], "stop");
        assert_eq!(safety["on_expiry"], "stop");
        assert_eq!(safety["optical_state_verified"], false);
        assert_eq!(safety["display_selection"], "gui_only");
        // The SDK accepts heartbeat intervals of 0.1–2.0 s.
        let hb = safety["heartbeat_seconds"].as_f64().unwrap();
        assert!((0.1..=2.0).contains(&hb));
        assert!(safety["lease_timeout_seconds"].as_f64().unwrap() > 2.0 * hb);
        // No stop_on_error policy: the SDK sends no release reason by default.
        assert!(safety.get("stop_policy_defaults").is_none());
        assert!(
            safety["reset_pattern"]
                .as_str()
                .unwrap()
                .contains("not verified dark")
        );
    }

    #[test]
    fn config_withholds_paths_device_ids_and_free_text() {
        let r = common_response(
            &common("control.config"),
            json!({"ok": true, "result": config(&snapshot())}),
        );
        let text = r.to_string();
        for secret in [
            "hunter2",
            "serial-secret",
            "monitor_device_path\":\"",
            "/Users/",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        let c = &r["result"];
        assert_eq!(c["source"], "file");
        assert_eq!(c["saved"], true);
        assert_eq!(c["output"]["backend"], "demo");
        assert_eq!(c["output"]["display"]["friendly_name"], "SLM-200");
        assert_eq!(c["output"]["display"]["width"], 1920);
        assert_eq!(c["output"]["confirmed_mode"]["mode"], "60hz");
        assert_eq!(c["profile"]["generation"]["max_code"], 1023);
        assert_eq!(c["profile"]["calibration_note_present"], true);
    }

    #[test]
    fn status_reports_unknown_output_honestly_and_redacts_home() {
        let mut s = snapshot();
        let r = status(&s);
        assert_eq!(r["phase"], "disconnected");
        assert_eq!(r["busy"], false);
        assert_eq!(r["ownership"]["owned"], false);
        assert_eq!(r["ownership"]["supported"], true);
        assert_eq!(r["cleanup"], "not_required");
        assert_eq!(r["native"]["external_control"], "available");
        assert_eq!(r["native"]["reset_code"], 0);

        s.device = DeviceState::Unknown {
            reason: "Display moved; see /Users/someone/x.log".into(),
        };
        let r = status(&s);
        assert_eq!(r["phase"], "error");
        assert_eq!(r["cleanup"], "unknown");
        assert_eq!(r["native"]["output_known"], false);
        assert_eq!(r["native"]["unknown_reason"], "Display moved; see ~/x.log");

        // Disconnecting from an unknown output stays unknown, and so does the
        // closed output afterwards: closing does not verify what is shown.
        s.output_unverified = Some("Display moved".into());
        s.disconnecting = true;
        let r = status(&s);
        assert_eq!(r["phase"], "stopping");
        assert_eq!(r["busy"], true);
        assert_eq!(r["cleanup"], "unknown");
        s.disconnecting = false;
        s.device = DeviceState::Disconnected;
        let r = status(&s);
        // Conservative: an unverified closed output is an error phase, so a
        // status-polling stop/release reports cleanup_unconfirmed.
        assert_eq!(r["phase"], "error");
        assert_eq!(r["native"]["device"], "disconnected");
        assert_eq!(r["busy"], false);
        assert_eq!(r["cleanup"], "unknown");
        assert_eq!(r["native"]["output"], "unknown");
        assert_eq!(r["native"]["unknown_reason"], "Display moved");
        assert_eq!(r["native"]["external_control"], "output_unverified");

        // A confirmed connect (fresh receipt) is the positive event that clears it.
        s.output_unverified = None;
        s.device = connected("Reset");
        let r = status(&s);
        assert_eq!(r["phase"], "ready");
        assert_eq!(r["cleanup"], "not_required");
        assert_eq!(r["native"]["output"], "confirmed");
        assert_eq!(r["native"]["showing"]["receipt"]["uniform_code"], 0);
        assert_eq!(r["native"]["previous_showing"], Value::Null);
        assert_eq!(r["native"]["simulated"], true);
    }

    #[test]
    fn apply_and_disconnect_in_flight_are_busy_pending_and_not_current() {
        let mut s = snapshot();
        s.device = connected("Reset");
        for (applying, disconnecting) in [(true, false), (false, true)] {
            s.last_apply = if applying {
                LastApply::Applying {
                    label: "Single".into(),
                }
            } else {
                LastApply::Applied {
                    label: "Single".into(),
                }
            };
            s.disconnecting = disconnecting;
            let r = status(&s);
            assert_eq!(r["busy"], true);
            assert_eq!(r["cleanup"], "pending");
            assert_eq!(r["native"]["output"], "pending");
            assert_eq!(r["native"]["output_known"], false);
            // The prior receipt is history, never the current confirmed frame.
            assert_eq!(r["native"]["showing"], Value::Null);
            assert_eq!(r["native"]["previous_showing"]["label"], "Reset");
            let phase = if disconnecting { "stopping" } else { "ready" };
            assert_eq!(r["phase"], phase);
        }
        s.disconnecting = false;
        s.device = DeviceState::Connecting;
        let r = status(&s);
        assert_eq!(r["busy"], true);
        assert_eq!(r["cleanup"], "pending");

        // A known failure keeps the previous frame, which is still confirmed.
        s.device = connected("Reset");
        s.last_apply = LastApply::Failed {
            label: "Single".into(),
            error: "render failed".into(),
            output_known: true,
        };
        let r = status(&s);
        assert_eq!(r["busy"], false);
        assert_eq!(r["native"]["output"], "confirmed");
        assert_eq!(r["native"]["showing"]["label"], "Reset");

        // A leased stop waiting for its worker acknowledgement.
        s.lease_owned = true;
        s.operation = Some(("job".into(), "stop"));
        s.closing = true;
        let r = status(&s);
        assert_eq!(r["busy"], true);
        assert_eq!(r["phase"], "stopping");
        assert_eq!(r["cleanup"], "pending");
        assert_eq!(r["ownership"]["owned"], true);
        assert_eq!(r["native"]["operation"]["kind"], "stop");
        assert_eq!(r["native"]["external_control"], "leased");
        assert!(!r.to_string().contains("lease\":\""), "no lease token");
    }

    #[test]
    fn accept_loop_retries_transient_errors_and_returns_on_permanent_ones() {
        use std::io::{Error, ErrorKind};
        let stop = AtomicBool::new(false);
        let mut calls = 0;
        let mut handled = Vec::new();
        let r = accept_loop(
            || {
                calls += 1;
                match calls {
                    1 => Err(Error::from(ErrorKind::WouldBlock)),
                    2 => Err(Error::from(ErrorKind::ConnectionAborted)),
                    3 => Ok((1, "127.0.0.1:9".parse().unwrap())),
                    4 => Ok((2, "192.0.2.1:9".parse().unwrap())),
                    _ => Err(Error::other("listener closed")),
                }
            },
            &stop,
            |n| handled.push(n),
        );
        assert!(r.is_err());
        assert_eq!(calls, 5);
        assert_eq!(handled, [1], "non-loopback peer dropped");
    }

    #[test]
    fn listener_exit_marks_server_inactive_and_withdraws_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let woke = Arc::new(AtomicBool::new(false));
        let w = woke.clone();
        let wake = move || w.store(true, Ordering::Release);
        let server = Server::start(Some(dir.path().into()), wake).unwrap();
        assert!(server.is_active());
        server.stop_listener();
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.is_active() {
            assert!(Instant::now() < deadline, "listener did not end");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!server.descriptor.exists());
        assert!(woke.load(Ordering::Acquire), "GUI woken");
    }

    #[test]
    fn expired_requests_report_timeout_without_work() {
        let r = expired(&common("control.status"));
        assert_eq!(r["error"]["code"], "timeout");
        assert_eq!(r["request_id"], "r1");
    }

    fn descriptor_of(server: &Server) -> Value {
        serde_json::from_slice(&std::fs::read(&server.descriptor).unwrap()).unwrap()
    }

    #[test]
    fn descriptor_is_app_specific_and_removed_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(Some(dir.path().into()), || {}).unwrap();
        let d = descriptor_of(&server);
        assert_eq!(d["application"], APPLICATION);
        assert_eq!(d["control_protocol"], 1);
        assert_eq!(d["host"], "127.0.0.1");
        assert!(d["token"].as_str().is_some_and(|t| t.len() >= 32));
        let name = server.descriptor.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("slm-"), "{name}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&server.descriptor)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "descriptor readable by others: {mode:o}");
        }
        let path = server.descriptor.clone();
        drop(server);
        assert!(!path.exists());
        assert!(default_run_dir().is_none_or(|p| p.ends_with(".kohdalab/slm/run")));
    }

    #[test]
    fn bad_token_never_reaches_gui_and_good_token_is_queued() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(Some(dir.path().into()), || {}).unwrap();
        let d = descriptor_of(&server);
        let port = d["port"].as_u64().unwrap() as u16;
        let exchange = |body: Value| {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            writeln!(stream, "{body}").unwrap();
            stream
        };
        let mut line = String::new();
        BufReader::new(exchange(
            json!({"control_protocol":1,"method":"control.status","token":"wrong"}),
        ))
        .read_line(&mut line)
        .unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["error"]["code"], "authentication");
        assert!(server.requests.try_recv().is_err());

        let stream =
            exchange(json!({"control_protocol":1,"method":"control.describe","token":d["token"]}));
        let request = server.requests.recv_timeout(TIMEOUT).unwrap();
        request
            .reply
            .send(common_response(
                &request.body,
                json!({"ok": true, "result": describe()}),
            ))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result"]["application"], APPLICATION);
    }
}
