//! Acceptance test 3 (build-plan §3.1): recorded fixtures compile to snapshotted locks, compile is byte-identical
//! across runs, and changing one help line changes exactly one `capability_digest` and resets only that
//! capability's admission.

#[path = "fixtures/harness.rs"]
mod harness;

use aa_core::api::{AuditSink, Candidate};
use aa_core::manifest::{self, CompileInput, CompileOutput, Overlay, parse_overlay};
use aa_core::{digest, pb};
use harness::*;

struct Fixture {
    target: pb::Target,
    overlay: Overlay,
    overlay_text: &'static str,
    sources: Vec<pb::SourceRef>,
    candidates: Vec<Candidate>,
}

fn git(help: &str) -> Fixture {
    let target = git_target();
    Fixture {
        candidates: help_text_candidates(&target, help),
        sources: vec![source(
            "help-text@1",
            "argv:git {status,log,branch,add,rev-parse} -h",
            help,
        )],
        overlay: parse_overlay(GIT_OVERLAY).unwrap(),
        overlay_text: GIT_OVERLAY,
        target,
    }
}

fn notes() -> Fixture {
    let target = notes_target();
    Fixture {
        candidates: click_candidates(&target, NOTES_CLICK),
        sources: vec![source(
            "click-info@1",
            "argv:python -I aa_click_info.py notes-cli",
            NOTES_CLICK,
        )],
        overlay: parse_overlay(NOTES_OVERLAY).unwrap(),
        overlay_text: NOTES_OVERLAY,
        target,
    }
}

fn gh() -> Fixture {
    let target = gh_target();
    Fixture {
        candidates: cobra_candidates(&target, GH_COBRA),
        sources: vec![source("cobra-complete@1", "argv:gh __complete alias", GH_COBRA)],
        overlay: parse_overlay(GH_OVERLAY).unwrap(),
        overlay_text: GH_OVERLAY,
        target,
    }
}

fn run(
    f: &Fixture,
    records: &[pb::VerificationRecord],
    audit: &dyn AuditSink,
    previous: Option<&pb::CapabilityLock>,
) -> CompileOutput {
    manifest::compile(
        CompileInput {
            target: f.target.clone(),
            overlay: &f.overlay,
            overlay_sha256: digest::sha256_hex(f.overlay_text.as_bytes()),
            sources: f.sources.clone(),
            candidates: f.candidates.clone(),
            verification: records,
            audit,
        },
        previous,
    )
    .unwrap()
}

fn pretty(lock: &pb::CapabilityLock) -> String {
    let v: serde_json::Value = serde_json::from_slice(&manifest::lock_to_json(lock).unwrap()).unwrap();
    serde_json::to_string_pretty(&v).unwrap()
}

fn snapshot(name: &str, lock: &pb::CapabilityLock) {
    insta::with_settings!({snapshot_path => "fixtures/snapshots", prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(name, pretty(lock));
    });
}

fn ids(lock: &pb::CapabilityLock) -> Vec<&str> {
    lock.capabilities.iter().map(|c| c.id.as_str()).collect()
}

