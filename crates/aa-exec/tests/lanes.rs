//! L1 parser lane (real git and helper parsers), L3 HOME side effects, and fixture building.

mod common;

use aa_core::pb;
use aa_exec::{CliArgvBinding, PrivateRoot, Supervisor, lanes};
use std::sync::Arc;

const TORTURE: &str = env!("CARGO_BIN_EXE_torture");

struct Harness {
    runner: Arc<Supervisor>,
    binding: CliArgvBinding,
    lock: pb::CapabilityLock,
    scratch: tempfile::TempDir,
}

impl Harness {
    fn new(lock: pb::CapabilityLock) -> Self {
        let runner = Arc::new(common::supervisor());
        let binding = CliArgvBinding::new(runner.clone());
        Self {
            runner,
            binding,
            lock,
            scratch: tempfile::tempdir().unwrap(),
        }
    }

    fn ctx(&self) -> lanes::LaneCtx<'_> {
        lanes::LaneCtx {
            lock: &self.lock,
            binding: &self.binding,
            runner: self.runner.as_ref(),
            scratch_parent: self.scratch.path(),
            engine_version: "test",
        }
    }
}

fn git_harness(caps: Vec<pb::Capability>) -> Option<Harness> {
    let Some(git) = common::git_path() else {
        eprintln!("SKIPPED: git not on PATH (does not count as a pass)");
        return None;
    };
    Some(Harness::new(common::git_lock(&git, caps)))
}

/// A lock whose only pinned file is `exe`, with one capability running `exe <command_path…>`.
fn tool_lock(
    exe: &str,
    command_path: &[&str],
    args: Vec<pb::ArgMapping>,
    examples: Vec<pb::Example>,
) -> pb::CapabilityLock {
    let id = aa_exec::file_identity("executable", std::path::Path::new(exe), None).unwrap();
    let mut identity = pb::TargetIdentity {
        files: vec![id],
        ..Default::default()
    };
    identity.identity_digest = aa_exec::identity_digest(&identity).unwrap();
    let cap = pb::Capability {
        id: "tool.cmd".into(),
        target_id: "tool".into(),
        input_schema_json: r#"{"additionalProperties":false,"properties":{"opt":{"type":"string"}},"type":"object"}"#
            .into(),
        binding: Some(pb::Binding {
            kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
                executable_role: "executable".into(),
                command_path: command_path.iter().map(|s| s.to_string()).collect(),
                args,
                output: pb::OutputMode::TextStdout as i32,
                usage_exit_code: 2,
                ..Default::default()
            })),
        }),
        examples,
        ..Default::default()
    };
    pb::CapabilityLock {
        schema: "aa.lock/v1".into(),
        target: Some(pb::Target {
            id: "tool".into(),
            identity: Some(identity),
            fixtures: common::fixtures(),
            ..Default::default()
        }),
        capabilities: vec![cap],
        lock_digest: "lock".into(),
        ..Default::default()
    }
}

fn has_finding(rec: &pb::VerificationRecord, reason: pb::WithheldReason) -> bool {
    rec.findings.iter().any(|f| f.reason == reason as i32)
}

#[tokio::test]
async fn l1_git_status_passes() {
    let cap = common::git_status(true, "one-commit-repo");
    let Some(h) = git_harness(vec![cap.clone()]) else {
        return;
    };
    let rec = lanes::run_parser_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.lane, pb::Lane::Parser as i32);
    assert_eq!(rec.status, pb::LaneStatus::Passed as i32, "{rec:#?}");
    assert!(rec.observed_files.is_empty());
    assert!(!rec.non_claims.is_empty());
    assert!(rec.record_id.starts_with("vr_"));
}

#[tokio::test]
async fn l1_git_log_passes_with_its_revision_parser_exit_code() {
    let cap = common::git_log();
    let Some(h) = git_harness(vec![cap.clone()]) else {
        return;
    };
    let rec = lanes::run_parser_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Passed as i32, "{rec:#?}");
}

#[tokio::test]
async fn l1_flags_a_parser_that_accepts_unknown_flags() {
    let lock = tool_lock("/usr/bin/true", &[], vec![], vec![]);
    let cap = lock.capabilities[0].clone();
    let h = Harness::new(lock);
    let rec = lanes::run_parser_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Failed as i32);
    assert!(has_finding(&rec, pb::WithheldReason::ParserMismatch), "{rec:#?}");
    assert!(rec.findings[0].hint.contains("--aa-probe-unknown"));
}

#[tokio::test]
async fn l1_detects_no_equals_form() {
    let opt = pb::ArgMapping {
        pointer: "/opt".into(),
        style: pb::ArgStyle::Option as i32,
        flag: "--opt".into(),
        ..Default::default()
    };
    let lock = tool_lock(
        TORTURE,
        &["strict-sep"],
        vec![opt],
        vec![common::example(r#"{"opt":"v"}"#, "")],
    );
    let cap = lock.capabilities[0].clone();
    let h = Harness::new(lock);
    let rec = lanes::run_parser_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Failed as i32, "{rec:#?}");
    let f = rec
        .findings
        .iter()
        .find(|f| f.hint.starts_with("no_equals_form:/opt"))
        .unwrap_or_else(|| panic!("{rec:#?}"));
    assert_eq!(f.reason, pb::WithheldReason::ParserMismatch as i32);
    assert!(rec.evidence_ids.contains(&"no_equals_form:/opt".to_string()));
    // The unknown-flag probe itself was rejected.
    assert!(
        !rec.findings
            .iter()
            .any(|f| f.hint.contains("the parser accepted the probe")),
        "{rec:#?}"
    );
}

#[tokio::test]
async fn l1_is_skipped_for_other_bindings() {
    let lock = tool_lock("/usr/bin/true", &[], vec![], vec![]);
    let mut cap = lock.capabilities[0].clone();
    cap.binding = Some(pb::Binding {
        kind: Some(pb::binding::Kind::Mcp(pb::McpToolBinding::default())),
    });
    let h = Harness::new(lock);
    let rec = lanes::run_parser_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Skipped as i32);
}

