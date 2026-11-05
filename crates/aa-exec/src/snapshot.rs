//! Tree snapshots (rel path -> blake3) and diffs for L3 and post-observation.
//!
//! - Regular files map to the blake3 hex of their contents.
//! - Directories map to `"dir"`, so creating or removing an empty directory is observed too.
//! - Symlinks are never followed; they map to `"symlink:" + blake3(target)`.
//! - Unreadable files map to `"unreadable:<size>:<mtime_ns>"`, so a change is still observed.
//! - A `root` that is a regular file is snapshotted as one entry keyed by its file name; a missing root is an
//!   empty snapshot (so "created" is observable for a state handle that did not exist yet).
//!
//! `ignore` entries are relative paths; an entry matches itself and everything below it.

use aa_core::{AaError, pb};
use std::{
    collections::BTreeMap,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Relative path -> blake3 hex.
pub struct TreeSnapshot(pub std::collections::BTreeMap<std::path::PathBuf, String>);

fn file_value(path: &Path, md: &std::fs::Metadata) -> String {
    let hashed = std::fs::File::open(path).and_then(|f| {
        let mut h = blake3::Hasher::new();
        h.update_reader(f)?;
        Ok(h.finalize().to_hex().to_string())
    });
    hashed.unwrap_or_else(|_| {
        let mtime_ns = md.mtime() as i128 * 1_000_000_000 + md.mtime_nsec() as i128;
        format!("unreadable:{}:{mtime_ns}", md.len())
    })
}

fn entry_value(path: &Path, md: &std::fs::Metadata) -> String {
    let ft = md.file_type();
    if ft.is_symlink() {
        let target = std::fs::read_link(path)
            .map(|t| t.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("symlink:{}", blake3::hash(target.as_bytes()).to_hex())
    } else if ft.is_dir() {
        "dir".to_string()
    } else if ft.is_file() {
        file_value(path, md)
    } else {
        format!("special:{:o}", md.mode())
    }
}

pub fn snapshot(root: &std::path::Path, ignore: &[&str]) -> aa_core::AaResult<TreeSnapshot> {
    let mut map = BTreeMap::new();
    let md = match std::fs::symlink_metadata(root) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TreeSnapshot(map)),
        Err(e) => return Err(AaError::internal(format!("cannot stat {}: {e}", root.display()))),
    };
    if !md.is_dir() {
        let name = root.file_name().map(PathBuf::from).unwrap_or_default();
        map.insert(name, entry_value(root, &md));
        return Ok(TreeSnapshot(map));
    }
    let ignored: Vec<PathBuf> = ignore
        .iter()
        .map(|s| PathBuf::from(s.trim_start_matches('/')))
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    let mut walker = walkdir::WalkDir::new(root).follow_links(false).min_depth(1).into_iter();
    while let Some(next) = walker.next() {
        let entry = match next {
            Ok(e) => e,
            Err(e) => {
                // A directory we cannot read: record it so a change of readability is at least visible.
                if let Some(p) = e.path()
                    && let Ok(rel) = p.strip_prefix(root)
                {
                    map.insert(rel.to_path_buf(), "unreadable-dir".to_string());
                }
                continue;
            }
        };
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_path_buf();
        if ignored.iter().any(|i| rel.starts_with(i)) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        let md = match entry.path().symlink_metadata() {
            Ok(md) => md,
            Err(_) => continue, // vanished during the walk
        };
        map.insert(rel, entry_value(entry.path(), &md));
    }
    Ok(TreeSnapshot(map))
}

pub fn diff(before: &TreeSnapshot, after: &TreeSnapshot, root_label: &str) -> Vec<aa_core::pb::ObservedFileEffect> {
    let mut out = Vec::new();
    let effect = |path: &Path, change: pb::FileChange, b: &str, a: &str| pb::ObservedFileEffect {
        path: path.to_string_lossy().into_owned(),
        root: root_label.to_string(),
        change: change as i32,
        before_sha256: b.to_string(),
        after_sha256: a.to_string(),
    };
    for (path, b) in &before.0 {
        match after.0.get(path) {
            None => out.push(effect(path, pb::FileChange::Deleted, b, "")),
            Some(a) if a != b => out.push(effect(path, pb::FileChange::Modified, b, a)),
            Some(_) => {}
        }
    }
    for (path, a) in &after.0 {
        if !before.0.contains_key(path) {
            out.push(effect(path, pb::FileChange::Created, "", a));
        }
    }
    out.sort_by(|x, y| x.path.cmp(&y.path));
    out
}

/// Convenience used by the binding and the lanes: snapshot or an empty snapshot plus a logged warning.
pub(crate) fn snapshot_or_empty(root: &Path, ignore: &[&str]) -> TreeSnapshot {
    snapshot(root, ignore).unwrap_or_else(|e| {
        tracing::warn!(root = %root.display(), error = %e, "snapshot failed");
        TreeSnapshot(BTreeMap::new())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_modified_deleted_and_ignore() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("a/b")).unwrap();
        std::fs::create_dir_all(r.join("tmp")).unwrap();
        std::fs::write(r.join("a/b/keep"), b"1").unwrap();
        std::fs::write(r.join("a/mod"), b"1").unwrap();
        std::fs::write(r.join("a/del"), b"1").unwrap();
        std::fs::write(r.join("tmp/x"), b"1").unwrap();
        let before = snapshot(r, &["tmp"]).unwrap();
        assert!(before.0.contains_key(Path::new("a/b/keep")));
        assert!(!before.0.keys().any(|k| k.starts_with("tmp")));
        assert_eq!(before.0[Path::new("a")], "dir");
        assert_eq!(before.0[Path::new("a/b/keep")], blake3::hash(b"1").to_hex().to_string());

        std::fs::write(r.join("a/mod"), b"2").unwrap();
        std::fs::remove_file(r.join("a/del")).unwrap();
        std::fs::write(r.join("a/new"), b"n").unwrap();
        std::fs::create_dir(r.join("emptydir")).unwrap();
        std::fs::write(r.join("tmp/y"), b"ignored").unwrap();
        let after = snapshot(r, &["tmp"]).unwrap();
        let d = diff(&before, &after, "private:cwd");
        let got: Vec<(String, i32)> = d.iter().map(|e| (e.path.clone(), e.change)).collect();
        assert_eq!(
            got,
            vec![
                ("a/del".to_string(), pb::FileChange::Deleted as i32),
                ("a/mod".to_string(), pb::FileChange::Modified as i32),
                ("a/new".to_string(), pb::FileChange::Created as i32),
                ("emptydir".to_string(), pb::FileChange::Created as i32),
            ]
        );
        assert!(d.iter().all(|e| e.root == "private:cwd"));
        let m = d.iter().find(|e| e.path == "a/mod").unwrap();
        assert_eq!(m.before_sha256, blake3::hash(b"1").to_hex().to_string());
        assert_eq!(m.after_sha256, blake3::hash(b"2").to_hex().to_string());
        assert!(diff(&after, &snapshot(r, &["tmp"]).unwrap(), "x").is_empty());
    }

    #[test]
    fn symlinks_are_not_followed_and_files_and_missing_roots() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"s").unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
        let s = snapshot(d.path(), &[]).unwrap();
        assert_eq!(s.0.len(), 1);
        assert!(s.0[Path::new("link")].starts_with("symlink:"));

        let f = d.path().join("state.json");
        assert!(snapshot(&f, &[]).unwrap().0.is_empty());
        std::fs::write(&f, b"{}").unwrap();
        let s = snapshot(&f, &[]).unwrap();
        assert_eq!(s.0.keys().next().unwrap(), Path::new("state.json"));
    }
}
