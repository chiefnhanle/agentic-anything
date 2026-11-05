//! L3 sandbox-diff on real git (build-plan §3.2 acceptance 5).
//!
//! - plain `git status` after an mtime bump rewrites `.git/index` → `SIDE_EFFECT`, `repo/.git/index MODIFIED`;
//! - with `--no-optional-locks` → no diff, FALSE signals (at CONFINED when Seatbelt is available).

mod common;

use aa_core::{api::ProcessRunner, pb};
use aa_exec::{CliArgvBinding, lanes};
use std::sync::Arc;

fn signal<'a>(rec: &'a pb::VerificationRecord, field: &str) -> Option<&'a pb::EffectSignal> {
    rec.effect_signals.iter().find(|s| s.field == field)
}

async fn l3(no_optional_locks: bool) -> Option<(pb::VerificationRecord, pb::Isolation)> {
    let Some(git) = common::git_path() else {
        eprintln!("SKIPPED: git not on PATH (does not count as a pass)");
        return None;
    };
    let runner = Arc::new(common::supervisor());
    let binding = CliArgvBinding::new(runner.clone());
    let cap = common::git_status(no_optional_locks, "bumped-mtime");
    let lock = common::git_lock(&git, vec![cap.clone()]);
    let scratch = tempfile::tempdir().unwrap();
    let ctx = lanes::LaneCtx {
        lock: &lock,
        binding: &binding,
        runner: runner.as_ref(),
        scratch_parent: scratch.path(),
        engine_version: "test",
    };
    let rec = lanes::run_sandbox_diff_lane(&ctx, &cap).await;
    // Every private root is gone afterwards.
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
    Some((rec, runner.max_isolation()))
}

#[tokio::test]
async fn plain_git_status_is_caught_rewriting_the_index() {
    let Some((rec, iso)) = l3(false).await else { return };
    assert_eq!(rec.lane, pb::Lane::SandboxDiff as i32);
    assert_eq!(rec.status, pb::LaneStatus::Failed as i32, "{rec:#?}");
    assert!(
        rec.findings
            .iter()
            .any(|f| f.reason == pb::WithheldReason::SideEffect as i32 && f.hint.contains(".git/index")),
        "{:#?}",
        rec.findings
    );
    let idx = rec
        .observed_files
        .iter()
        .find(|o| o.path == "repo/.git/index")
        .expect("repo/.git/index observed");
    assert_eq!(idx.change, pb::FileChange::Modified as i32);
    assert_eq!(idx.root, "private:cwd");
    let wf = signal(&rec, "writes_filesystem").unwrap();
    assert_eq!(wf.value, pb::Tri::True as i32);
    assert_eq!(wf.confidence, pb::Confidence::Observed as i32);
    assert_eq!(wf.source, format!("verify:{}", rec.record_id));
    assert!(signal(&rec, "mutates_target").is_none(), "no state handle changed");
    assert_eq!(rec.isolation, iso as i32);
    assert!(rec.non_claims.iter().any(|n| n == lanes::NON_CLAIM_REMOTE));
}

#[tokio::test]
async fn git_status_without_optional_locks_passes() {
    let Some((rec, iso)) = l3(true).await else { return };
    assert_eq!(rec.status, pb::LaneStatus::Passed as i32, "{rec:#?}");
    assert!(rec.observed_files.is_empty(), "{:#?}", rec.observed_files);
    assert!(rec.findings.is_empty());
    for field in ["mutates_target", "writes_filesystem"] {
        let s = signal(&rec, field).unwrap();
        assert_eq!(s.value, pb::Tri::False as i32);
        assert_eq!(s.isolation, iso as i32);
    }
    assert_eq!(rec.capability_id, "git.status");
    assert_eq!(rec.capability_digest, "digest-status");
    assert_eq!(rec.lock_digest, "lock-digest");
    assert!(!rec.identity_digest.is_empty());
    assert!(rec.record_id.starts_with("vr_"));
    assert!(rec.ran_at.is_some());
    if iso == pb::Isolation::Confined {
        assert_eq!(rec.isolation, pb::Isolation::Confined as i32);
        for field in ["accesses_network", "launches_process"] {
            let s = signal(&rec, field).unwrap();
            assert_eq!(s.value, pb::Tri::False as i32);
            assert_eq!(s.isolation, pb::Isolation::Confined as i32);
        }
        assert!(!rec.non_claims.iter().any(|n| n == lanes::NON_CLAIM_PROCESS));
    } else {
        eprintln!("NOTE: Seatbelt unavailable; L3 ran at ISOLATION_PROCESS");
        assert!(signal(&rec, "accesses_network").is_none());
        assert!(signal(&rec, "launches_process").is_none());
        assert!(rec.non_claims.iter().any(|n| n == lanes::NON_CLAIM_PROCESS));
    }
}

#[tokio::test]
async fn fewer_than_two_examples_is_skipped() {
    let Some(git) = common::git_path() else { return };
    let runner = Arc::new(common::supervisor());
    let binding = CliArgvBinding::new(runner.clone());
    let mut cap = common::git_status(true, "one-commit-repo");
    cap.examples.truncate(1);
    let lock = common::git_lock(&git, vec![cap.clone()]);
    let scratch = tempfile::tempdir().unwrap();
    let ctx = lanes::LaneCtx {
        lock: &lock,
        binding: &binding,
        runner: runner.as_ref(),
        scratch_parent: scratch.path(),
        engine_version: "test",
    };
    let rec = lanes::run_sandbox_diff_lane(&ctx, &cap).await;
    assert_eq!(rec.status, pb::LaneStatus::Skipped as i32);
    assert_eq!(rec.skip_reason, "needs ≥2 examples");
}
