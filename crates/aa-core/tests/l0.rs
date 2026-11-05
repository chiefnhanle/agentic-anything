//! Acceptance test 4 (build-plan §3.1): the L0 static lane.

#[path = "fixtures/harness.rs"]
mod harness;

use aa_core::lanes::run_l0;
use aa_core::manifest::{self, CompileInput, compile::capability_digest, parse_overlay};
use aa_core::{digest, pb};
use harness::*;

fn git_lock() -> pb::CapabilityLock {
    let target = git_target();
    let overlay = parse_overlay(GIT_OVERLAY).unwrap();
    let audit = TestAudit::new();
    manifest::compile(
        CompileInput {
            candidates: help_text_candidates(&target, GIT_HELP),
            target,
            overlay: &overlay,
            overlay_sha256: digest::sha256_hex(GIT_OVERLAY.as_bytes()),
            sources: vec![source("help-text@1", "argv:git -h", GIT_HELP)],
            verification: &[],
            audit: &audit,
        },
        None,
    )
    .unwrap()
    .lock
}

fn idx(lock: &pb::CapabilityLock, id: &str) -> usize {
    lock.capabilities.iter().position(|c| c.id == id).unwrap()
}

/// Re-derives the digest after a test mutation so only the intended defect remains.
fn redigest(lock: &mut pb::CapabilityLock, i: usize) {
    lock.capabilities[i].capability_digest = capability_digest(&lock.capabilities[i]).unwrap();
}

fn l0(lock: &pb::CapabilityLock, id: &str) -> pb::VerificationRecord {
    run_l0(lock, cap(lock, id), "test")
}

fn has(rec: &pb::VerificationRecord, reason: pb::WithheldReason, needle: &str) -> bool {
    rec.findings
        .iter()
        .any(|f| f.reason == reason as i32 && f.hint.contains(needle))
}

#[test]
fn compiled_reads_pass() {
    let lock = git_lock();
    for id in ["git.status", "git.log", "git.branch.list"] {
        let rec = l0(&lock, id);
        assert_eq!(rec.status, pb::LaneStatus::Passed as i32, "{id}: {:?}", rec.findings);
        assert_eq!(rec.lane, pb::Lane::Static as i32);
        assert_eq!(rec.capability_digest, cap(&lock, id).capability_digest);
        assert_eq!(rec.lock_digest, lock.lock_digest);
        assert!(rec.record_id.starts_with("vr_"));
        assert!(!rec.non_claims.is_empty());
    }
}

#[test]
fn fails_on_open_schema() {
    let mut lock = git_lock();
    let i = idx(&lock, "git.status");
    lock.capabilities[i].input_schema_json = lock.capabilities[i]
        .input_schema_json
        .replace("\"additionalProperties\":false,", "");
    redigest(&mut lock, i);
    let rec = l0(&lock, "git.status");
    assert_eq!(rec.status, pb::LaneStatus::Failed as i32);
    assert!(
        has(&rec, pb::WithheldReason::SchemaInvalid, "open"),
        "{:?}",
        rec.findings
    );
}

#[test]
fn fails_on_duplicate_mcp_name() {
    let mut lock = git_lock();
    let i = idx(&lock, "git.log");
    lock.capabilities[i].projections.as_mut().unwrap().mcp_name = "git_status".into();
    let rec = l0(&lock, "git.status");
    assert!(
        has(&rec, pb::WithheldReason::SchemaInvalid, "duplicate MCP name"),
        "{:?}",
        rec.findings
    );
}

#[test]
fn fails_on_oracle_to_non_servable_read() {
    let lock = git_lock();
    // git.add's read-back oracle names git.status, which is not servable before verification.
    let rec = l0(&lock, "git.add");
    assert!(
        has(&rec, pb::WithheldReason::NoOracle, "git.status"),
        "{:?}",
        rec.findings
    );
    let mut lock = lock;
    let i = idx(&lock, "git.add");
    if let Some(pb::oracle::Kind::ReadBack(rb)) = lock.capabilities[i].oracles[0].kind.as_mut() {
        rb.capability_id = "git.nope".into();
    }
    redigest(&mut lock, i);
    assert!(has(
        &l0(&lock, "git.add"),
        pb::WithheldReason::NoOracle,
        "not in the lock"
    ));
}

