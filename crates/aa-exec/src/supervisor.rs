//! The supervision kernel: owned process group, scrubbed env, deadlines, bounded output, drop-kills-group.
//!
//! One `run()`:
//! 1. validates the spec (absolute argv[0], no NUL bytes), and wraps the argv with the [`Confiner`] when
//!    `spec.confinement` is set (refusing with `ISOLATION_NOT_PROVEN` if confinement is unavailable: a run that
//!    asked for confinement never silently runs unconfined);
//! 2. spawns argv[0] directly (never a shell) as the leader of a new process group (`process-wrap`), with
//!    `env_clear()` + `spec.env` + `AA_RUN_ID`, stdin `/dev/null` unless bytes are given, stdout/stderr piped;
//! 3. arms a [`GroupGuard`] that SIGKILLs the whole group if the future is dropped;
//! 4. drains stdout into a bounded buffer (overflow counted, never retained) and stderr into a head+tail excerpt;
//! 5. on the deadline: SIGTERM(group) → `term_grace` → SIGKILL(group);
//! 6. after the leader exits, checks the group for survivors, terminates them, and reports `CleanupStatus`.

use crate::confine::{Confiner, classify_denial};
use aa_core::{
    AaError, AaResult,
    api::{ProcessRunner, RunOutput, RunSpec},
    pb,
};
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup, ResetSigmask};
use std::{
    collections::VecDeque,
    os::unix::process::ExitStatusExt,
    process::Stdio,
    sync::{Arc, LazyLock, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

pub struct SupervisorConfig {
    pub confiner: std::sync::Arc<dyn Confiner>,
    /// 250 ms by default.
    pub term_grace: std::time::Duration,
}

/// Default grace between SIGTERM and SIGKILL of the owned group.
pub const DEFAULT_TERM_GRACE: Duration = Duration::from_millis(250);
/// How long survivors and pipe holders get after the leader is gone.
const SURVIVOR_POLL: Duration = Duration::from_secs(1);
const PIPE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_DENIALS: usize = 32;
const MAX_LINE: usize = 4096;

impl Default for SupervisorConfig {
    /// `default_confiner()` and a 250 ms grace.
    fn default() -> Self {
        Self {
            confiner: crate::confine::default_confiner(),
            term_grace: DEFAULT_TERM_GRACE,
        }
    }
}

pub struct Supervisor {
    cfg: SupervisorConfig,
    confined: OnceLock<bool>,
}

impl Supervisor {
    pub fn new(cfg: SupervisorConfig) -> Self {
        Self {
            cfg,
            confined: OnceLock::new(),
        }
    }

    fn confinement_available(&self) -> bool {
        *self.confined.get_or_init(|| self.cfg.confiner.available())
    }
}

/// SIGKILLs the owned group when dropped while armed (the "client dropped the RPC" path).
struct GroupGuard {
    pgid: Pid,
    armed: bool,
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = killpg(self.pgid, Signal::SIGKILL);
        }
    }
}

/// Aborts the reader tasks when the run future is dropped.
struct AbortOnDrop(Vec<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for h in &self.0 {
            h.abort();
        }
    }
}

#[derive(Default)]
struct StdoutCapture {
    kept: Vec<u8>,
    total: u64,
}

#[derive(Default)]
struct StderrCapture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
    line: Vec<u8>,
    denials: Vec<String>,
}

impl StderrCapture {
    fn push(&mut self, chunk: &[u8], cap: usize, confined: bool) {
        self.total += chunk.len() as u64;
        let head_cap = cap / 2;
        let tail_cap = cap - head_cap;
        let mut rest = chunk;
        if self.head.len() < head_cap {
            let n = (head_cap - self.head.len()).min(rest.len());
            self.head.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
        }
        for &b in rest {
            if tail_cap == 0 {
                break;
            }
            if self.tail.len() == tail_cap {
                self.tail.pop_front();
            }
            self.tail.push_back(b);
        }
        for &b in chunk {
            if b == b'\n' {
                self.finish_line(confined);
            } else if self.line.len() < MAX_LINE {
                self.line.push(b);
            }
        }
    }

    fn finish_line(&mut self, confined: bool) {
        if !self.line.is_empty() {
            let line = String::from_utf8_lossy(&self.line).into_owned();
            if let Some(d) = classify_denial(&line, confined)
                && self.denials.len() < MAX_DENIALS
                && !self.denials.contains(&d)
            {
                self.denials.push(d);
            }
        }
        self.line.clear();
    }