#[tokio::test]
async fn l3_catches_a_read_that_writes_home() {
    let lock = tool_lock(
        TORTURE,
        &["write-home", ".cache/notes/stats"],
        vec![],
        vec![common::example("{}", ""), common::example("", "")],
    );
    let cap = lock.capabilities[0].clone();
    let h = Harness::new(lock);
    let rec = lanes::run_sandbox_diff_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Failed as i32, "{rec:#?}");
    assert!(has_finding(&rec, pb::WithheldReason::SideEffect));
    assert!(
        rec.observed_files.iter().any(|o| o.root == "private:home"
            && o.path == ".cache/notes/stats"
            && o.change == pb::FileChange::Created as i32),
        "{:#?}",
        rec.observed_files
    );
    // Declaring the effect removes the finding (observed ⊆ declared), but never yields FALSE signals.
    let mut declared = cap.clone();
    declared.effects = Some(pb::Effects {
        writes_filesystem: pb::Tri::True as i32,
        ..Default::default()
    });
    let rec = lanes::run_sandbox_diff_lane(&h.ctx(), &declared).await;
    assert!(!has_finding(&rec, pb::WithheldReason::SideEffect), "{rec:#?}");
    assert_eq!(rec.status, pb::LaneStatus::Passed as i32);
    assert!(
        rec.effect_signals
            .iter()
            .all(|s| s.field != "writes_filesystem" || s.value == pb::Tri::True as i32)
    );
}

#[tokio::test]
async fn l3_reports_a_failing_example() {
    let lock = tool_lock(
        "/usr/bin/false",
        &[],
        vec![],
        vec![common::example("{}", ""), common::example("{}", "")],
    );
    let cap = lock.capabilities[0].clone();
    let h = Harness::new(lock);
    let rec = lanes::run_sandbox_diff_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Failed as i32);
    assert!(has_finding(&rec, pb::WithheldReason::Unconfirmed));
    assert!(
        rec.effect_signals.iter().all(|s| s.value != pb::Tri::False as i32),
        "no FALSE from failed runs"
    );
}

#[tokio::test]
async fn l3_errors_on_a_broken_fixture() {
    let lock = tool_lock(
        "/usr/bin/true",
        &[],
        vec![],
        vec![common::example("{}", "cycle-a"), common::example("{}", "cycle-a")],
    );
    let cap = lock.capabilities[0].clone();
    let h = Harness::new(lock);
    let rec = lanes::run_sandbox_diff_lane(&h.ctx(), &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Error as i32);
    assert!(rec.skip_reason.contains("cycle"), "{}", rec.skip_reason);
}

#[tokio::test]
async fn build_fixture_seeds_a_repo_and_rejects_bad_steps() {
    let cap = common::git_status(true, "one-commit-repo");
    let Some(mut h) = git_harness(vec![cap]) else { return };
    let root = PrivateRoot::create(h.scratch.path()).unwrap();
    lanes::build_fixture(&h.ctx(), "bumped-mtime", &root).await.unwrap();
    assert!(root.cwd.join("repo/.git/HEAD").is_file());
    assert_eq!(std::fs::read(root.cwd.join("repo/README")).unwrap(), b"hello\n");

    let e = lanes::build_fixture(&h.ctx(), "no-such", &root).await.unwrap_err();
    assert_eq!(e.code, pb::ErrorCode::ConfigurationInvalid);
    let e = lanes::build_fixture(&h.ctx(), "cycle-a", &root).await.unwrap_err();
    assert!(e.message.contains("cycle-a -> cycle-b -> cycle-a"), "{}", e.message);

    let target = h.lock.target.as_mut().unwrap();
    target.fixtures.push(pb::Fixture {
        id: "unpinned".into(),
        steps: vec![pb::FixtureStep {
            step: Some(pb::fixture_step::Step::Argv(pb::ArgvStep {
                argv: vec!["sh".into(), "-c".into(), "true".into()],
                cwd: String::new(),
            })),
        }],
        ready_capability_id: String::new(),
    });
    target.fixtures.push(pb::Fixture {
        id: "escape".into(),
        steps: vec![pb::FixtureStep {
            step: Some(pb::fixture_step::Step::File(pb::FileStep {
                path: "../outside".into(),
                content: b"x".to_vec(),
            })),
        }],
        ready_capability_id: String::new(),
    });
    let e = lanes::build_fixture(&h.ctx(), "unpinned", &root).await.unwrap_err();
    assert!(e.message.contains("not a pinned identity file"), "{}", e.message);
    assert!(lanes::build_fixture(&h.ctx(), "escape", &root).await.is_err());
    assert!(!root.base.join("outside").exists());
}
