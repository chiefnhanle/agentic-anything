//! Test doubles for the frozen traits (`cfg(any(test, feature = "testing"))`).
//!
//! - [`FakeRunner`]: a scripted [`ProcessRunner`] that records every [`RunSpec`] and never spawns anything.
//! - [`MemoryAudit`]: an in-memory [`AuditSink`] with a real hash chain (same entry digest rule as the JSONL log).

use super::{AuditSink, ProcessRunner, RunOutput, RunSpec};
use crate::{AaError, AaResult, digest, pb};
use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Clone)]
enum Script {
    Output(RunOutput),
    /// Never completes; the future is expected to be dropped.
    Hang,
}

/// Scripted `RunOutput` per argv prefix. The longest matching prefix wins; ties go to the rule registered first.
/// An argv with no matching rule yields `CAPABILITY_UNAVAILABLE` (the call is still recorded).
#[derive(Debug)]
pub struct FakeRunner {
    rules: Vec<(Vec<String>, Script)>,
    calls: Mutex<Vec<RunSpec>>,
    dropped: Arc<AtomicUsize>,
    isolation: pb::Isolation,
}

impl Default for FakeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeRunner {
    /// No rules; `max_isolation()` reports `ISOLATION_PROCESS`.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            calls: Mutex::new(Vec::new()),
            dropped: Arc::default(),
            isolation: pb::Isolation::Process,
        }
    }

    /// Scripts `out` for every argv starting with `argv_prefix`.
    pub fn on(mut self, argv_prefix: &[&str], out: RunOutput) -> Self {
        self.rules
            .push((argv_prefix.iter().map(|s| s.to_string()).collect(), Script::Output(out)));
        self
    }

    /// Every argv starting with `argv_prefix` hangs forever (until its future is dropped; see [`Self::dropped`]).
    pub fn hang(mut self, argv_prefix: &[&str]) -> Self {
        self.rules
            .push((argv_prefix.iter().map(|s| s.to_string()).collect(), Script::Hang));
        self
    }

    /// Overrides what `max_isolation()` reports.
    pub fn with_isolation(mut self, isolation: pb::Isolation) -> Self {
        self.isolation = isolation;
        self
    }

    /// Every spec passed to `run`, in call order.
    pub fn calls(&self) -> Vec<RunSpec> {
        lock(&self.calls).clone()
    }

    /// How many `run` futures were dropped before completing (the "kill the group" path).
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }

    fn script_for(&self, argv: &[String]) -> Option<Script> {
        self.rules
            .iter()
            .filter(|(prefix, _)| argv.starts_with(prefix))
            .fold(None::<&(Vec<String>, Script)>, |best, r| match best {
                Some(b) if b.0.len() >= r.0.len() => Some(b),
                _ => Some(r),
            })
            .map(|(_, s)| s.clone())
    }
}

