//! Writer lease, job receipts and request-ID deduplication for the GUI
//! control API.
//!
//! Pure state: no sockets, no device worker, no App. The App owns one
//! [`Engine`], drives it from the GUI thread, and completes jobs only from
//! real worker acknowledgements (`ConnectDone`, `Applied`/`ApplyFailed`,
//! `DisconnectDone`), matched by the worker request number.
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Interval the Python SDK heartbeat thread uses (SDK accepts 0.1–2.0 s).
pub const HEARTBEAT_SECONDS: f64 = 0.5;
/// A lease without a heartbeat for this long is revoked and output stopped.
pub const LEASE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_JOBS: usize = 64;
const MAX_REQUESTS: usize = 128;
const MAX_REVOKED: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Connect,
    Apply,
    Reset,
    Disconnect,
    Stop,
}

impl JobKind {
    pub fn name(self) -> &'static str {
        match self {
            JobKind::Connect => "connect",
            JobKind::Apply => "apply",
            JobKind::Reset => "reset",
            JobKind::Disconnect => "disconnect",
            JobKind::Stop => "stop",
        }
    }
    /// Runs the worker's reset-then-close path.
    pub fn closes(self) -> bool {
        matches!(self, JobKind::Disconnect | JobKind::Stop)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Completed,
    Error,
}

impl JobState {
    pub fn name(self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Error => "error",
        }
    }
}

/// Why a job was started (`native.origin` in its receipt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Request,
    Release,
    LeaseExpired,
    GuiStop,
    ListenerEnded,
}

impl Origin {
    pub fn name(self) -> &'static str {
        match self {
            Origin::Request => "request",
            Origin::Release => "release",
            Origin::LeaseExpired => "lease_expired",
            Origin::GuiStop => "gui_stop",
            Origin::ListenerEnded => "listener_ended",
        }
    }
}

/// Why a lease ended. A request that still carries it gets `revoked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revocation {
    Released,
    Expired,
    GuiTakeControl,
    GuiStop,
    ListenerEnded,
}