    fn excerpt(&self, cap: usize) -> String {
        if cap == 0 {
            return String::new();
        }
        if self.total as usize <= self.head.len() + self.tail.len() {
            let mut all = self.head.clone();
            all.extend(self.tail.iter());
            redact(&String::from_utf8_lossy(&all))
        } else {
            let head = String::from_utf8_lossy(&self.head).into_owned();
            let tail = String::from_utf8_lossy(&self.tail.iter().copied().collect::<Vec<_>>()).into_owned();
            let elided = self.total - (self.head.len() + self.tail.len()) as u64;
            format!(
                "{}\n…[{elided} bytes elided]…\n{}",
                redact(&redact_cut_end(&head)),
                redact(&redact_cut_start(&tail))
            )
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Token redaction for stderr excerpts (REA `ProviderDiagnosticRedaction`-style), including truncated
/// prefixes of well-known token formats.
static REDACTIONS: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(-----END [A-Z ]*PRIVATE KEY-----|\z)", "[REDACTED PRIVATE KEY]"),
        (r"\b(gh[pousr]_|github_pat_)[A-Za-z0-9_]*", "${1}[REDACTED]"),
        (r"\b(sk-|sk_live_|sk_test_|rk_live_)[A-Za-z0-9_\-]{3,}", "${1}[REDACTED]"),
        (r"\b(xox[abprs]-)[A-Za-z0-9\-]*", "${1}[REDACTED]"),
        (r"\b(AKIA|ASIA)[0-9A-Z]{4,}", "${1}[REDACTED]"),
        (r"\b(glpat-|npm_|pypi-)[A-Za-z0-9_\-]{3,}", "${1}[REDACTED]"),
        (r"\beyJ[A-Za-z0-9_\-]{5,}(\.[A-Za-z0-9_\-]*){0,2}", "[REDACTED JWT]"),
        (r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=\-]{6,}", "${1} [REDACTED]"),
        (
            r#"(?i)\b(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|client[_-]?secret|authorization)(["']?\s*[=:]\s*["']?)[^\s"',;]+"#,
            "${1}${2}[REDACTED]",
        ),
        (r"(://)[^/\s:@]+:[^/\s@]+@", "${1}[REDACTED]@"),
    ]
    .into_iter()
    .filter_map(|(re, rep)| regex::Regex::new(re).ok().map(|r| (r, rep)))
    .collect()
});

pub(crate) fn redact(s: &str) -> String {
    let mut out = s.to_string();
    for (re, rep) in REDACTIONS.iter() {
        if re.is_match(&out) {
            out = re.replace_all(&out, *rep).into_owned();
        }
    }
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.')
}

/// The excerpt cut may split a secret: a long token-like run touching the cut is redacted.
fn redact_cut_end(head: &str) -> String {
    let run = head.chars().rev().take_while(|c| is_token_char(*c)).count();
    if run >= 12 {
        let keep: String = head.chars().take(head.chars().count() - run).collect();
        format!("{keep}[REDACTED-CUT]")
    } else {
        head.to_string()
    }
}

fn redact_cut_start(tail: &str) -> String {
    let run = tail.chars().take_while(|c| is_token_char(*c)).count();
    if run >= 12 {
        let rest: String = tail.chars().skip(run).collect();
        format!("[REDACTED-CUT]{rest}")
    } else {
        tail.to_string()
    }
}

async fn read_stdout(mut r: impl AsyncRead + Unpin, cap: u64, sink: Arc<Mutex<StdoutCapture>>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut s = lock(&sink);
                s.total += n as u64;
                let room = cap.saturating_sub(s.kept.len() as u64) as usize;
                let take = room.min(n);
                s.kept.extend_from_slice(&buf[..take]);
            }
        }
    }
}

async fn read_stderr(mut r: impl AsyncRead + Unpin, cap: usize, confined: bool, sink: Arc<Mutex<StderrCapture>>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => lock(&sink).push(&buf[..n], cap, confined),
        }
    }
    lock(&sink).finish_line(confined);
}

/// Group state after the leader is gone.
enum GroupState {
    Gone,
    Alive,
    /// `EPERM`: only zombies (macOS) or members we may not signal.
    Unsignalable,
}

