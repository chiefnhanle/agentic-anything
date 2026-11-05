//! WP4 acceptance tests for the workspace (build-plan §3.4, tests 5-7) plus layout and record round trips.

use aa_core::{digest, pb};
use aa_store::{AuditLog, HaltStore, Workspace};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

fn mode(p: &Path) -> u32 {
    fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
}

fn target(id: &str) -> pb::Target {
    pb::Target {
        id: id.into(),
        display_name: format!("{id} (test)"),
        identity: Some(pb::TargetIdentity::default()),
        authorization_statement: "I own this machine".into(),
        mcp_prefix: id.replace('.', "_"),
        families: vec!["vcs".into()],
        ..Default::default()
    }
}

#[test]
fn open_creates_private_layout_and_is_idempotent() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    let ws = Workspace::open(&root).unwrap();
    assert_eq!(ws.root(), fs::canonicalize(&root).unwrap());
    for sub in [
        "",
        "targets",
        "cas",
        "cas/sha256",
        "sessions",
        "scratch",
        "audit",
        "state",
    ] {
        assert_eq!(mode(&root.join(sub)), 0o700, "{sub}");
    }
    assert_eq!(ws.sessions_dir(), ws.root().join("sessions"));
    assert_eq!(ws.scratch_dir(), ws.root().join("scratch"));
    assert_eq!(ws.target_dir("git"), ws.root().join("targets/git"));
    drop(ws);
    Workspace::open(&root).unwrap();
}

