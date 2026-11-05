//! IdentityCache acceptance (build-plan §3.2 acceptance 6): a rewritten fake binary → TARGET_IDENTITY_CHANGED.

use aa_core::pb;
use aa_exec::{IdentityCache, file_identity, identity::file_identity_as_invoked, identity_digest};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};

fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn pinned(path: &Path) -> pb::TargetIdentity {
    let mut id = pb::TargetIdentity {
        files: vec![file_identity("executable", path, None).unwrap()],
        ..Default::default()
    };
    id.identity_digest = identity_digest(&id).unwrap();
    id
}

fn assert_changed(r: aa_core::AaResult<()>) {
    let e = r.expect_err("drift must be detected");
    assert_eq!(e.code, pb::ErrorCode::TargetIdentityChanged);
    let rec = e.recovery.expect("recovery");
    assert_eq!(rec.kind, pb::RecoveryKind::Redistill as i32);
    assert!(!e.remediation.is_empty());
}

#[test]
fn rewritten_fake_binary_is_detected() {
    let d = tempfile::tempdir().unwrap();
    let exe = d.path().join("fake-tool");
    write_exe(&exe, "#!/bin/sh\necho v1\n");
    let id = pinned(&exe);
    let cache = IdentityCache::new();
    cache.verify(&id).unwrap();
    cache.verify(&id).unwrap(); // cached stat tuple

    write_exe(&exe, "#!/bin/sh\necho v2 -- evil\n");
    assert_changed(cache.verify(&id));
}

#[test]
fn same_size_rewrite_with_restored_mtime_is_detected() {
    let d = tempfile::tempdir().unwrap();
    let exe = d.path().join("fake-tool");
    write_exe(&exe, "#!/bin/sh\necho v1\n");
    let id = pinned(&exe);
    let cache = IdentityCache::new();
    cache.verify(&id).unwrap();
    let mtime = std::fs::metadata(&exe).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    // Same length, same inode, mtime put back: only ctime (and the content) moved.
    std::fs::write(&exe, "#!/bin/sh\necho v9\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&exe)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_changed(cache.verify(&id));
}

#[test]
fn replacement_symlink_and_removal_are_detected() {
    let d = tempfile::tempdir().unwrap();
    let exe = d.path().join("fake-tool");
    write_exe(&exe, "#!/bin/sh\necho v1\n");
    let id = pinned(&exe);
    let realpath = std::path::PathBuf::from(&id.files[0].realpath);
    let cache = IdentityCache::new();
    cache.verify(&id).unwrap();

    // Atomic replace (new inode) with identical content still verifies: identity is the content.
    let tmp = d.path().join("staged");
    write_exe(&tmp, "#!/bin/sh\necho v1\n");
    std::fs::rename(&tmp, &realpath).unwrap();
    cache.verify(&id).unwrap();

    // A symlink in place of the pinned file: identity is content, so only a different target is drift.
    let same = d.path().join("same");
    write_exe(&same, "#!/bin/sh\necho v1\n");
    std::fs::remove_file(&realpath).unwrap();
    std::os::unix::fs::symlink(&same, &realpath).unwrap();
    cache.verify(&id).unwrap();
    let other = d.path().join("other");
    write_exe(&other, "#!/bin/sh\necho evil\n");
    std::fs::remove_file(&realpath).unwrap();
    std::os::unix::fs::symlink(&other, &realpath).unwrap();
    assert_changed(cache.verify(&id));

    std::fs::remove_file(&realpath).unwrap();
    assert_changed(cache.verify(&id));
}

#[test]
fn venv_style_symlinked_interpreter_pin() {
    let d = tempfile::tempdir().unwrap();
    let real = d.path().join("python3.14");
    write_exe(&real, "#!/bin/sh\necho py\n");
    let bin = d.path().join("venv/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let link = bin.join("python");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let f = file_identity_as_invoked("interpreter", &link, None).unwrap();
    assert_eq!(f.realpath, link.to_string_lossy());
    assert_eq!(f.sha256, file_identity("x", &real, None).unwrap().sha256);
    let id = pb::TargetIdentity {
        files: vec![f],
        ..Default::default()
    };
    let cache = IdentityCache::new();
    cache.verify(&id).unwrap();
    write_exe(&real, "#!/bin/sh\necho upgraded\n");
    assert_changed(cache.verify(&id));
    assert!(file_identity_as_invoked("interpreter", Path::new("venv/bin/python"), None).is_err());
}