#[test]
fn fails_on_unresolved_mixed_authority_branch() {
    let mut lock = git_lock();
    let i = idx(&lock, "git.status");
    lock.capabilities[i].branches.push(pb::EffectBranch {
        pointer: "/porcelain".into(),
        effects: None,
        ..Default::default()
    });
    redigest(&mut lock, i);
    let rec = l0(&lock, "git.status");
    assert!(
        has(&rec, pb::WithheldReason::MixedAuthority, "/porcelain"),
        "{:?}",
        rec.findings
    );
    // The native command an overlay mode-splits is unresolved too.
    assert!(has(
        &l0(&lock, "git.branch"),
        pb::WithheldReason::MixedAuthority,
        "mode"
    ));
}

#[test]
fn fails_on_fixture_include_cycle() {
    let mut lock = git_lock();
    let fixtures = &mut lock.target.as_mut().unwrap().fixtures;
    let include = |id: &str| pb::FixtureStep {
        step: Some(pb::fixture_step::Step::IncludeFixture(id.into())),
    };
    fixtures.push(pb::Fixture {
        id: "a".into(),
        steps: vec![include("b")],
        ..Default::default()
    });
    fixtures.push(pb::Fixture {
        id: "b".into(),
        steps: vec![include("a")],
        ..Default::default()
    });
    let i = idx(&lock, "git.status");
    lock.capabilities[i].examples.push(pb::Example {
        input_json: "{}".into(),
        fixture_id: "a".into(),
        source: "overlay".into(),
    });
    let rec = l0(&lock, "git.status");
    assert!(
        has(&rec, pb::WithheldReason::NotVerified, "cycle: a -> b -> a"),
        "{:?}",
        rec.findings
    );
    // The architecture's own include (repo-with-untracked -> one-commit-repo) is fine.
    assert_eq!(
        l0(&git_lock(), "git.add")
            .findings
            .iter()
            .filter(|f| f.hint.contains("cycle"))
            .count(),
        0
    );
}

#[test]
fn fails_on_unknown_field_without_hint() {
    let mut lock = git_lock();
    let i = idx(&lock, "git.status");
    lock.capabilities[i]
        .admission
        .as_mut()
        .unwrap()
        .withheld
        .retain(|w| w.reason != pb::WithheldReason::EffectsUnknown as i32);
    let rec = l0(&lock, "git.status");
    assert!(
        has(&rec, pb::WithheldReason::EffectsUnknown, "no withheld hint"),
        "{:?}",
        rec.findings
    );
}

#[test]
fn fails_on_split_without_harvested_path() {
    let target = git_target();
    let yaml = GIT_OVERLAY.replace(
        "  git.branch.list:\n    binding: { command_path: [branch]",
        "  git.branch.list:\n    binding: { command_path: [brnach]",
    );
    assert_ne!(yaml, GIT_OVERLAY);
    let overlay = parse_overlay(&yaml).unwrap();
    let audit = TestAudit::new();
    let lock = manifest::compile(
        CompileInput {
            candidates: help_text_candidates(&target, GIT_HELP),
            target,
            overlay: &overlay,
            overlay_sha256: String::new(),
            sources: vec![source("help-text@1", "argv:git -h", GIT_HELP)],
            verification: &[],
            audit: &audit,
        },
        None,
    )
    .unwrap()
    .lock;
    let rec = l0(&lock, "git.branch.list");
    assert!(
        has(
            &rec,
            pb::WithheldReason::SchemaInvalid,
            "matches no harvested native path"
        ),
        "{:?}",
        rec.findings
    );
}

#[test]
fn fails_when_derived_fields_do_not_recompute() {
    let mut lock = git_lock();
    let i = idx(&lock, "git.status");
    lock.capabilities[i].level = pb::EffectLevel::Read as i32;
    assert!(has(
        &l0(&lock, "git.status"),
        pb::WithheldReason::SchemaInvalid,
        "recompute"
    ));
    let mut lock = git_lock();
    lock.capabilities[i].capability_digest = "0".repeat(64);
    assert!(has(
        &l0(&lock, "git.status"),
        pb::WithheldReason::SchemaInvalid,
        "capability_digest"
    ));
}