fn group_state(pgid: Pid) -> GroupState {
    match killpg(pgid, None) {
        Ok(()) => GroupState::Alive,
        Err(Errno::ESRCH) => GroupState::Gone,
        Err(_) => GroupState::Unsignalable,
    }
}

async fn poll_gone(pgid: Pid, within: Duration) -> GroupState {
    let until = Instant::now() + within;
    loop {
        let st = group_state(pgid);
        if matches!(st, GroupState::Gone) || Instant::now() >= until {
            return st;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn validate(spec: &RunSpec) -> AaResult<()> {
    let Some(exe) = spec.argv.first() else {
        return Err(AaError::invalid("empty argv"));
    };
    if !std::path::Path::new(exe).is_absolute() {
        return Err(AaError::invalid(format!(
            "argv[0] must be an absolute realpath, got {exe:?}"
        )));
    }
    if spec.argv.iter().any(|a| a.contains('\0')) {
        return Err(AaError::invalid("argv contains a NUL byte"));
    }
    for (k, v) in &spec.env {
        if k.is_empty() || k.contains('=') || k.contains('\0') || v.contains('\0') {
            return Err(AaError::invalid(format!("invalid environment entry {k:?}")));
        }
    }
    if spec.run_id.contains('\0') {
        return Err(AaError::invalid("run_id contains a NUL byte"));
    }
    Ok(())
}

/// Default PATH for children when the caller does not supply one.
pub const DEFAULT_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Own process group, env_clear + spec.env, AA_RUN_ID last, stdin /dev/null, deadline -> SIGTERM(group) ->
/// grace -> SIGKILL(group), bounded stdout (drain overflow), head+tail stderr excerpt with token redaction,
/// survivor check => CleanupStatus, Drop guard kills group.
#[async_trait::async_trait]
impl ProcessRunner for Supervisor {
    async fn run(&self, spec: RunSpec) -> AaResult<RunOutput> {
        validate(&spec)?;
        let start = Instant::now();
        let deadline_at = tokio::time::Instant::now() + spec.deadline;

        // Confinement: wrap or refuse. The scratch dir holds the profile for the duration of the run.
        let mut _scratch: Option<tempfile::TempDir> = None;
        let (argv, isolation) = match &spec.confinement {
            Some(c) => {
                if !self.confinement_available() {
                    return Err(AaError::new(
                        pb::ErrorCode::IsolationNotProven,
                        "the run requires confinement, but no confiner is available on this host",
                        "check `aa status` (confinement_available); the capability needs ISOLATION_CONFINED",
                    ));
                }
                let dir = tempfile::Builder::new()
                    .prefix("aa-confine-")
                    .tempdir()
                    .map_err(|e| AaError::internal(format!("cannot create confinement scratch dir: {e}")))?;
                let wrapped = self.cfg.confiner.wrap(spec.argv.clone(), c, dir.path())?;
                _scratch = Some(dir);
                (wrapped, pb::Isolation::Confined)
            }
            None => (spec.argv.clone(), pb::Isolation::Process),
        };
        let confined = isolation == pb::Isolation::Confined;

        let mut cmd = CommandWrap::with_new(&argv[0], |c| {
            c.args(&argv[1..]).current_dir(&spec.cwd).env_clear();
            for (k, v) in &spec.env {
                if k != "AA_RUN_ID" {
                    c.env(k, v);
                }
            }
            // Exported last: it always overrides anything in spec.env.
            c.env("AA_RUN_ID", &spec.run_id);
            c.stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
            c.stdout(Stdio::piped());
            c.stderr(Stdio::piped());
        });
        cmd.wrap(ProcessGroup::leader()).wrap(KillOnDrop).wrap(ResetSigmask);
        let mut child = cmd.spawn().map_err(|e| {
            AaError::new(
                pb::ErrorCode::CapabilityUnavailable,
                format!("cannot spawn {:?} in {}: {e}", spec.argv[0], spec.cwd.display()),
                "check the pinned executable and re-run discovery: aa discover <target>",
            )
        })?;
        let pid = child
            .id()
            .and_then(|p| i32::try_from(p).ok())
            .ok_or_else(|| AaError::internal("spawned child has no pid"))?;
        let pgid = Pid::from_raw(pid);
        let mut guard = GroupGuard { pgid, armed: true };

        let out_sink = Arc::new(Mutex::new(StdoutCapture::default()));
        let err_sink = Arc::new(Mutex::new(StderrCapture::default()));
        let mut tasks = Vec::new();
        if let Some(stdout) = child.stdout().take() {
            tasks.push(tokio::spawn(read_stdout(
                stdout,
                spec.max_stdout,
                Arc::clone(&out_sink),
            )));
        }
        if let Some(stderr) = child.stderr().take() {
            tasks.push(tokio::spawn(read_stderr(
                stderr,
                spec.max_stderr_excerpt,
                confined,
                Arc::clone(&err_sink),
            )));
        }
        if let (Some(bytes), Some(mut stdin)) = (spec.stdin.clone(), child.stdin().take()) {
            tasks.push(tokio::spawn(async move {
                let _ = stdin.write_all(&bytes).await;
                let _ = stdin.shutdown().await;
            }));
        }
        let mut readers = AbortOnDrop(tasks);

        let mut timed_out = false;
        let status = tokio::select! {
            s = child.wait() => Some(s),
            () = tokio::time::sleep_until(deadline_at) => None,
        };
        let status = match status {
            Some(s) => s,
            None => {
                timed_out = true;
                let _ = killpg(pgid, Signal::SIGTERM);
                let grace = tokio::time::sleep(self.cfg.term_grace);
                tokio::select! {
                    s = child.wait() => s,
                    () = grace => {
                        let _ = killpg(pgid, Signal::SIGKILL);
                        child.wait().await
                    }
                }
            }
        }
        .map_err(|e| AaError::internal(format!("waiting for the child failed: {e}")))?;

        // Survivor check of the owned group (the leader is reaped by now).
        let cleanup = match group_state(pgid) {
            GroupState::Gone => pb::CleanupStatus::Complete,
            _ => {
                let _ = killpg(pgid, Signal::SIGTERM);
                let st = match poll_gone(pgid, self.cfg.term_grace).await {
                    GroupState::Gone => GroupState::Gone,
                    _ => {
                        let _ = killpg(pgid, Signal::SIGKILL);
                        poll_gone(pgid, SURVIVOR_POLL).await
                    }
                };
                match st {
                    GroupState::Gone => pb::CleanupStatus::Complete,
                    GroupState::Alive => pb::CleanupStatus::Incomplete,
                    GroupState::Unsignalable => pb::CleanupStatus::Unverified,
                }
            }
        };
        if cleanup == pb::CleanupStatus::Complete {
            // The group no longer exists: never signal that pgid again (it may be reused).
            guard.armed = false;
        }

        // Finish draining; a process outside the group (setsid) may still hold a pipe.
        let drain = async {
            for t in readers.0.iter_mut() {
                let _ = t.await;
            }
        };
        let _ = tokio::time::timeout(PIPE_DRAIN_TIMEOUT, drain).await;
        drop(readers);

        let (stdout, stdout_total) = {
            let mut s = lock(&out_sink);
            (std::mem::take(&mut s.kept), s.total)
        };
        let (stderr_excerpt, stderr_total, mut confinement_denials) = {
            let s = lock(&err_sink);
            (s.excerpt(spec.max_stderr_excerpt), s.total, s.denials.clone())
        };
        let exit_code = status.code();
        let signal = status.signal();
        if confined && exit_code == Some(71) && confinement_denials.is_empty() {
            // sandbox-exec's own failure exit (EX_OSERR), e.g. a denied exec of argv[0].
            confinement_denials.push(format!("process-exec:{}", spec.argv[0]));
        }
        Ok(RunOutput {
            exit_code,
            signal,
            stdout_truncated: stdout_total > spec.max_stdout,
            stdout,
            stdout_total,
            stderr_excerpt,
            stderr_total,
            wall: start.elapsed(),
            timed_out,
            cleanup,
            confinement_denials,
            isolation,
        })
    }

    fn max_isolation(&self) -> pb::Isolation {
        if self.confinement_available() {
            pb::Isolation::Confined
        } else {
            pb::Isolation::Process
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec(argv: &[&str]) -> RunSpec {
        RunSpec {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: BTreeMap::from([("PATH".to_string(), DEFAULT_PATH.to_string())]),
            stdin: None,
            deadline: Duration::from_secs(10),
            max_stdout: 1024,
            max_stderr_excerpt: 256,
            confinement: None,
            run_id: "run-test".into(),
        }
    }

    fn sup() -> Supervisor {
        Supervisor::new(SupervisorConfig {
            confiner: Arc::new(crate::confine::NoConfiner),
            term_grace: DEFAULT_TERM_GRACE,
        })
    }

    #[test]
    fn redaction() {
        let s = redact(
            "auth ghp_abcdef123 token=supersecret Authorization: Bearer abcdefghijkl https://u:p@host/x sk-ant-api03-xyz",
        );
        assert!(!s.contains("abcdef123"), "{s}");
        assert!(!s.contains("supersecret"), "{s}");
        assert!(!s.contains("abcdefghijkl"), "{s}");
        assert!(!s.contains("u:p@"), "{s}");
        assert!(!s.contains("api03"), "{s}");
        assert!(s.contains("ghp_[REDACTED]"));
        // Truncated prefix still redacted.
        assert_eq!(redact("x ghp_"), "x ghp_[REDACTED]");
        assert_eq!(redact("plain text"), "plain text");
    }

    #[test]
    fn stderr_head_tail() {
        let mut c = StderrCapture::default();
        let data: Vec<u8> = (0..1000).map(|i| b'a' + (i % 7) as u8).chain(*b" end\n").collect();
        c.push(&data, 100, false);
        assert_eq!(c.total, 1005);
        let ex = c.excerpt(100);
        assert!(ex.contains("bytes elided"), "{ex}");
        assert!(ex.ends_with("end\n") || ex.contains("end"), "{ex}");
        let mut small = StderrCapture::default();
        small.push(b"short\n", 100, false);
        assert_eq!(small.excerpt(100), "short\n");
        assert_eq!(small.excerpt(0), "");
    }

    #[tokio::test]
    async fn env_is_scrubbed_and_run_id_last() {
        let mut s = spec(&["/usr/bin/env"]);
        s.env.insert("AA_RUN_ID".into(), "spoofed".into());
        s.env.insert("FOO".into(), "bar".into());
        // Nothing from the test process environment may leak into the child.
        let out = sup().run(s).await.unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        lines.sort();
        assert_eq!(
            lines,
            vec!["AA_RUN_ID=run-test", "FOO=bar", &format!("PATH={DEFAULT_PATH}")]
        );
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.cleanup, pb::CleanupStatus::Complete);
        assert_eq!(out.isolation, pb::Isolation::Process);
    }

    #[tokio::test]
    async fn stdin_default_null_and_bytes() {
        let out = sup().run(spec(&["/bin/cat"])).await.unwrap();
        assert_eq!(out.stdout, b"");
        let mut s = spec(&["/bin/cat"]);
        s.stdin = Some(b"hello".to_vec());
        let out = sup().run(s).await.unwrap();
        assert_eq!(out.stdout, b"hello");
    }

    #[tokio::test]
    async fn rejects_relative_and_nul_and_missing() {
        assert_eq!(
            sup().run(spec(&["ls"])).await.unwrap_err().code,
            pb::ErrorCode::InvalidRequest
        );
        assert_eq!(
            sup().run(spec(&["/bin/echo", "a\0b"])).await.unwrap_err().code,
            pb::ErrorCode::InvalidRequest
        );
        assert_eq!(
            sup().run(spec(&["/nonexistent/aa-bin"])).await.unwrap_err().code,
            pb::ErrorCode::CapabilityUnavailable
        );
    }

    #[tokio::test]
    async fn confinement_requested_but_unavailable_is_refused() {
        let mut s = spec(&["/usr/bin/true"]);
        s.confinement = Some(aa_core::api::ConfinementSpec::default());
        let e = sup().run(s).await.unwrap_err();
        assert_eq!(e.code, pb::ErrorCode::IsolationNotProven);
        assert_eq!(sup().max_isolation(), pb::Isolation::Process);
    }

    #[tokio::test]
    async fn exit_code_signal_and_timeout() {
        let out = sup().run(spec(&["/usr/bin/false"])).await.unwrap();
        assert_eq!(out.exit_code, Some(1));
        let mut s = spec(&["/bin/sleep", "30"]);
        s.deadline = Duration::from_millis(100);
        let t = Instant::now();
        let out = sup().run(s).await.unwrap();
        assert!(out.timed_out);
        assert_eq!(out.signal, Some(Signal::SIGTERM as i32));
        assert!(t.elapsed() < Duration::from_secs(3));
        assert_eq!(out.cleanup, pb::CleanupStatus::Complete);
    }
}
