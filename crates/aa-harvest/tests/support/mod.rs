//! Shared helpers for aa-harvest integration tests.
//!
//! `StdRunner` is a test-only `ProcessRunner` over `std::process` (no shell, `env_clear`, stdin `/dev/null`,
//! deadline kill). It ignores confinement: it exists to run the live/record tests, never product code.
#![allow(dead_code)]

use aa_core::{
    AaError, AaResult,
    api::{ProcessRunner, RawHarvest, RunOutput, RunSpec},
    pb,
};
use std::{
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    pin::pin,
    process::{Command, Stdio},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

/// Minimal executor: the harvest futures here never wait on a reactor (FakeRunner / StdRunner are synchronous).
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

pub fn fixture_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

pub fn raw_fixture(harvester: &str, file: &str, authority: pb::DiscoveryAuthority) -> RawHarvest {
    let bytes = std::fs::read(fixture_path(&format!("raw/{file}"))).unwrap_or_else(|e| panic!("fixture {file}: {e}"));
    RawHarvest {
        harvester: harvester.into(),
        name: file.into(),
        media_type: "application/json".into(),
        uri: format!("fixture:{file}"),
        authority: authority as i32,
        bytes,
    }
}

pub fn file(role: &str, realpath: &str) -> pb::FileIdentity {
    pb::FileIdentity {
        role: role.into(),
        realpath: realpath.into(),
        sha256: String::new(),
        version_banner: String::new(),
    }
}

pub fn target(id: &str, kind: pb::TargetKind, files: Vec<pb::FileIdentity>) -> pb::Target {
    pb::Target {
        id: id.into(),
        kind: kind as i32,
        identity: Some(pb::TargetIdentity {
            files,
            ..Default::default()
        }),
        authorization_statement: "Owner workstation; test fixture.".into(),
        ..Default::default()
    }
}

/// Env var value, or `None` after printing why the live test is skipped (a skip never counts as a pass).
pub fn live_env(var: &str, why: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("SKIPPED (not a pass): {var} is not set; {why}");
            None
        }
    }
}

pub struct StdRunner;

#[async_trait::async_trait]
impl ProcessRunner for StdRunner {
    async fn run(&self, spec: RunSpec) -> AaResult<RunOutput> {
        let start = Instant::now();
        let (exe, args) = spec.argv.split_first().ok_or_else(|| AaError::invalid("empty argv"))?;
        let mut child = Command::new(exe)
            .args(args)
            .env_clear()
            .envs(&spec.env)
            .env("AA_RUN_ID", &spec.run_id)
            .current_dir(&spec.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut out_pipe = child.stdout.take().expect("piped stdout");
        let mut err_pipe = child.stderr.take().expect("piped stderr");
        let out_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = out_pipe.read_to_end(&mut b);
            b
        });
        let err_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = err_pipe.read_to_end(&mut b);
            b
        });
        let mut timed_out = false;
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            if start.elapsed() > spec.deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait()?;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut stdout = out_t.join().unwrap_or_default();
        let stderr = err_t.join().unwrap_or_default();
        let total = stdout.len() as u64;
        let truncated = total > spec.max_stdout;
        stdout.truncate(usize::try_from(spec.max_stdout).unwrap_or(usize::MAX));
        let mut excerpt = String::from_utf8_lossy(&stderr).into_owned();
        if excerpt.len() > spec.max_stderr_excerpt {
            let mut cut = spec.max_stderr_excerpt;
            while !excerpt.is_char_boundary(cut) {
                cut -= 1;
            }
            excerpt.truncate(cut);
        }
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal = None;
        Ok(RunOutput {
            exit_code: status.code(),
            signal,
            stdout,
            stdout_total: total,
            stdout_truncated: truncated,
            stderr_excerpt: excerpt,
            stderr_total: stderr.len() as u64,
            wall: start.elapsed(),
            timed_out,
            cleanup: pb::CleanupStatus::Complete,
            confinement_denials: Vec::new(),
            isolation: pb::Isolation::None,
        })
    }

    fn max_isolation(&self) -> pb::Isolation {
        pb::Isolation::None
    }
}

/// Candidate -> stable JSON for snapshots (pbjson proto3-JSON for the capability).
pub fn candidate_json(c: &aa_core::api::Candidate) -> serde_json::Value {
    serde_json::json!({
        "native_path": c.native_path,
        "interactive": c.interactive,
        "native_signals": c.native_signals.len(),
        "capability": serde_json::to_value(&c.capability).expect("pbjson"),
    })
}

/// The `CliArgvBinding` of a candidate.
pub fn cli(c: &aa_core::api::Candidate) -> &pb::CliArgvBinding {
    match c.capability.binding.as_ref().and_then(|b| b.kind.as_ref()) {
        Some(pb::binding::Kind::Cli(b)) => b,
        other => panic!("not a cli binding: {other:?}"),
    }
}

pub fn schema(c: &aa_core::api::Candidate) -> serde_json::Value {
    serde_json::from_str(&c.capability.input_schema_json).expect("schema json")
}