#[test]
fn git_lock_golden_and_deterministic() {
    let f = git(GIT_HELP);
    let audit = TestAudit::new();
    let a = run(&f, &[], &audit, None);
    let b = run(&f, &[], &audit, None);
    let bytes = manifest::lock_to_json(&a.lock).unwrap();
    assert_eq!(
        bytes,
        manifest::lock_to_json(&b.lock).unwrap(),
        "compile must be byte-identical"
    );
    assert_eq!(manifest::lock_from_json(&bytes).unwrap(), a.lock);

    // 5 native + 2 overlay mode splits.
    assert_eq!(
        ids(&a.lock),
        [
            "git.add",
            "git.branch",
            "git.branch.delete",
            "git.branch.list",
            "git.log",
            "git.rev-parse",
            "git.status"
        ]
    );
    assert_eq!(a.added.len(), 7);
    assert!(reasons(cap(&a.lock, "git.branch")).contains(&pb::WithheldReason::MixedAuthority));
    assert!(reasons(cap(&a.lock, "git.rev-parse")).contains(&pb::WithheldReason::MixedAuthority));
    assert!(!reasons(cap(&a.lock, "git.log")).contains(&pb::WithheldReason::MixedAuthority));

    let status = cap(&a.lock, "git.status");
    let schema: serde_json::Value = serde_json::from_str(&status.input_schema_json).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["repo"],
        serde_json::json!({"format": "path", "type": "string"})
    );
    assert!(schema["required"].as_array().unwrap().contains(&"repo".into()));
    assert_eq!(
        schema["properties"]["untracked_files"]["enum"],
        serde_json::json!(["all", "normal", "no"])
    );
    assert_eq!(status.output_schema_json, manifest::compile::text_output_schema());
    assert_eq!(status.level, pb::EffectLevel::Unknown as i32);
    assert!(reasons(status).contains(&pb::WithheldReason::EffectsUnknown));
    assert_eq!(status.projections.as_ref().unwrap().mcp_name, "git_status");
    let Some(pb::binding::Kind::Cli(cli)) = status.binding.as_ref().unwrap().kind.as_ref() else {
        panic!("cli binding")
    };
    assert_eq!(
        cli.global_args,
        ["--no-optional-locks", "-c", "core.pager=cat", "-c", "color.ui=never"]
    );
    assert_eq!(cli.fixed_args, ["--porcelain=v2", "--branch"]);
    assert_eq!(cli.usage_exit_code, 129);
    assert_eq!(cli.cwd_pointer, "/repo");
    assert_eq!(cli.env.get("GIT_OPTIONAL_LOCKS").map(String::as_str), Some("0"));

    // git.add: verb raise + WRITE_PATH scope (writes_filesystem can never be lowered); --force branch.
    let add = cap(&a.lock, "git.add");
    assert_eq!(add.effects.as_ref().unwrap().mutates_target, pb::Tri::True as i32);
    assert!(
        add.scopes
            .iter()
            .any(|s| s.pointer == "/repo" && s.kind == pb::ScopeKind::WritePath as i32)
    );
    assert!(add.branches.iter().any(|b| b.pointer == "/force"));
    assert_eq!(add.oracles.len(), 1);

    // Mode split: delete is DESTRUCTIVE by verb, list inherits no native args.
    let del = cap(&a.lock, "git.branch.delete");
    assert_eq!(del.effects.as_ref().unwrap().may_discard_data, pb::Tri::True as i32);
    let list = cap(&a.lock, "git.branch.list");
    let list_schema: serde_json::Value = serde_json::from_str(&list.input_schema_json).unwrap();
    assert_eq!(list_schema["properties"].as_object().unwrap().len(), 1, "{list_schema}");

    assert_eq!(a.lock.rule_versions.get("help-text").map(String::as_str), Some("1"));
    assert_eq!(a.lock.rule_versions.get("compile").map(String::as_str), Some("1"));
    snapshot("git.lock", &a.lock);
}