struct DropProbe {
    counter: Arc<AtomicUsize>,
    armed: bool,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        if self.armed {
            self.counter.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[async_trait::async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(&self, spec: RunSpec) -> AaResult<RunOutput> {
        let script = self.script_for(&spec.argv);
        let argv = spec.argv.clone();
        lock(&self.calls).push(spec);
        let mut probe = DropProbe {
            counter: Arc::clone(&self.dropped),
            armed: true,
        };
        let result = match script {
            Some(Script::Output(out)) => Ok(out),
            Some(Script::Hang) => {
                std::future::pending::<()>().await;
                unreachable!("pending never resolves")
            }
            None => Err(AaError::new(
                pb::ErrorCode::CapabilityUnavailable,
                format!("FakeRunner has no scripted output for argv {argv:?}"),
                "script the argv prefix with FakeRunner::on",
            )),
        };
        probe.armed = false;
        result
    }

    fn max_isolation(&self) -> pb::Isolation {
        self.isolation
    }
}

/// A successful run: exit 0, `stdout`, empty stderr, cleanup COMPLETE, at `ISOLATION_PROCESS`.
pub fn ok_output(stdout: &str) -> RunOutput {
    RunOutput {
        exit_code: Some(0),
        signal: None,
        stdout: stdout.as_bytes().to_vec(),
        stdout_total: stdout.len() as u64,
        stdout_truncated: false,
        stderr_excerpt: String::new(),
        stderr_total: 0,
        wall: Duration::from_millis(1),
        timed_out: false,
        cleanup: pb::CleanupStatus::Complete,
        confinement_denials: Vec::new(),
        isolation: pb::Isolation::Process,
    }
}

/// `Vec<pb::AuditEntry>` with a real hash chain: seq starts at 1, `prev_sha256` of the first entry is `""`,
/// `entry_sha256 = sha256(JCS(entry with entry_sha256 = ""))`.
#[derive(Debug, Default)]
pub struct MemoryAudit {
    entries: Mutex<Vec<pb::AuditEntry>>,
    fail_next_intent: AtomicBool,
}

impl MemoryAudit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> Vec<pb::AuditEntry> {
        lock(&self.entries).clone()
    }

    /// The next `append_intent` fails with `AUDIT_UNAVAILABLE` (and appends nothing).
    pub fn fail_next_intent(&self) {
        self.fail_next_intent.store(true, Ordering::SeqCst);
    }

    /// Only the ActionRecords, in order.
    pub fn actions(&self) -> Vec<pb::ActionRecord> {
        self.entries()
            .into_iter()
            .filter_map(|e| match e.body {
                Some(pb::audit_entry::Body::Action(a)) => Some(a),
                _ => None,
            })
            .collect()
    }

    /// Recomputes the whole chain; `true` if every link and digest verifies.
    pub fn verify_chain(&self) -> bool {
        let entries = lock(&self.entries);
        let mut prev = String::new();
        for (i, e) in entries.iter().enumerate() {
            let ok_digest = entry_digest(e).is_ok_and(|d| d == e.entry_sha256);
            if e.seq != i as u64 + 1 || e.prev_sha256 != prev || !ok_digest {
                return false;
            }
            prev = e.entry_sha256.clone();
        }
        true
    }

    fn append(&self, principal: &pb::Principal, halt_epoch: u64, body: pb::audit_entry::Body) -> AaResult<u64> {
        let mut entries = lock(&self.entries);
        let (seq, prev) = entries
            .last()
            .map_or((1, String::new()), |l| (l.seq + 1, l.entry_sha256.clone()));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let mut entry = pb::AuditEntry {
            seq,
            prev_sha256: prev,
            entry_sha256: String::new(),
            written_at: Some(pbjson_types::Timestamp {
                seconds: i64::try_from(now.as_secs()).unwrap_or(i64::MAX),
                nanos: i32::try_from(now.subsec_nanos()).unwrap_or(0),
            }),
            principal: Some(principal.clone()),
            halt_epoch,
            body: Some(body),
        };
        entry.entry_sha256 = entry_digest(&entry)?;
        entries.push(entry);
        Ok(seq)
    }
}

/// `sha256(JCS(entry with entry_sha256 = ""))`.
pub fn entry_digest(entry: &pb::AuditEntry) -> AaResult<String> {
    digest::self_digest(entry, |e| e.entry_sha256.clear())
}

impl AuditSink for MemoryAudit {
    fn append_intent(&self, principal: &pb::Principal, halt_epoch: u64, intent: pb::ActionIntent) -> AaResult<u64> {
        if self.fail_next_intent.swap(false, Ordering::SeqCst) {
            return Err(AaError::new(
                pb::ErrorCode::AuditUnavailable,
                "intent could not be made durable (MemoryAudit::fail_next_intent)",
                "aa audit --verify",
            ));
        }
        self.append(principal, halt_epoch, pb::audit_entry::Body::Intent(intent))
    }

    fn append_action(&self, principal: &pb::Principal, halt_epoch: u64, record: pb::ActionRecord) -> AaResult<u64> {
        self.append(principal, halt_epoch, pb::audit_entry::Body::Action(record))
    }

