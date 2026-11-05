//! Supervision torture tests (build-plan §3.2 acceptance 1), driven by the `torture` helper binary.

use aa_core::{
    api::{ProcessRunner, RunSpec},
    pb,
};
use aa_exec::{NoConfiner, Supervisor, SupervisorConfig};
use nix::{
    errno::Errno,
    sys::signal::{kill, killpg},
    unistd::Pid,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const TORTURE: &str = env!("CARGO_BIN_EXE_torture");
const GRACE: Duration = Duration::from_millis(250);

fn sup() -> Supervisor {
    Supervisor::new(SupervisorConfig {
        confiner: Arc::new(NoConfiner),
        term_grace: GRACE,
    })
}

fn spec(args: &[&str], cwd: &Path) -> RunSpec {
    let mut argv = vec![TORTURE.to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    RunSpec {
        argv,
        cwd: cwd.to_path_buf(),
        env: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
        stdin: None,
        deadline: Duration::from_secs(20),
        max_stdout: 1024 * 1024,
        max_stderr_excerpt: 1024,
        confinement: None,
        run_id: "act_torture".into(),
    }
}

/// True once `pid` no longer exists (zombies are reaped by launchd quickly once reparented).
async fn gone_within(pid: i32, within: Duration) -> bool {
    let until = Instant::now() + within;
    loop {
        if kill(Pid::from_raw(pid), None) == Err(Errno::ESRCH) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn read_pids(path: &Path) -> Vec<i32> {
    for _ in 0..200 {
        if let Ok(s) = std::fs::read_to_string(path)
            && !s.trim().is_empty()
        {
            return s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("pid file {} never appeared", path.display());
}

#[tokio::test]
async fn ignores_sigterm_killed_within_deadline_plus_grace() {
    let d = tempfile::tempdir().unwrap();
    let mut s = spec(&["ignore-term", "60"], d.path());
    s.deadline = Duration::from_millis(400);
    let t = Instant::now();
    let out = sup().run(s).await.unwrap();
    let wall = t.elapsed();
    assert!(out.timed_out);
    assert_eq!(out.signal, Some(nix::sys::signal::Signal::SIGKILL as i32), "{out:?}");
    assert_eq!(out.cleanup, pb::CleanupStatus::Complete);
    assert!(
        wall < Duration::from_millis(400) + GRACE + Duration::from_millis(750),
        "took {wall:?}"
    );
    assert!(
        out.stderr_excerpt.contains("ignoring SIGTERM"),
        "{:?}",
        out.stderr_excerpt
    );
}

#[tokio::test]
async fn grandchild_in_the_same_group_is_reaped() {
    let d = tempfile::tempdir().unwrap();
    let pidfile = d.path().join("gc.pid");
    let t = Instant::now();
    let out = sup()
        .run(spec(&["grandchild", pidfile.to_str().unwrap()], d.path()))
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0));
    assert!(!out.timed_out);
    assert_eq!(out.cleanup, pb::CleanupStatus::Complete, "{out:?}");
    assert!(t.elapsed() < Duration::from_secs(5), "the grandchild held the run open");
    let pids = read_pids(&pidfile).await;
    assert!(
        gone_within(pids[0], Duration::from_secs(1)).await,
        "grandchild survived"
    );
}

#[tokio::test]
async fn stdout_flood_is_bounded_and_drained() {
    let d = tempfile::tempdir().unwrap();
    let total: u64 = 100 * 1024 * 1024;
    let out = sup().run(spec(&["flood", &total.to_string()], d.path())).await.unwrap();
    assert_eq!(out.exit_code, Some(0), "the child must not block on a full pipe");
    assert!(out.stdout_truncated);
    assert_eq!(out.stdout_total, total);
    assert_eq!(out.stdout.len(), 1024 * 1024);
    assert!(out.stdout.capacity() <= 2 * 1024 * 1024, "memory not bounded");
}

#[tokio::test]
async fn dropping_the_run_future_kills_the_group() {
    let d = tempfile::tempdir().unwrap();
    let pidfile = d.path().join("tree.pid");
    let runner = sup();
    let fut = runner.run(spec(&["tree", pidfile.to_str().unwrap()], d.path()));
    assert!(tokio::time::timeout(Duration::from_millis(500), fut).await.is_err());
    // The future is dropped here.
    let pids = read_pids(&pidfile).await;
    let (leader, grandchild) = (pids[0], pids[1]);
    let t = Instant::now();
    assert!(
        gone_within(grandchild, Duration::from_secs(1)).await,
        "grandchild survived the drop"
    );
    // The leader may linger as an unreaped zombie of this test process, but nothing in the group is alive.
    loop {
        match killpg(Pid::from_raw(leader), None) {
            Err(_) => break,
            Ok(()) if t.elapsed() < Duration::from_secs(1) => tokio::time::sleep(Duration::from_millis(10)).await,
            Ok(()) => panic!("the process group is still alive 1 s after the drop"),
        }
    }
}

#[tokio::test]
async fn stderr_excerpt_is_redacted() {
    let d = tempfile::tempdir().unwrap();
    let out = sup().run(spec(&["stderr-token"], d.path())).await.unwrap();
    assert_eq!(out.exit_code, Some(3));
    assert!(out.stderr_total > 0);
    assert!(
        !out.stderr_excerpt.contains("abcdefghijklmnop"),
        "{}",
        out.stderr_excerpt
    );
    assert!(out.stderr_excerpt.contains("ghp_[REDACTED]"), "{}", out.stderr_excerpt);
}

#[tokio::test]
async fn plain_deadline_sends_sigterm_first() {
    let d = tempfile::tempdir().unwrap();
    let mut s = spec(&["sleep", "60"], d.path());
    s.deadline = Duration::from_millis(200);
    let out = sup().run(s).await.unwrap();
    assert!(out.timed_out);
    assert_eq!(out.signal, Some(nix::sys::signal::Signal::SIGTERM as i32));
    assert_eq!(out.cleanup, pb::CleanupStatus::Complete);
}