#[test]
fn git_verification_then_one_help_line_change() {
    let f = git(GIT_HELP);
    let audit = TestAudit::new();
    let first = run(&f, &[], &audit, None);
    let reads = [
        "git.status",
        "git.log",
        "git.branch.list",
        "git.branch",
        "git.rev-parse",
    ];
    let records = passing_records(&first.lock, &reads, 100);
    let verified = run(&f, &records, &audit, Some(&first.lock));
    assert!(
        verified.changed.is_empty(),
        "verification must not change capability digests"
    );
    for id in ["git.status", "git.log", "git.branch.list"] {
        let c = cap(&verified.lock, id);
        assert_eq!(state(c), pb::AdmissionState::Servable, "{id}: {:?}", c.admission);
        assert_eq!(c.level, pb::EffectLevel::Read as i32);
        assert_eq!(c.tier, pb::AuthorityTier::Observe as i32);
        assert!(c.hints.as_ref().unwrap().read_only);
        assert_eq!(
            c.admission.as_ref().unwrap().required_runtime_isolation,
            pb::Isolation::Confined as i32
        );
        assert_eq!(c.admission.as_ref().unwrap().verification_record_ids.len(), 3);
    }
    for id in ["git.branch", "git.rev-parse"] {
        let c = cap(&verified.lock, id);
        assert_eq!(state(c), pb::AdmissionState::Withheld);
        assert!(reasons(c).contains(&pb::WithheldReason::MixedAuthority), "{id}");
    }
    assert!(reasons(cap(&verified.lock, "git.add")).contains(&pb::WithheldReason::NotVerified));
    assert_eq!(
        state(cap(&verified.lock, "git.branch.delete")),
        pb::AdmissionState::Withheld
    );
    assert_eq!(
        verified.lock.target.as_ref().unwrap().status,
        pb::TargetStatus::LiveVerified as i32
    );
    snapshot("git.verified.lock", &verified.lock);

    // Recompiling with the same records is a fixed point.
    let again = run(&f, &records, &audit, Some(&verified.lock));
    assert_eq!(
        manifest::lock_to_json(&again.lock).unwrap(),
        manifest::lock_to_json(&verified.lock).unwrap()
    );
    // Without records, unchanged capabilities carry their admission over from the previous lock.
    let carried = run(&f, &[], &audit, Some(&verified.lock));
    assert_eq!(
        manifest::lock_to_json(&carried.lock).unwrap(),
        manifest::lock_to_json(&verified.lock).unwrap()
    );

    // One changed help line: `git log` gains a value on --source.
    let edited = GIT_HELP.replacen(
        "--[no-]source         show source",
        "--[no-]source <ref>   show source",
        1,
    );
    assert_ne!(edited, GIT_HELP, "fixture line not found");
    let f2 = git(&edited);
    for (recs, prev) in [(&records[..], &verified.lock), (&[][..], &verified.lock)] {
        let out = run(&f2, recs, &audit, Some(prev));
        assert_eq!(out.changed, ["git.log"]);
        assert!(out.added.is_empty() && out.removed.is_empty());
        for c in &out.lock.capabilities {
            let before = cap(&verified.lock, &c.id);
            if c.id == "git.log" {
                assert_ne!(c.capability_digest, before.capability_digest);
                assert_eq!(state(c), pb::AdmissionState::Withheld);
                assert!(reasons(c).contains(&pb::WithheldReason::NotVerified));
            } else {
                assert_eq!(c.capability_digest, before.capability_digest, "{}", c.id);
                assert_eq!(c.admission, before.admission, "{}", c.id);
            }
        }
    }
}

#[test]
fn git_side_effect_is_caught() {
    let f = git(GIT_HELP);
    let audit = TestAudit::new();
    let first = run(&f, &[], &audit, None);
    let mut records = passing_records(&first.lock, &["git.status"], 100);
    let l3 = records
        .iter_mut()
        .find(|r| r.lane == pb::Lane::SandboxDiff as i32)
        .unwrap();
    l3.status = pb::LaneStatus::Failed as i32;
    l3.observed_files.push(pb::ObservedFileEffect {
        path: "repo/.git/index".into(),
        root: "private:cwd".into(),
        change: pb::FileChange::Modified as i32,
        ..Default::default()
    });
    l3.findings.push(pb::Withheld {
        reason: pb::WithheldReason::SideEffect as i32,
        hint: "observed writes_filesystem: private:cwd repo/.git/index MODIFIED".into(),
        ..Default::default()
    });
    let out = run(&f, &records, &audit, Some(&first.lock));
    let status = cap(&out.lock, "git.status");
    assert_eq!(state(status), pb::AdmissionState::Withheld);
    assert_eq!(status.effects.as_ref().unwrap().writes_filesystem, pb::Tri::True as i32);
    assert_ne!(status.level, pb::EffectLevel::Read as i32);
    let side = status
        .admission
        .as_ref()
        .unwrap()
        .withheld
        .iter()
        .find(|w| w.reason == pb::WithheldReason::SideEffect as i32)
        .expect("SIDE_EFFECT finding");
    assert!(side.hint.contains(".git/index"));
}