impl Revocation {
    pub fn name(self) -> &'static str {
        match self {
            Revocation::Released => "released",
            Revocation::Expired => "expired",
            Revocation::GuiTakeControl => "gui_take_control",
            Revocation::GuiStop => "gui_stop",
            Revocation::ListenerEnded => "listener_ended",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lease {
    pub token: String,
    pub client_id: String,
    pub last_seen: Instant,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub origin: Origin,
    /// Worker request number; the acknowledgement carries the same number.
    pub worker_request: u64,
    pub state: JobState,
    /// `pending` while running; then `verified`, `not_required` or `unknown`.
    pub cleanup: &'static str,
    pub native: Map<String, Value>,
}

impl Job {
    pub fn receipt(&self) -> Value {
        let mut native = self.native.clone();
        native.insert("origin".into(), json!(self.origin.name()));
        kohdalab_control_protocol::receipt(
            &self.id,
            self.kind.name(),
            self.state.name(),
            self.cleanup,
            None,
            Value::Object(native),
        )
    }
}

/// Outcome of looking a request ID up in the deduplication record.
#[derive(Debug, PartialEq)]
pub enum Seen {
    New,
    /// Same request (method, client and parameters): its original answer.
    Duplicate(Value),
    /// Same ID, different request: refused, nothing is executed.
    Conflict,
}

#[derive(Default)]
pub struct Engine {
    pub lease: Option<Lease>,
    pub last_revocation: Option<Revocation>,
    revoked: VecDeque<(String, Revocation)>,
    jobs: VecDeque<Job>,
    seen: VecDeque<(String, String, Value)>,
}

pub type Refusal = (&'static str, String);

impl Engine {
    pub fn grant(&mut self, client_id: &str, now: Instant) -> &Lease {
        self.lease.insert(Lease {
            token: uuid::Uuid::new_v4().to_string(),
            client_id: client_id.to_string(),
            last_seen: now,
        })
    }

    /// The caller holds the active lease.
    pub fn check_lease(&self, client_id: &str, lease: Option<&str>) -> Result<(), Refusal> {
        let Some(lease) = lease else {
            return Err((
                "ownership",
                "This operation needs the SLM lease; call control.acquire first".into(),
            ));
        };
        if let Some(active) = &self.lease
            && active.token == lease
        {
            return if active.client_id == client_id {
                Ok(())
            } else {
                Err((
                    "invalid_lease",
                    "The lease belongs to another client".into(),
                ))
            };
        }
        match self.revoked.iter().find(|(token, _)| token == lease) {
            Some((_, why)) => Err((
                "revoked",
                format!(
                    "The SLM lease was revoked ({}); acquire a new lease with a new client",
                    why.name()
                ),
            )),
            None => Err(("invalid_lease", "Unknown SLM lease".into())),
        }
    }

    pub fn renew(&mut self, now: Instant) {
        if let Some(lease) = &mut self.lease {
            lease.last_seen = now;
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.lease
            .as_ref()
            .is_some_and(|l| now.saturating_duration_since(l.last_seen) > LEASE_TIMEOUT)
    }

    /// End the active lease; later requests carrying it get `revoked`.
    pub fn revoke(&mut self, why: Revocation) -> bool {
        let Some(lease) = self.lease.take() else {
            return false;
        };
        self.revoked.push_back((lease.token, why));
        while self.revoked.len() > MAX_REVOKED {
            self.revoked.pop_front();
        }
        self.last_revocation = Some(why);
        true
    }

    /// The oldest job still waiting for its worker acknowledgement. Only a
    /// stop/release cleanup can be queued behind another running job.
    pub fn running(&self) -> Option<&Job> {
        self.jobs.iter().find(|j| j.state == JobState::Running)
    }

    /// A reset-then-close job still waiting for its acknowledgement.
    pub fn running_close(&self) -> Option<&Job> {
        self.jobs
            .iter()
            .find(|j| j.state == JobState::Running && j.kind.closes())
    }

    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn start(
        &mut self,
        kind: JobKind,
        origin: Origin,
        worker_request: u64,
        native: Value,
    ) -> &Job {
        let native = match native {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        self.jobs.push_back(Job {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            origin,
            worker_request,
            state: JobState::Running,
            cleanup: "pending",
            native,
        });
        // Evict the oldest finished receipts; running jobs are never evicted.
        while self.jobs.len() > MAX_JOBS {
            match self.jobs.iter().position(|j| j.state != JobState::Running) {
                Some(i) => {
                    self.jobs.remove(i);
                }
                None => break,
            }
        }
        self.jobs.back().expect("job just pushed")
    }

    /// Complete the running job of one of `kinds` acknowledged by the worker
    /// as `worker_request`. Unrelated (manual GUI) acknowledgements match
    /// nothing and are ignored.
    pub fn finish(
        &mut self,
        worker_request: u64,
        kinds: &[JobKind],
        state: JobState,
        cleanup: &'static str,
        native: Value,
    ) -> Option<&Job> {
        let job = self.jobs.iter_mut().find(|j| {
            j.state == JobState::Running
                && j.worker_request == worker_request
                && kinds.contains(&j.kind)
        })?;
        job.state = state;
        job.cleanup = cleanup;
        if let Value::Object(extra) = native {
            job.native.extend(extra);
        }
        Some(job)
    }

    /// Deduplication: request IDs of accepted mutations are remembered with
    /// a fingerprint of method, client and parameters.
    pub fn lookup(&self, request_id: &str, fingerprint: &str) -> Seen {
        match self.seen.iter().find(|(id, _, _)| id == request_id) {
            None => Seen::New,
            Some((_, f, answer)) if f == fingerprint => Seen::Duplicate(answer.clone()),
            Some(_) => Seen::Conflict,
        }
    }

    pub fn remember(&mut self, request_id: &str, fingerprint: String, answer: Value) {
        self.seen
            .push_back((request_id.to_string(), fingerprint, answer));
        while self.seen.len() > MAX_REQUESTS {
            self.seen.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_is_checked_per_client_and_revoked_leases_say_why() {
        let mut e = Engine::default();
        let now = Instant::now();
        let token = e.grant("a", now).token.clone();
        assert!(e.check_lease("a", Some(&token)).is_ok());
        assert_eq!(
            e.check_lease("b", Some(&token)).unwrap_err().0,
            "invalid_lease"
        );
        assert_eq!(e.check_lease("a", None).unwrap_err().0, "ownership");
        assert_eq!(
            e.check_lease("a", Some("forged")).unwrap_err().0,
            "invalid_lease"
        );
        assert!(e.revoke(Revocation::GuiTakeControl));
        assert!(!e.revoke(Revocation::GuiTakeControl), "only once");
        let (code, message) = e.check_lease("a", Some(&token)).unwrap_err();
        assert_eq!(code, "revoked");
        assert!(message.contains("gui_take_control"), "{message}");
        assert_eq!(e.last_revocation, Some(Revocation::GuiTakeControl));
    }

    #[test]
    fn expiry_uses_the_last_heartbeat() {
        let mut e = Engine::default();
        let t0 = Instant::now();
        e.grant("a", t0);
        assert!(!e.expired(t0 + LEASE_TIMEOUT));
        assert!(e.expired(t0 + LEASE_TIMEOUT + Duration::from_millis(1)));
        e.renew(t0 + Duration::from_secs(2));
        assert!(!e.expired(t0 + LEASE_TIMEOUT + Duration::from_millis(1)));
        // Clock values before the heartbeat never underflow.
        assert!(!e.expired(t0));
    }

    #[test]
    fn jobs_complete_only_from_their_own_acknowledgement() {
        let mut e = Engine::default();
        let id = e
            .start(JobKind::Apply, Origin::Request, 5, json!({"label": "x"}))
            .id
            .clone();
        assert_eq!(e.running().unwrap().id, id);
        let r = e.job(&id).unwrap().receipt();
        assert_eq!(r["status"], "running");
        assert_eq!(r["terminal"], false);
        assert_eq!(r["cleanup"], "pending");
        // Another request number or a closing ack does not complete it.
        assert!(
            e.finish(
                6,
                &[JobKind::Apply],
                JobState::Completed,
                "not_required",
                json!({})
            )
            .is_none()
        );
        assert!(
            e.finish(
                5,
                &[JobKind::Stop],
                JobState::Completed,
                "verified",
                json!({})
            )
            .is_none()
        );
        e.finish(
            5,
            &[JobKind::Apply, JobKind::Reset],
            JobState::Error,
            "unknown",
            json!({"error": "lost"}),
        );
        let r = e.job(&id).unwrap().receipt();
        assert_eq!(r["status"], "error");
        assert_eq!(r["terminal"], true);
        assert_eq!(r["cleanup"], "unknown");
        assert_eq!(r["native"]["label"], "x");
        assert_eq!(r["native"]["error"], "lost");
        assert_eq!(r["native"]["origin"], "request");
        assert!(e.running().is_none());
        // A terminal job is never completed twice.
        assert!(
            e.finish(
                5,
                &[JobKind::Apply],
                JobState::Completed,
                "not_required",
                json!({})
            )
            .is_none()
        );
    }

    #[test]
    fn finished_receipts_are_bounded_but_running_jobs_are_kept() {
        let mut e = Engine::default();
        let first = e
            .start(JobKind::Stop, Origin::LeaseExpired, 1, json!({}))
            .id
            .clone();
        for n in 0..(MAX_JOBS as u64 + 10) {
            e.start(JobKind::Apply, Origin::Request, 100 + n, json!({}));
            e.finish(
                100 + n,
                &[JobKind::Apply],
                JobState::Completed,
                "not_required",
                json!({}),
            );
        }
        assert!(e.job(&first).is_some(), "running job evicted");
        assert_eq!(e.jobs.len(), MAX_JOBS);
        assert_eq!(e.running_close().unwrap().id, first);
    }

    #[test]
    fn request_ids_deduplicate_and_conflict() {
        let mut e = Engine::default();
        assert_eq!(e.lookup("r", "f"), Seen::New);
        e.remember("r", "f".into(), json!({"ok": true}));
        assert_eq!(e.lookup("r", "f"), Seen::Duplicate(json!({"ok": true})));
        assert_eq!(e.lookup("r", "g"), Seen::Conflict);
        for n in 0..MAX_REQUESTS {
            e.remember(&n.to_string(), "f".into(), json!(n));
        }
        assert_eq!(e.lookup("r", "f"), Seen::New, "bounded record");
    }
}
