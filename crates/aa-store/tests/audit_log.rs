//! WP4 acceptance tests for the JSONL audit log (build-plan §3.4, tests 1-4) plus query/index behaviour.

use aa_core::{api::AuditSink, digest, pb};
use aa_store::{AuditLog, AuditQuery};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn agent() -> pb::Principal {
    pb::Principal {
        role: pb::Role::Agent as i32,
        channel: pb::Channel::AgentSocket as i32,
        uid: 501,
        pid: 4242,
        client_name: "aa-cli".into(),
        ..Default::default()
    }
}

fn operator() -> pb::Principal {
    pb::Principal {
        role: pb::Role::Operator as i32,
        channel: pb::Channel::OperatorSocket as i32,
        uid: 501,
        ..Default::default()
    }
}

fn intent(id: &str, cap: &str, level: pb::EffectLevel, key: &str) -> pb::ActionIntent {
    pb::ActionIntent {
        action_id: id.into(),
        capability_id: cap.into(),
        capability_digest: digest::sha256_hex(cap.as_bytes()),
        action_digest: digest::sha256_hex(id.as_bytes()),
        args_commitment: "hmac-sha256:00".into(),
        level: level as i32,
        decision: Some(pb::PolicyDecision {
            verdict: pb::Verdict::Allow as i32,
            deciding_rule_id: "rule-1".into(),
            ..Default::default()
        }),
        idempotency_key: key.into(),
    }
}