#[test]
fn notes_click_harness_golden() {
    let f = notes();
    let audit = TestAudit::new();
    let first = run(&f, &[], &audit, None);
    assert_eq!(first.lock.capabilities.len(), 7);
    let add = cap(&first.lock, "fixture-notes.notes.add");
    // add -> mutates_target; autosave with the --project state handle -> writes_filesystem.
    assert_eq!(add.effects.as_ref().unwrap().mutates_target, pb::Tri::True as i32);
    assert_eq!(add.effects.as_ref().unwrap().writes_filesystem, pb::Tri::True as i32);
    assert!(
        add.signals
            .iter()
            .any(|s| s.source == "rule:cli-anything@1" && s.field == "writes_filesystem")
    );
    let remove = cap(&first.lock, "fixture-notes.notes.remove");
    assert_eq!(remove.effects.as_ref().unwrap().may_discard_data, pb::Tri::True as i32);
    let export = cap(&first.lock, "fixture-notes.export");
    assert!(
        export
            .scopes
            .iter()
            .any(|s| s.pointer == "/output" && s.kind == pb::ScopeKind::WritePath as i32)
    );
    let repl = cap(&first.lock, "fixture-notes.repl");
    assert!(reasons(repl).contains(&pb::WithheldReason::Interactive));
    assert!(repl.projections.as_ref().unwrap().mcp_omit);
    let list = cap(&first.lock, "fixture-notes.notes.list");
    assert_eq!(list.projections.as_ref().unwrap().mcp_name, "notes_notes_list");
    assert!(list.core);
    snapshot("notes.lock", &first.lock);

    // Verify: list passes; stats writes ~/.cache (SIDE_EFFECT); lenient accepts the unknown flag (PARSER_MISMATCH).
    let mut records = passing_records(
        &first.lock,
        &[
            "fixture-notes.notes.list",
            "fixture-notes.notes.stats",
            "fixture-notes.notes.lenient",
            "fixture-notes.notes.add",
        ],
        100,
    );
    for r in &mut records {
        if r.capability_id == "fixture-notes.notes.stats" && r.lane == pb::Lane::SandboxDiff as i32 {
            r.status = pb::LaneStatus::Failed as i32;
            r.observed_files.push(pb::ObservedFileEffect {
                path: ".cache/notes/stats".into(),
                root: "private:home".into(),
                change: pb::FileChange::Created as i32,
                ..Default::default()
            });
            r.findings.push(pb::Withheld {
                reason: pb::WithheldReason::SideEffect as i32,
                hint: "private:home .cache/notes/stats CREATED".into(),
                ..Default::default()
            });
        }
        if r.capability_id == "fixture-notes.notes.lenient" && r.lane == pb::Lane::Parser as i32 {
            r.status = pb::LaneStatus::Failed as i32;
            r.findings.push(pb::Withheld {
                reason: pb::WithheldReason::ParserMismatch as i32,
                hint: "--aa-probe-unknown was accepted".into(),
                ..Default::default()
            });
        }
    }
    let out = run(&f, &records, &audit, Some(&first.lock));
    assert_eq!(
        state(cap(&out.lock, "fixture-notes.notes.list")),
        pb::AdmissionState::Servable
    );
    assert!(reasons(cap(&out.lock, "fixture-notes.notes.stats")).contains(&pb::WithheldReason::SideEffect));
    assert!(reasons(cap(&out.lock, "fixture-notes.notes.lenient")).contains(&pb::WithheldReason::ParserMismatch));
    let add = cap(&out.lock, "fixture-notes.notes.add");
    assert_eq!(add.level, pb::EffectLevel::Write as i32);
    assert!(reasons(add).contains(&pb::WithheldReason::NotVerified));
    assert!(reasons(cap(&out.lock, "fixture-notes.repl")).contains(&pb::WithheldReason::Interactive));
}