    fn append_admin(&self, principal: &pb::Principal, halt_epoch: u64, event: pb::AdminEvent) -> AaResult<u64> {
        self.append(principal, halt_epoch, pb::audit_entry::Body::Admin(event))
    }

    fn head(&self) -> (u64, String) {
        lock(&self.entries)
            .last()
            .map_or((0, String::new()), |e| (e.seq, e.entry_sha256.clone()))
    }

    fn entry(&self, seq: u64) -> AaResult<Option<pb::AuditEntry>> {
        Ok(lock(&self.entries).iter().find(|e| e.seq == seq).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    fn spec(argv: &[&str]) -> RunSpec {
        RunSpec {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: "/".into(),
            env: BTreeMap::new(),
            stdin: None,
            deadline: Duration::from_secs(1),
            max_stdout: 1024,
            max_stderr_excerpt: 256,
            confinement: None,
            run_id: "r".into(),
        }
    }

    /// Minimal executor: these futures never touch a reactor.
    fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn fake_runner_longest_prefix_and_records_calls() {
        let r = FakeRunner::new()
            .on(&["/bin/git"], ok_output("short"))
            .on(&["/bin/git", "status"], ok_output("long"));
        let out = block_on(r.run(spec(&["/bin/git", "status", "-s"]))).unwrap();
        assert_eq!(out.stdout, b"long");
        let out = block_on(r.run(spec(&["/bin/git", "log"]))).unwrap();
        assert_eq!(out.stdout, b"short");
        let err = block_on(r.run(spec(&["/bin/gh"]))).unwrap_err();
        assert_eq!(err.code, pb::ErrorCode::CapabilityUnavailable);
        assert_eq!(r.calls().len(), 3);
        assert_eq!(r.dropped(), 0);
        assert_eq!(r.max_isolation(), pb::Isolation::Process);
    }

    #[test]
    fn fake_runner_hang_observes_drop() {
        let r = FakeRunner::new().hang(&["/bin/sleep"]);
        {
            let mut fut = r.run(spec(&["/bin/sleep", "100"]));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(fut.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(r.dropped(), 1);
        assert_eq!(r.calls().len(), 1);
    }

    #[test]
    fn memory_audit_chain() {
        let a = MemoryAudit::new();
        assert_eq!(a.head(), (0, String::new()));
        let p = pb::Principal {
            role: pb::Role::Agent as i32,
            ..Default::default()
        };
        let s1 = a.append_admin(
            &p,
            0,
            pb::AdminEvent {
                kind: pb::AdminEventKind::EngineStart as i32,
                ..Default::default()
            },
        );
        assert_eq!(s1.unwrap(), 1);
        a.fail_next_intent();
        assert_eq!(
            a.append_intent(&p, 0, pb::ActionIntent::default()).unwrap_err().code,
            pb::ErrorCode::AuditUnavailable
        );
        assert_eq!(
            a.append_intent(
                &p,
                0,
                pb::ActionIntent {
                    action_id: "act_1".into(),
                    ..Default::default()
                }
            )
            .unwrap(),
            2
        );
        assert_eq!(
            a.append_action(
                &p,
                0,
                pb::ActionRecord {
                    action_id: "act_1".into(),
                    ..Default::default()
                }
            )
            .unwrap(),
            3
        );
        assert!(a.verify_chain());
        let (seq, head) = a.head();
        assert_eq!(seq, 3);
        assert_eq!(a.entry(3).unwrap().unwrap().entry_sha256, head);
        assert_eq!(
            a.entry(2).unwrap().unwrap().entry_sha256,
            a.entry(3).unwrap().unwrap().prev_sha256
        );
        assert!(a.entry(4).unwrap().is_none());
        assert_eq!(a.actions().len(), 1);

        // Tamper: the chain no longer verifies.
        lock(&a.entries)[1].halt_epoch = 9;
        assert!(!a.verify_chain());
    }
}
