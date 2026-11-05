//! Seatbelt confinement acceptance tests (build-plan §3.2 acceptance 2). On a host without a working
//! `sandbox-exec` each test prints a skip reason and returns: a skip never counts as coverage.

use aa_core::{
    api::{ConfinementSpec, ProcessRunner, RunSpec},
    pb,
};
use aa_exec::{Confiner, PrivateRoot, SeatbeltConfiner, Supervisor, SupervisorConfig};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn confined_supervisor() -> Option<Supervisor> {
    if !SeatbeltConfiner.available() {
        eprintln!("SKIPPED: /usr/bin/sandbox-exec is absent or failed its self-test (does not count as a pass)");
        return None;
    }
    Some(Supervisor::new(SupervisorConfig {
        confiner: Arc::new(SeatbeltConfiner),
        term_grace: Duration::from_millis(250),
    }))
}

fn spec(argv: &[&str], root: &PrivateRoot, confinement: ConfinementSpec) -> RunSpec {
    RunSpec {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: root.cwd.clone(),
        env: root.base_env("/usr/bin:/bin"),
        stdin: None,
        deadline: Duration::from_secs(20),
        max_stdout: 1024 * 1024,
        max_stderr_excerpt: 4096,
        confinement: Some(confinement),
        run_id: "act_confine".into(),
    }
}

fn realpath(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

#[tokio::test]
async fn write_outside_the_root_is_denied_and_reported() {
    let Some(sup) = confined_supervisor() else { return };
    assert_eq!(sup.max_isolation(), pb::Isolation::Confined);
    let parent = tempfile::tempdir().unwrap();
    let root = PrivateRoot::create(parent.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = std::fs::canonicalize(outside.path()).unwrap().join("escaped");
    let c = ConfinementSpec {
        deny_network: true,
        writable: vec![root.base.clone()],
        exec_allow: vec![realpath("/usr/bin/touch")],
    };
    let out = sup
        .run(spec(&["/usr/bin/touch", target.to_str().unwrap()], &root, c.clone()))
        .await
        .unwrap();
    assert_ne!(out.exit_code, Some(0));
    assert!(!target.exists(), "the write escaped the root");
    assert!(
        out.stderr_excerpt.contains("Operation not permitted"),
        "{}",
        out.stderr_excerpt
    );
    assert!(!out.confinement_denials.is_empty());
    assert_eq!(out.isolation, pb::Isolation::Confined);

    // The same tool may write inside the root.
    let inside = root.cwd.join("ok");
    let out = sup
        .run(spec(&["/usr/bin/touch", inside.to_str().unwrap()], &root, c))
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "{}", out.stderr_excerpt);
    assert!(inside.exists());
    assert!(out.confinement_denials.is_empty());
}

#[tokio::test]
async fn network_is_denied() {
    let Some(sup) = confined_supervisor() else { return };
    if !std::path::Path::new("/usr/bin/curl").exists() {
        eprintln!("SKIPPED: /usr/bin/curl absent (does not count as a pass)");
        return;
    }
    let parent = tempfile::tempdir().unwrap();
    let root = PrivateRoot::create(parent.path()).unwrap();
    let c = ConfinementSpec {
        deny_network: true,
        writable: vec![root.base.clone()],
        exec_allow: vec![realpath("/usr/bin/curl")],
    };
    let out = sup
        .run(spec(
            &[
                "/usr/bin/curl",
                "-sS",
                "--max-time",
                "5",
                "-o",
                "/dev/null",
                "http://1.1.1.1/",
            ],
            &root,
            c,
        ))
        .await
        .unwrap();
    assert_ne!(out.exit_code, Some(0), "curl reached the network under deny_network");
}

#[tokio::test]
async fn exec_not_on_the_allow_list_is_denied() {
    let Some(sup) = confined_supervisor() else { return };
    let parent = tempfile::tempdir().unwrap();
    let root = PrivateRoot::create(parent.path()).unwrap();
    let c = ConfinementSpec {
        deny_network: true,
        writable: vec![root.base.clone()],
        exec_allow: vec![realpath("/usr/bin/touch")],
    };
    let out = sup
        .run(spec(&["/bin/sh", "-c", "echo should-not-run"], &root, c))
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(71), "{}", out.stderr_excerpt);
    assert!(out.stdout.is_empty());
    assert!(
        out.confinement_denials.iter().any(|d| d == "process-exec:/bin/sh"),
        "{:?}",
        out.confinement_denials
    );
}

#[tokio::test]
async fn git_status_without_optional_locks_works_confined() {
    let Some(sup) = confined_supervisor() else { return };
    let Ok(git) = which::which("git") else {
        eprintln!("SKIPPED: git not on PATH (does not count as a pass)");
        return;
    };
    let git = std::fs::canonicalize(git).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let root = PrivateRoot::create(parent.path()).unwrap();
    let mut env = root.base_env("/usr/bin:/bin");
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    let run = |args: &[&str], cwd: PathBuf, confinement: Option<ConfinementSpec>| {
        let mut argv = vec![git.to_string_lossy().into_owned()];
        argv.extend(args.iter().map(|s| s.to_string()));
        RunSpec {
            argv,
            cwd,
            env: env.clone(),
            stdin: None,
            deadline: Duration::from_secs(20),
            max_stdout: 1024 * 1024,
            max_stderr_excerpt: 4096,
            confinement,
            run_id: "act_git".into(),
        }
    };
    let plain = Supervisor::new(SupervisorConfig {
        confiner: Arc::new(aa_exec::NoConfiner),
        term_grace: Duration::from_millis(250),
    });
    let repo = root.cwd.join("repo");
    for args in [
        &["init", "-q", "repo"][..],
        &[
            "-C",
            "repo",
            "-c",
            "user.name=aa",
            "-c",
            "user.email=aa@localhost",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    ] {
        let out = plain.run(run(args, root.cwd.clone(), None)).await.unwrap();
        assert_eq!(out.exit_code, Some(0), "{}", out.stderr_excerpt);
    }
    let c = ConfinementSpec {
        deny_network: true,
        writable: vec![root.base.clone()],
        exec_allow: vec![git.clone()],
    };
    let out = sup
        .run(run(
            &["--no-optional-locks", "status", "--porcelain=v2", "--branch"],
            repo,
            Some(c),
        ))
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "{}", out.stderr_excerpt);
    assert!(String::from_utf8_lossy(&out.stdout).contains("# branch.head"));
    assert!(out.confinement_denials.is_empty(), "{:?}", out.confinement_denials);
    assert_eq!(out.isolation, pb::Isolation::Confined);
}