#[test]
fn gh_alias_cobra_golden() {
    let f = gh();
    let audit = TestAudit::new();
    let first = run(&f, &[], &audit, None);
    assert_eq!(
        ids(&first.lock),
        ["gh.alias.delete", "gh.alias.import", "gh.alias.list", "gh.alias.set"]
    );
    let set = cap(&first.lock, "gh.alias.set");
    assert!(set.branches.iter().any(|b| b.pointer == "/clobber"));
    let Some(pb::binding::Kind::Cli(cli)) = set.binding.as_ref().unwrap().kind.as_ref() else {
        panic!("cli")
    };
    assert_eq!(cli.usage_exit_code, 1);
    assert_eq!(cli.usage_stderr_marker, "unknown flag");
    snapshot("gh-alias.lock", &first.lock);
    let records = passing_records(&first.lock, &["gh.alias.list"], 100);
    let out = run(&f, &records, &audit, Some(&first.lock));
    assert_eq!(state(cap(&out.lock, "gh.alias.list")), pb::AdmissionState::Servable);
    assert_eq!(state(cap(&out.lock, "gh.alias.delete")), pb::AdmissionState::Withheld);
}

#[test]
fn assumptions_count_only_when_the_audit_entry_verifies() {
    let operator = pb::Principal {
        role: pb::Role::Operator as i32,
        channel: pb::Channel::OperatorSocket as i32,
        ..Default::default()
    };
    let audit = TestAudit::new();
    let assume = |field: &str, value: bool| pb::AdminEvent {
        kind: pb::AdminEventKind::EffectAssumed as i32,
        subject: "git".into(),
        details_json: serde_json::json!({"field": field, "value": value, "glob": "git.status"}).to_string(),
        operator_note: None,
    };
    let seq = audit
        .append_admin(&operator, 0, assume("reads_credentials", false))
        .unwrap();
    let agent = pb::Principal {
        role: pb::Role::Agent as i32,
        ..Default::default()
    };
    let forged = audit.append_admin(&agent, 0, assume("mutates_device", false)).unwrap();
    let mut f = git(GIT_HELP);
    let mut yaml = GIT_OVERLAY.to_string();
    yaml.push_str(&format!(
        "assumed:\n  - {{ field: reads_credentials, value: false, glob: git.status, reason: \"reads no credentials\", audit_seq: {seq} }}\n  - {{ field: mutates_device, value: false, glob: git.status, reason: forged, audit_seq: {forged} }}\n  - {{ field: irreversible, value: false, glob: git.status, reason: missing, audit_seq: 999 }}\n"
    ));
    f.overlay = parse_overlay(&yaml).unwrap();
    let out = run(&f, &[], &audit, None);
    let status = cap(&out.lock, "git.status");
    let e = status.effects.as_ref().unwrap();
    assert_eq!(e.reads_credentials, pb::Tri::False as i32);
    assert_eq!(e.mutates_device, pb::Tri::Unknown as i32);
    assert_eq!(e.irreversible, pb::Tri::Unknown as i32);
    assert!(
        status
            .signals
            .iter()
            .any(|s| s.source == format!("operator:audit_seq={seq}") && s.confidence == pb::Confidence::Operator as i32)
    );
    // Other capabilities are outside the glob.
    assert_eq!(
        cap(&out.lock, "git.log").effects.as_ref().unwrap().reads_credentials,
        pb::Tri::Unknown as i32
    );
}

#[test]
fn overlay_exclude_and_unknown_overlay_capability() {
    let mut f = git(GIT_HELP);
    let yaml = GIT_OVERLAY.replace(
        "exclude:\n",
        "exclude:\n  - { capability: git.rev-parse, reason: \"not needed\" }\n",
    );
    f.overlay = parse_overlay(&yaml).unwrap();
    let audit = TestAudit::new();
    let out = run(&f, &[], &audit, None);
    let rp = cap(&out.lock, "git.rev-parse");
    assert_eq!(state(rp), pb::AdmissionState::Excluded);
    assert!(rp.projections.as_ref().unwrap().mcp_omit);

    let yaml = yaml.replacen("capabilities:\n", "capabilities:\n  git.nope:\n    core: true\n", 1);
    f.overlay = parse_overlay(&yaml).unwrap();
    let err = manifest::compile(
        CompileInput {
            target: f.target.clone(),
            overlay: &f.overlay,
            overlay_sha256: String::new(),
            sources: f.sources.clone(),
            candidates: f.candidates.clone(),
            verification: &[],
            audit: &audit,
        },
        None,
    )
    .err()
    .expect("unknown overlay capability must fail");
    assert_eq!(err.code, pb::ErrorCode::ConfigurationInvalid);
}
