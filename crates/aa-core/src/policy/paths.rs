//! PathScope canonicalization and checks (deny wins).
//!
//! Scope checks compare **canonical** paths component-wise. Roots are canonicalized at check time when they
//! exist (on macOS `/var` and `/tmp` are symlinks into `/private`), so a root written either way matches.

use crate::{AaError, AaResult, pb};
use std::path::{Component, Path, PathBuf};

pub struct PathScope {
    pub write_roots: Vec<std::path::PathBuf>,
    pub read_roots: Vec<std::path::PathBuf>,
    pub deny: Vec<std::path::PathBuf>,
}

fn outside(msg: String) -> AaError {
    AaError::new(
        pb::ErrorCode::PolicyDenied,
        msg,
        "pass a path inside the session's read/write roots: aa describe <id>",
    )
    .with_denial(pb::DenialReason::OutsidePathScope)
}

fn has_dot_components(p: &Path) -> bool {
    p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}

/// realpath (write: realpath of parent + file name); rejects `..` after canonicalization, symlink escapes.
///
/// - The path must be absolute and free of NUL bytes.
/// - Read: the path must exist; every symlink is resolved.
/// - Write: the parent must exist and is resolved; the final component must be a plain name. If the final
///   component already exists as a symlink it is resolved too (a dangling symlink is refused), so the scope
///   check sees the real destination.
pub fn canonicalize_for_scope(p: &std::path::Path, write: bool) -> crate::AaResult<std::path::PathBuf> {
    if p.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(AaError::invalid("path contains a NUL byte"));
    }
    if !p.is_absolute() {
        return Err(AaError::invalid(format!("path {} must be absolute", p.display())));
    }
    if !write {
        let real = std::fs::canonicalize(p)
            .map_err(|e| AaError::invalid(format!("cannot resolve path {}: {e}", p.display())))?;
        return checked(real);
    }
    if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
        let real = std::fs::canonicalize(p).map_err(|e| {
            outside(format!(
                "write path {} is a symlink that does not resolve: {e}",
                p.display()
            ))
        })?;
        return checked(real);
    }
    let name = match p.components().next_back() {
        Some(Component::Normal(n)) => n.to_owned(),
        _ => return Err(AaError::invalid(format!("write path {} has no file name", p.display()))),
    };
    let parent = p
        .parent()
        .ok_or_else(|| AaError::invalid(format!("write path {} has no parent", p.display())))?;
    let real_parent = std::fs::canonicalize(parent)
        .map_err(|e| AaError::invalid(format!("cannot resolve parent of {}: {e}", p.display())))?;
    checked(real_parent.join(name))
}

fn checked(p: PathBuf) -> AaResult<PathBuf> {
    if has_dot_components(&p) || !p.is_absolute() {
        return Err(outside(format!("path {} is not canonical", p.display())));
    }
    Ok(p)
}

/// The canonical form of a root for comparison (falls back to the lexical path if it does not exist).
pub fn canonical_root(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

fn under(p: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|r| p.starts_with(r) || p.starts_with(canonical_root(r)))
}

/// Deny wins; returns the denial reason on failure.
///
/// A write must be under a write root; a read must be under a read root or a write root.
pub fn check(scope: &PathScope, canonical: &std::path::Path, write: bool) -> Result<(), pb::DenialReason> {
    if !canonical.is_absolute() || has_dot_components(canonical) {
        return Err(pb::DenialReason::OutsidePathScope);
    }
    if under(canonical, &scope.deny) {
        return Err(pb::DenialReason::OutsidePathScope);
    }
    let allowed = if write {
        under(canonical, &scope.write_roots)
    } else {
        under(canonical, &scope.read_roots) || under(canonical, &scope.write_roots)
    };
    if allowed {
        Ok(())
    } else {
        Err(pb::DenialReason::OutsidePathScope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_wins_and_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical_root(tmp.path());
        std::fs::create_dir_all(root.join("ro/secret")).unwrap();
        std::fs::create_dir_all(root.join("rw")).unwrap();
        let scope = PathScope {
            write_roots: vec![root.join("rw")],
            read_roots: vec![tmp.path().join("ro")],
            deny: vec![root.join("ro/secret")],
        };
        let ro = canonicalize_for_scope(&tmp.path().join("ro"), false).unwrap();
        assert_eq!(check(&scope, &ro, false), Ok(()));
        assert_eq!(check(&scope, &ro, true), Err(pb::DenialReason::OutsidePathScope));
        let secret = canonicalize_for_scope(&root.join("ro/secret"), false).unwrap();
        assert_eq!(check(&scope, &secret, false), Err(pb::DenialReason::OutsidePathScope));
        let new_file = canonicalize_for_scope(&root.join("rw/new.txt"), true).unwrap();
        assert_eq!(check(&scope, &new_file, true), Ok(()));
        assert_eq!(
            check(&scope, Path::new("/etc"), false),
            Err(pb::DenialReason::OutsidePathScope)
        );
    }

    #[test]
    fn symlink_escape_is_resolved_and_denied() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonical_root(tmp.path());
        std::fs::create_dir_all(root.join("rw")).unwrap();
        std::os::unix::fs::symlink("/etc", root.join("rw/link")).unwrap();
        std::os::unix::fs::symlink(root.join("nowhere"), root.join("rw/dangling")).unwrap();
        let scope = PathScope {
            write_roots: vec![root.join("rw")],
            read_roots: vec![],
            deny: vec![],
        };
        let p = canonicalize_for_scope(&root.join("rw/link/passwd"), false).unwrap();
        assert!(!p.starts_with(&root));
        assert_eq!(check(&scope, &p, false), Err(pb::DenialReason::OutsidePathScope));
        let w = canonicalize_for_scope(&root.join("rw/link"), true).unwrap();
        assert_eq!(check(&scope, &w, true), Err(pb::DenialReason::OutsidePathScope));
        assert!(canonicalize_for_scope(&root.join("rw/dangling"), true).is_err());
        assert!(canonicalize_for_scope(Path::new("relative/x"), false).is_err());
        assert!(canonicalize_for_scope(&root.join("rw/.."), true).is_err());
    }
}
