//! Shared helpers for the lane tests: a hand-built `git` lock shaped like architecture §7.3.

#![allow(dead_code)]

use aa_core::pb;
use aa_exec::{Supervisor, SupervisorConfig};
use std::path::PathBuf;

pub fn git_path() -> Option<PathBuf> {
    which::which("git").ok().and_then(|p| std::fs::canonicalize(p).ok())
}

pub fn supervisor() -> Supervisor {
    Supervisor::new(SupervisorConfig::default())
}

fn argv_step(argv: &[&str]) -> pb::FixtureStep {
    pb::FixtureStep {
        step: Some(pb::fixture_step::Step::Argv(pb::ArgvStep {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: String::new(),
        })),
    }
}

fn file_step(path: &str, content: &str) -> pb::FixtureStep {
    pb::FixtureStep {
        step: Some(pb::fixture_step::Step::File(pb::FileStep {
            path: path.into(),
            content: content.as_bytes().to_vec(),
        })),
    }
}

fn include(id: &str) -> pb::FixtureStep {
    pb::FixtureStep {
        step: Some(pb::fixture_step::Step::IncludeFixture(id.into())),
    }
}

pub fn fixtures() -> Vec<pb::Fixture> {
    vec![
        pb::Fixture {
            id: "one-commit-repo".into(),
            steps: vec![
                argv_step(&["git", "init", "-q", "repo"]),
                file_step("repo/README", "hello\n"),
                argv_step(&["git", "-C", "repo", "add", "README"]),
                argv_step(&[
                    "git",
                    "-C",
                    "repo",
                    "-c",
                    "user.name=aa",
                    "-c",
                    "user.email=aa@localhost",
                    "commit",
                    "-qm",
                    "init",
                ]),
                argv_step(&["git", "-C", "repo", "branch", "feature-x"]),
            ],
            ready_capability_id: "git.status".into(),
        },
        // The tracked file's mtime moves after the commit, so a plain `git status` refreshes `.git/index`.
        pb::Fixture {
            id: "bumped-mtime".into(),
            steps: vec![
                include("one-commit-repo"),
                argv_step(&["touch", "-m", "-t", "201001010000", "repo/README"]),
            ],
            ready_capability_id: String::new(),
        },
        pb::Fixture {
            id: "cycle-a".into(),
            steps: vec![include("cycle-b")],
            ready_capability_id: String::new(),
        },
        pb::Fixture {
            id: "cycle-b".into(),
            steps: vec![include("cycle-a")],
            ready_capability_id: String::new(),
        },
    ]
}

fn opt(pointer: &str, flag: &str) -> pb::ArgMapping {
    pb::ArgMapping {
        pointer: pointer.into(),
        style: pb::ArgStyle::Option as i32,
        flag: flag.into(),
        ..Default::default()
    }
}

pub fn example(input: &str, fixture: &str) -> pb::Example {
    pb::Example {
        input_json: input.into(),
        fixture_id: fixture.into(),
        source: "overlay".into(),
    }
}

/// `git.status`, with or without `--no-optional-locks`.
pub fn git_status(no_optional_locks: bool, fixture: &str) -> pb::Capability {
    let mut global_args: Vec<String> = ["-c", "core.pager=cat", "-c", "color.ui=never"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if no_optional_locks {
        global_args.insert(0, "--no-optional-locks".into());
    }
    let binding = pb::CliArgvBinding {
        executable_role: "executable".into(),
        global_args,
        command_path: vec!["status".into()],
        fixed_args: vec!["--porcelain=v2".into(), "--branch".into()],
        args: vec![opt("/untracked_files", "--untracked-files")],
        cwd_pointer: "/repo".into(),
        output: pb::OutputMode::TextStdout as i32,
        errors: pb::ErrorConvention::ExitCode as i32,
        deadline_ms: 15000,
        max_output_bytes: 8 * 1024 * 1024,
        kill_scope: pb::KillScope::OwnedGroup as i32,
        usage_exit_code: 129,
        env: [("GIT_CONFIG_NOSYSTEM", "1"), ("GIT_TERMINAL_PROMPT", "0")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..Default::default()
    };
    pb::Capability {
        id: "git.status".into(),
        target_id: "git".into(),
        input_schema_json: r#"{"additionalProperties":false,"properties":{"repo":{"format":"path","type":"string"},"untracked_files":{"enum":["no","normal","all"]}},"required":["repo"],"type":"object"}"#.into(),
        binding: Some(pb::Binding {
            kind: Some(pb::binding::Kind::Cli(binding)),
        }),
        examples: vec![
            example(r#"{"repo":{"$root":"/repo"}}"#, fixture),
            example(r#"{"repo":{"$root":"/repo"},"untracked_files":"no"}"#, fixture),
        ],
        capability_digest: "digest-status".into(),
        ..Default::default()
    }
}

/// `git.log` with an integer `max_count` (git's revision parser exits 128 on bad input, not 129).
pub fn git_log() -> pb::Capability {
    let mut cap = git_status(true, "one-commit-repo");
    cap.id = "git.log".into();
    cap.input_schema_json = r#"{"additionalProperties":false,"properties":{"author":{"type":"string"},"max_count":{"type":"integer"},"repo":{"format":"path","type":"string"}},"required":["repo"],"type":"object"}"#.into();
    if let Some(pb::binding::Kind::Cli(b)) = cap.binding.as_mut().and_then(|b| b.kind.as_mut()) {
        b.command_path = vec!["log".into()];
        b.fixed_args = vec!["--format=%H%x09%an%x09%aI%x09%s".into()];
        b.args = vec![opt("/max_count", "--max-count"), opt("/author", "--author")];
    }
    cap.examples = vec![
        example(r#"{"repo":{"$root":"/repo"},"max_count":5}"#, "one-commit-repo"),
        example(
            r#"{"repo":{"$root":"/repo"},"max_count":1,"author":"aa"}"#,
            "one-commit-repo",
        ),
    ];
    cap
}

pub fn git_lock(git: &std::path::Path, caps: Vec<pb::Capability>) -> pb::CapabilityLock {
    let git_id = aa_exec::file_identity("executable", git, Some(&["--version"])).unwrap();
    let touch = aa_exec::file_identity("helper:touch", std::path::Path::new("/usr/bin/touch"), None).unwrap();
    let mut identity = pb::TargetIdentity {
        files: vec![git_id, touch],
        ..Default::default()
    };
    identity.identity_digest = aa_exec::identity_digest(&identity).unwrap();
    pb::CapabilityLock {
        schema: "aa.lock/v1".into(),
        target: Some(pb::Target {
            id: "git".into(),
            identity: Some(identity),
            fixtures: fixtures(),
            ..Default::default()
        }),
        capabilities: caps,
        lock_digest: "lock-digest".into(),
        ..Default::default()
    }
}