#[test]
fn t6_open_refuses_loose_mode_and_symlinked_root() {
    let d = tempfile::tempdir().unwrap();
    let loose = d.path().join("loose");
    fs::create_dir(&loose).unwrap();
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o777)).unwrap();
    let err = Workspace::open(&loose).err().expect("0777 refused");
    assert_eq!(err.code, pb::ErrorCode::ConfigurationInvalid);
    assert!(err.message.contains("0777"), "{}", err.message);

    let real = d.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    let link = d.path().join("link");
    symlink(&real, &link).unwrap();
    let err = Workspace::open(&link).err().expect("symlinked root refused");
    assert_eq!(err.code, pb::ErrorCode::ConfigurationInvalid);
    assert!(err.message.contains("symlink"), "{}", err.message);

    // Group/other-writable (but not 0777) is refused too; group/other-readable roots are accepted.
    fs::set_permissions(&real, fs::Permissions::from_mode(0o770)).unwrap();
    assert!(Workspace::open(&real).is_err());
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
    Workspace::open(&real).unwrap();

    // A symlinked or loose private subdirectory is refused.
    let ws2 = d.path().join("ws2");
    Workspace::open(&ws2).unwrap();
    fs::set_permissions(ws2.join("state"), fs::Permissions::from_mode(0o750)).unwrap();
    assert!(Workspace::open(&ws2).is_err());
    fs::set_permissions(ws2.join("state"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir(ws2.join("sessions")).unwrap();
    symlink(&real, ws2.join("sessions")).unwrap();
    assert!(Workspace::open(&ws2).is_err());
}

#[test]
fn t7_hmac_key_is_0600_and_stable() {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    let key_path = d.path().join("state/hmac.key");
    assert!(!key_path.exists(), "created on first use only");
    let k1 = ws.hmac_key().unwrap();
    assert_eq!(mode(&key_path), 0o600);
    assert_eq!(fs::read(&key_path).unwrap(), k1);
    assert_ne!(k1, [0u8; 32]);
    assert_eq!(ws.hmac_key().unwrap(), k1);
    drop(ws);
    let ws = Workspace::open(d.path()).unwrap();
    assert_eq!(ws.hmac_key().unwrap(), k1);
    // No temp files left behind.
    let names: Vec<_> = fs::read_dir(d.path().join("state"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["hmac.key"]);
    // A key readable by others, or of the wrong length, is refused.
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(ws.hmac_key().unwrap_err().code, pb::ErrorCode::ConfigurationInvalid);
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&key_path, b"short").unwrap();
    assert_eq!(ws.hmac_key().unwrap_err().code, pb::ErrorCode::ConfigurationInvalid);
}

#[test]
fn hmac_key_concurrent_first_use_agrees() {
    let d = tempfile::tempdir().unwrap();
    let ws = std::sync::Arc::new(Workspace::open(d.path()).unwrap());
    let keys: Vec<_> = (0..8)
        .map(|_| {
            let ws = std::sync::Arc::clone(&ws);
            std::thread::spawn(move || ws.hmac_key().unwrap())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    assert!(keys.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn t5_write_lock_read_lock_write_lock_is_byte_identical() {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    let mut lock = pb::CapabilityLock {
        schema: "aa.lock/v1".into(),
        target: Some(target("git")),
        capabilities: vec![pb::Capability {
            id: "git.status".into(),
            input_schema_json: r#"{"type":"object","properties":{"repo":{"type":"string"}}}"#.into(),
            ..Default::default()
        }],
        rule_versions: [("rule:cli-verbs".to_string(), "1".to_string())].into_iter().collect(),
        overlay_sha256: digest::sha256_hex(b"overlay"),
        lock_digest: String::new(),
    };
    aa_core::manifest::finalize_digest(&mut lock).unwrap();
    let sha1 = ws.write_lock(&lock).unwrap();
    let path = ws.target_dir("git").join("capabilities.lock.json");
    let bytes1 = fs::read(&path).unwrap();
    assert_eq!(sha1, digest::sha256_hex(&bytes1));
    assert_eq!(bytes1, aa_core::manifest::lock_to_json(&lock).unwrap());
    assert_eq!(bytes1.last(), Some(&b'\n'));

    let back = ws.read_lock("git").unwrap().expect("lock present");
    assert_eq!(back, lock);
    let sha2 = ws.write_lock(&back).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes1, "byte-identical rewrite");
    assert_eq!(sha2, sha1);
    assert!(ws.read_lock("other").unwrap().is_none());
}

#[test]
fn write_lock_requires_a_target() {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    let err = ws.write_lock(&pb::CapabilityLock::default()).unwrap_err();
    assert_eq!(err.code, pb::ErrorCode::InvalidRequest);
}

#[test]
fn overlay_and_target_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    assert!(ws.list_targets().unwrap().is_empty());
    assert!(ws.read_overlay("git").unwrap().is_none());
    assert!(ws.read_target("git").unwrap().is_none());

    let yaml = "schema: aa.overlay/v1\ntarget:\n  id: git\n";
    let sha = ws.write_overlay("git", yaml).unwrap();
    assert_eq!(sha, digest::sha256_hex(yaml.as_bytes()));
    assert_eq!(ws.read_overlay("git").unwrap().as_deref(), Some(yaml));
    // Atomic replace.
    ws.write_overlay("git", "schema: aa.overlay/v1\n").unwrap();
    assert_eq!(
        ws.read_overlay("git").unwrap().as_deref(),
        Some("schema: aa.overlay/v1\n")
    );

    let t = target("cli-anything.libreoffice");
    ws.write_target(&t).unwrap();
    assert_eq!(ws.read_target("cli-anything.libreoffice").unwrap(), Some(t.clone()));
    let on_disk = fs::read(ws.target_dir("cli-anything.libreoffice").join("target.json")).unwrap();
    assert_eq!(on_disk, [digest::jcs(&t).unwrap(), b"\n".to_vec()].concat());
    assert_eq!(ws.list_targets().unwrap(), ["cli-anything.libreoffice", "git"]);

    // Target ids are validated: no path traversal.
    for bad in ["../evil", "Git", "", "a/b", ".hidden"] {
        assert_eq!(
            ws.write_overlay(bad, "x").unwrap_err().code,
            pb::ErrorCode::InvalidRequest,
            "{bad:?}"
        );
        assert!(ws.read_target(bad).is_err());
    }
    assert!(ws.write_target(&target("../evil")).is_err());
    assert!(!d.path().join("evil").exists());
    // A stray file or invalid directory name under targets/ is not listed.
    fs::write(d.path().join("targets/README"), b"x").unwrap();
    fs::create_dir(d.path().join("targets/Not_Valid")).unwrap();
    assert_eq!(ws.list_targets().unwrap().len(), 2);
    // A target.json naming another target is refused.
    fs::create_dir(ws.target_dir("gh")).unwrap();
    fs::write(
        ws.target_dir("gh").join("target.json"),
        digest::jcs(&target("git")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        ws.read_target("gh").unwrap_err().code,
        pb::ErrorCode::ConfigurationInvalid
    );
}

#[test]
fn harvest_index_and_cas() {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    assert!(ws.read_harvest_index("git").unwrap().is_empty());
    let raw = b"usage: git [-v | --version] ...";
    let sha = ws.cas().put(raw).unwrap();
    assert!(ws.cas().contains(&sha));
    assert_eq!(ws.cas().get(&sha).unwrap(), raw);
    assert!(d.path().join("cas/sha256").join(&sha[..2]).join(&sha[2..]).is_file());
    let sources = vec![
        pb::SourceRef {
            harvester: "help-text@1".into(),
            uri: "exec:git --help".into(),
            sha256: sha.clone(),
            authority: 1,
            harvested_at: Some(pbjson_types::Timestamp {
                seconds: 1_790_000_000,
                nanos: 5,
            }),
            action_id: "act_01".into(),
        },
        pb::SourceRef {
            harvester: "cobra-complete@1".into(),
            sha256: digest::sha256_hex(b"other"),
            ..Default::default()
        },
    ];
    ws.write_harvest_index("git", &sources).unwrap();
    assert_eq!(ws.read_harvest_index("git").unwrap(), sources);
    ws.write_harvest_index("git", &[]).unwrap();
    assert!(ws.read_harvest_index("git").unwrap().is_empty());
}

#[test]
fn verification_records_by_longest_target_prefix() {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    let rec = |cap: &str, lane: pb::Lane| pb::VerificationRecord {
        capability_id: cap.into(),
        lane: lane as i32,
        status: pb::LaneStatus::Passed as i32,
        non_claims: vec!["does not establish write safety".into()],
        ..Default::default()
    };
    // Unregistered target.
    assert_eq!(
        ws.write_verification(&rec("git.status", pb::Lane::Static))
            .unwrap_err()
            .code,
        pb::ErrorCode::UnresolvedSelector
    );
    ws.write_target(&target("cli-anything")).unwrap();
    ws.write_target(&target("cli-anything.libreoffice")).unwrap();
    ws.write_target(&target("git")).unwrap();

    ws.write_verification(&rec("git.status", pb::Lane::Static)).unwrap();
    ws.write_verification(&rec("git.status", pb::Lane::Parser)).unwrap();
    let explicit = pb::VerificationRecord {
        record_id: format!("vr_{}", "a".repeat(64)),
        ..rec("git.log", pb::Lane::SandboxDiff)
    };
    ws.write_verification(&explicit).unwrap();
    ws.write_verification(&rec("cli-anything.libreoffice.writer.new", pb::Lane::Static))
        .unwrap();

    let git = ws.read_verifications("git").unwrap();
    assert_eq!(git.len(), 3);
    assert_eq!(git[0], explicit, "ordered by capability id, then record id");
    for r in &git[1..] {
        assert_eq!(r.capability_id, "git.status");
        assert_eq!(
            r.record_id,
            format!("vr_{}", digest::self_digest(r, |r| r.record_id.clear()).unwrap())
        );
        assert!(
            ws.target_dir("git")
                .join("verification/git.status")
                .join(format!("{}.json", r.record_id))
                .is_file()
        );
    }
    // Longest registered prefix wins: the libreoffice record lives under the dotted target.
    assert_eq!(ws.read_verifications("cli-anything.libreoffice").unwrap().len(), 1);
    assert!(ws.read_verifications("cli-anything").unwrap().is_empty());
    // Rewriting the same record is idempotent.
    ws.write_verification(&rec("git.status", pb::Lane::Static)).unwrap();
    assert_eq!(ws.read_verifications("git").unwrap().len(), 3);
    // Unsafe ids are refused.
    assert!(ws.write_verification(&rec("git/../../x", pb::Lane::Static)).is_err());
    let bad_id = pb::VerificationRecord {
        record_id: "../x".into(),
        ..rec("git.status", pb::Lane::Static)
    };
    assert!(ws.write_verification(&bad_id).is_err());
}

#[test]
fn audit_and_halt_open_from_the_workspace_root_or_their_dirs() {
    use aa_core::api::AuditSink;
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::open(d.path()).unwrap();
    {
        let (log, _) = AuditLog::open(ws.root()).unwrap();
        log.append_admin(
            &pb::Principal::default(),
            0,
            pb::AdminEvent {
                kind: pb::AdminEventKind::EngineStart as i32,
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert!(ws.root().join("audit/segment-000001.jsonl").is_file());
    let (log, boot) = AuditLog::open(&ws.root().join("audit")).unwrap();
    assert_eq!(boot.entries, 1);
    assert_eq!(log.head().0, 1);

    let halted = pb::HaltState {
        halted: true,
        halt_epoch: 2,
        ..Default::default()
    };
    HaltStore::open(ws.root()).unwrap().save(&halted).unwrap();
    assert!(ws.root().join("state/halt.json").is_file());
    assert_eq!(HaltStore::open(&ws.root().join("state")).unwrap().load(), halted);
}