fn action(id: &str, cap: &str, level: pb::EffectLevel, key: &str) -> pb::ActionRecord {
    pb::ActionRecord {
        action_id: id.into(),
        capability_id: cap.into(),
        level: level as i32,
        idempotency_key: key.into(),
        outcome: pb::Outcome::Completed as i32,
        input_redacted_json: r#"{"repo":"/tmp/r","note":"ünïcode ✓"}"#.into(),
        process: Some(pb::ProcessReport {
            exit_code: 0,
            stdout_bytes: 12,
            wall_ms: 3,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn admin(kind: pb::AdminEventKind, subject: &str) -> pb::AdminEvent {
    pb::AdminEvent {
        kind: kind as i32,
        subject: subject.into(),
        details_json: r#"{"k":1}"#.into(),
        ..Default::default()
    }
}

fn segments(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(dir.join("audit"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with("segment-"))
        .collect();
    v.sort();
    v
}

/// Appends `n` mixed entries: admin events, READ actions, and WRITE intent+action pairs (no dangling intents).
fn append_mixed(log: &AuditLog, n: u64) {
    let mut written = 0;
    let mut i = 0;
    while written < n {
        match i % 4 {
            0 => {
                log.append_admin(&operator(), 1, admin(pb::AdminEventKind::PolicyLoaded, "policy"))
                    .unwrap();
                written += 1;
            }
            1 | 2 => {
                log.append_action(
                    &agent(),
                    1,
                    action(&format!("act_r{i}"), "git.status", pb::EffectLevel::Read, ""),
                )
                .unwrap();
                written += 1;
            }
            _ if written + 2 <= n => {
                let id = format!("act_w{i}");
                log.append_intent(
                    &agent(),
                    1,
                    intent(&id, "fixture-notes.add", pb::EffectLevel::Write, ""),
                )
                .unwrap();
                log.append_action(
                    &agent(),
                    1,
                    action(&id, "fixture-notes.add", pb::EffectLevel::Write, ""),
                )
                .unwrap();
                written += 2;
            }
            _ => {}
        }
        i += 1;
    }
}

#[test]
fn t1_thousand_mixed_entries_reopen_chain_ok() {
    let d = tempfile::tempdir().unwrap();
    let head = {
        let (log, boot) = AuditLog::open(d.path()).unwrap();
        assert!(boot.chain_ok && boot.head_matches && boot.entries == 0);
        append_mixed(&log, 1000);
        let head = log.head();
        assert_eq!(head.0, 1000);
        assert!(log.verify_chain().chain_ok);
        head
    };
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(boot.chain_ok, "{boot:?}");
    assert!(boot.head_matches);
    assert_eq!(boot.entries, 1000);
    assert_eq!(boot.first_broken_seq, None);
    assert!(boot.recovered_action_ids.is_empty());
    assert_eq!(log.head(), head);
    assert_eq!(log.entry(1000).unwrap().unwrap().entry_sha256, head.1);
    // The chain continues across reopen.
    let seq = log
        .append_admin(&operator(), 1, admin(pb::AdminEventKind::EngineStop, ""))
        .unwrap();
    assert_eq!(seq, 1001);
    assert_eq!(log.entry(1001).unwrap().unwrap().prev_sha256, head.1);
    assert!(log.verify_chain().chain_ok);
}

#[test]
fn t2_tamper_flip_one_byte_reports_that_line() {
    let d = tempfile::tempdir().unwrap();
    {
        let (log, _) = AuditLog::open(d.path()).unwrap();
        append_mixed(&log, 120);
    }
    let seg = segments(d.path()).remove(0);
    let mut bytes = fs::read(&seg).unwrap();
    let target_line = 60usize;
    let start = bytes
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(target_line - 2)
        .map(|(i, _)| i + 1)
        .unwrap();
    let end = start + bytes[start..].iter().position(|b| *b == b'\n').unwrap();
    for pos in [start + (end - start) / 2, start + 3, end - 2] {
        let mut tampered = bytes.clone();
        tampered[pos] ^= 0x01;
        fs::write(&seg, &tampered).unwrap();
        let (log, boot) = AuditLog::open(d.path()).unwrap();
        assert!(!boot.chain_ok, "flip at {pos}");
        assert_eq!(boot.first_broken_seq, Some(target_line as u64), "flip at {pos}");
        assert_eq!(boot.entries, 120);
        assert!(boot.head_matches, "the tail is intact");
        assert_eq!(log.verify_chain().first_broken_seq, Some(target_line as u64));
        drop(log);
    }
    // Restore, and the log verifies again (opening never rewrote anything).
    fs::write(&seg, &bytes).unwrap();
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(boot.chain_ok, "{boot:?}");
    drop(log);

    // Delete the last line: the chain itself is consistent, but HEAD no longer matches.
    bytes.pop();
    let cut = bytes.iter().rposition(|b| *b == b'\n').unwrap() + 1;
    bytes.truncate(cut);
    fs::write(&seg, &bytes).unwrap();
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(!boot.head_matches);
    assert!(!boot.chain_ok);
    assert_eq!(boot.first_broken_seq, None);
    assert_eq!(boot.entries, 119);
    assert!(!log.verify_chain().head_matches);
}

#[test]
fn t2b_tampered_valid_json_is_detected() {
    // A semantically valid edit (the line still parses) must also break the chain.
    let d = tempfile::tempdir().unwrap();
    {
        let (log, _) = AuditLog::open(d.path()).unwrap();
        append_mixed(&log, 10);
    }
    let seg = segments(d.path()).remove(0);
    let text = fs::read_to_string(&seg).unwrap();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut e: pb::AuditEntry = serde_json::from_str(&lines[4]).unwrap();
    e.halt_epoch = 99;
    lines[4] = String::from_utf8(digest::jcs(&e).unwrap()).unwrap();
    fs::write(&seg, lines.join("\n") + "\n").unwrap();
    let (_log, boot) = AuditLog::open(d.path()).unwrap();
    assert_eq!(boot.first_broken_seq, Some(5));
}

#[test]
fn t3_crash_recovery_closes_dangling_intent_once() {
    let d = tempfile::tempdir().unwrap();
    {
        let (log, _) = AuditLog::open(d.path()).unwrap();
        log.append_intent(
            &agent(),
            3,
            intent("act_done", "git.commit", pb::EffectLevel::Write, ""),
        )
        .unwrap();
        log.append_action(
            &agent(),
            3,
            action("act_done", "git.commit", pb::EffectLevel::Write, ""),
        )
        .unwrap();
        log.append_intent(
            &agent(),
            3,
            intent("act_lost", "git.commit", pb::EffectLevel::Write, "idem-1"),
        )
        .unwrap();
        // Crash: no ActionRecord for act_lost.
    }
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(boot.chain_ok, "{boot:?}");
    assert_eq!(boot.entries, 3);
    assert_eq!(boot.recovered_action_ids, ["act_lost"]);
    let rec = log.find_action("act_lost").unwrap().unwrap();
    assert_eq!(rec.outcome, pb::Outcome::Unknown as i32);
    assert!(rec.recovered);
    assert_eq!(
        rec.error.as_ref().unwrap().code,
        pb::ErrorCode::ExecutionStateUnknown as i32
    );
    assert_eq!(rec.principal.as_ref().unwrap().role, pb::Role::EngineInternal as i32);
    assert_eq!(rec.capability_id, "git.commit");
    assert_eq!(rec.level, pb::EffectLevel::Write as i32);
    assert_eq!(rec.idempotency_key, "idem-1");
    let entry = log.entry(4).unwrap().unwrap();
    assert_eq!(entry.principal.as_ref().unwrap().role, pb::Role::EngineInternal as i32);
    assert_eq!(entry.halt_epoch, 3);
    assert_eq!(
        log.find_by_idempotency_key("idem-1").unwrap().unwrap().action_id,
        "act_lost"
    );
    assert!(!log.find_action("act_done").unwrap().unwrap().recovered);
    drop(log);

    // Idempotent: a second reopen adds nothing.
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(boot.chain_ok);
    assert_eq!(boot.entries, 4);
    assert!(boot.recovered_action_ids.is_empty());
    assert_eq!(log.head().0, 4);
    let lost = log
        .query(&AuditQuery {
            action_id: Some("act_lost".into()),
            actions_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(lost.len(), 1);
}

#[test]
fn t4_lines_are_canonical_proto3_json_audit_entries() {
    let d = tempfile::tempdir().unwrap();
    let (log, _) = AuditLog::open(d.path()).unwrap();
    append_mixed(&log, 12);
    let mut prev = String::new();
    let mut seq = 0;
    for seg in segments(d.path()) {
        let bytes = fs::read(&seg).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            seq += 1;
            let entry: pb::AuditEntry = serde_json::from_slice(line).expect("line is a proto3-JSON AuditEntry");
            // Byte-identical canonical form, and a lossless round trip through the proto3-JSON mapping.
            assert_eq!(digest::jcs(&entry).unwrap(), line);
            let again: pb::AuditEntry = serde_json::from_str(&serde_json::to_string(&entry).unwrap()).unwrap();
            assert_eq!(again, entry);
            // proto3-JSON names and string-encoded 64-bit ints.
            let v: serde_json::Value = serde_json::from_slice(line).unwrap();
            assert_eq!(v["seq"], serde_json::Value::String(seq.to_string()));
            assert!(v.get("entrySha256").is_some());
            assert!(v.get("writtenAt").is_some_and(serde_json::Value::is_string));
            assert_eq!(entry.seq, seq);
            assert_eq!(entry.prev_sha256, prev);
            assert_eq!(
                entry.entry_sha256,
                digest::self_digest(&entry, |e| e.entry_sha256.clear()).unwrap()
            );
            assert_eq!(log.entry(seq).unwrap().unwrap(), entry);
            prev = entry.entry_sha256.clone();
        }
    }
    assert_eq!(seq, 12);
    let head = fs::read_to_string(d.path().join("audit/HEAD")).unwrap();
    assert_eq!(head, format!("12 {prev}\n"));
    // An entry with an unknown field does not validate as AuditEntry.
    assert!(serde_json::from_str::<pb::AuditEntry>(r#"{"seq":"1","bogus":true}"#).is_err());
}

#[test]
fn query_filters_and_indexes() {
    let d = tempfile::tempdir().unwrap();
    let (log, _) = AuditLog::open(&d.path().join("audit")).unwrap();
    log.append_admin(&operator(), 0, admin(pb::AdminEventKind::TargetRegistered, "git"))
        .unwrap(); // 1
    log.append_action(&agent(), 0, action("act_1", "git.status", pb::EffectLevel::Read, "k1"))
        .unwrap(); // 2
    log.append_intent(&agent(), 0, intent("act_2", "gitx.push", pb::EffectLevel::Write, ""))
        .unwrap(); // 3
    log.append_action(&agent(), 0, action("act_2", "gitx.push", pb::EffectLevel::Write, ""))
        .unwrap(); // 4
    log.append_action(&agent(), 0, action("act_3", "git.log", pb::EffectLevel::Read, "k1"))
        .unwrap(); // 5
    log.append_admin(&operator(), 0, admin(pb::AdminEventKind::EffectAssumed, "git.log"))
        .unwrap(); // 6

    let seqs = |q: AuditQuery| log.query(&q).unwrap().iter().map(|e| e.seq).collect::<Vec<_>>();
    assert_eq!(seqs(AuditQuery::default()), [1, 2, 3, 4, 5, 6]);
    assert_eq!(
        seqs(AuditQuery {
            target_id: Some("git".into()),
            ..Default::default()
        }),
        [1, 2, 5, 6]
    );
    assert_eq!(
        seqs(AuditQuery {
            target_id: Some("git".into()),
            actions_only: true,
            ..Default::default()
        }),
        [2, 5]
    );
    assert_eq!(
        seqs(AuditQuery {
            capability_id: Some("git.log".into()),
            ..Default::default()
        }),
        [5, 6]
    );
    assert_eq!(
        seqs(AuditQuery {
            action_id: Some("act_2".into()),
            ..Default::default()
        }),
        [3, 4]
    );
    assert_eq!(
        seqs(AuditQuery {
            after_seq: 2,
            limit: 2,
            ..Default::default()
        }),
        [3, 4]
    );
    assert!(
        seqs(AuditQuery {
            after_seq: 6,
            ..Default::default()
        })
        .is_empty()
    );

    assert_eq!(log.find_action("act_2").unwrap().unwrap().capability_id, "gitx.push");
    assert!(log.find_action("act_nope").unwrap().is_none());
    // The most recent record with the key wins.
    assert_eq!(log.find_by_idempotency_key("k1").unwrap().unwrap().action_id, "act_3");
    assert!(log.find_by_idempotency_key("").unwrap().is_none());
    assert!(log.entry(0).unwrap().is_none());
    assert!(log.entry(7).unwrap().is_none());
    assert_eq!(log.head().0, 6);
}

#[test]
fn concurrent_appends_keep_one_chain() {
    let d = tempfile::tempdir().unwrap();
    let (log, _) = AuditLog::open(d.path()).unwrap();
    let log = Arc::new(log);
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                for i in 0..25 {
                    log.append_action(
                        &agent(),
                        0,
                        action(&format!("act_{t}_{i}"), "git.status", pb::EffectLevel::Read, ""),
                    )
                    .unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let report = log.verify_chain();
    assert!(report.chain_ok, "{report:?}");
    assert_eq!(report.entries, 100);
}

#[test]
fn audit_dir_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let (log, _) = AuditLog::open(d.path()).unwrap();
    log.append_admin(&operator(), 0, admin(pb::AdminEventKind::EngineStart, ""))
        .unwrap();
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&d.path().join("audit")), 0o700);
    assert_eq!(mode(&segments(d.path())[0]), 0o600);
    assert_eq!(mode(&d.path().join("audit/HEAD")), 0o600);
}

#[test]
fn head_lagging_by_one_is_a_crash_window_not_truncation() {
    let d = tempfile::tempdir().unwrap();
    let head_path = d.path().join("audit/HEAD");
    let old_head = {
        let (log, _) = AuditLog::open(d.path()).unwrap();
        append_mixed(&log, 5);
        let old = fs::read(&head_path).unwrap();
        log.append_admin(&operator(), 0, admin(pb::AdminEventKind::EngineStop, ""))
            .unwrap();
        old
    };
    // Simulate a crash after the line reached the segment but before HEAD's rename.
    fs::write(&head_path, &old_head).unwrap();
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(boot.head_matches && boot.chain_ok, "{boot:?}");
    drop(log);
    // Lagging by two is not tolerated.
    {
        let (log, _) = AuditLog::open(d.path()).unwrap();
        log.append_admin(&operator(), 0, admin(pb::AdminEventKind::EngineStop, ""))
            .unwrap();
    }
    fs::write(&head_path, &old_head).unwrap();
    let (_log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(!boot.head_matches && !boot.chain_ok);
    // A garbage HEAD never matches.
    drop(_log);
    fs::write(&head_path, b"garbage").unwrap();
    assert!(!AuditLog::open(d.path()).unwrap().1.head_matches);
}

#[test]
fn recovery_is_deferred_while_the_chain_is_broken() {
    let d = tempfile::tempdir().unwrap();
    {
        let (log, _) = AuditLog::open(d.path()).unwrap();
        log.append_admin(&operator(), 0, admin(pb::AdminEventKind::EngineStart, ""))
            .unwrap();
        log.append_intent(&agent(), 0, intent("act_x", "git.commit", pb::EffectLevel::Write, ""))
            .unwrap();
    }
    let seg = segments(d.path()).remove(0);
    let good = fs::read(&seg).unwrap();
    let mut bad = good.clone();
    bad[10] ^= 0x01; // line 1
    fs::write(&seg, &bad).unwrap();
    {
        let (log, boot) = AuditLog::open(d.path()).unwrap();
        assert_eq!(boot.first_broken_seq, Some(1));
        assert!(boot.recovered_action_ids.is_empty());
        assert_eq!(log.head().0, 2);
        assert!(log.find_action("act_x").unwrap().is_none());
    }
    assert_eq!(fs::read(&seg).unwrap(), bad, "never rewritten");
    fs::write(&seg, &good).unwrap();
    let (log, boot) = AuditLog::open(d.path()).unwrap();
    assert!(boot.chain_ok);
    assert_eq!(boot.recovered_action_ids, ["act_x"]);
    assert!(log.find_action("act_x").unwrap().unwrap().recovered);
}
