//! Private per-run roots (0700, realpath'd, owner-checked) with private HOME/TMPDIR/cwd.
//!
//! Layout: `<parent>/aa-run-XXXXXX/{home,cwd,tmp}`. The base directory is created with `tempfile` (unique,
//! race-free name), forced to mode 0700, canonicalized (so Seatbelt `subpath` rules match the resolved path,
//! e.g. `/private/var/...` rather than `/var/...`), and checked with `lstat` to be a real directory owned by
//! the effective uid with no group/other permission bits.

use aa_core::{AaError, AaResult};
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub struct PrivateRoot {
    pub base: std::path::PathBuf,
    pub home: std::path::PathBuf,
    pub cwd: std::path::PathBuf,
    pub tmp: std::path::PathBuf,
}

/// Creates `dir` (which must not exist yet) with mode 0700.
fn mkdir_private(dir: &Path) -> AaResult<()> {
    std::fs::create_dir(dir)
        .map_err(|e| AaError::internal(format!("cannot create private dir {}: {e}", dir.display())))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| AaError::internal(format!("cannot chmod private dir {}: {e}", dir.display())))?;
    Ok(())
}

/// lstat-checks that `dir` is a real directory owned by the effective uid with no group/other bits.
pub(crate) fn check_private_dir(dir: &Path) -> AaResult<()> {
    let md = std::fs::symlink_metadata(dir)
        .map_err(|e| AaError::internal(format!("cannot lstat private dir {}: {e}", dir.display())))?;
    if !md.file_type().is_dir() {
        return Err(AaError::internal(format!(
            "private dir {} is not a directory (symlink or file)",
            dir.display()
        )));
    }
    let euid = nix::unistd::geteuid().as_raw();
    if md.uid() != euid {
        return Err(AaError::internal(format!(
            "private dir {} is owned by uid {} instead of {euid}",
            dir.display(),
            md.uid()
        )));
    }
    if md.mode() & 0o077 != 0 {
        return Err(AaError::internal(format!(
            "private dir {} has group/other permission bits ({:o})",
            dir.display(),
            md.mode() & 0o777
        )));
    }
    Ok(())
}

impl PrivateRoot {
    /// 0700, realpath'd, owner-checked.
    pub fn create(parent: &std::path::Path) -> aa_core::AaResult<Self> {
        std::fs::create_dir_all(parent)
            .map_err(|e| AaError::internal(format!("cannot create scratch parent {}: {e}", parent.display())))?;
        let dir = tempfile::Builder::new()
            .prefix("aa-run-")
            .tempdir_in(parent)
            .map_err(|e| AaError::internal(format!("cannot create private root in {}: {e}", parent.display())))?;
        // From here on `PrivateRoot`'s Drop owns the removal.
        let raw = dir.keep();
        let cleanup = |e: AaError| {
            let _ = std::fs::remove_dir_all(&raw);
            e
        };
        std::fs::set_permissions(&raw, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| cleanup(AaError::internal(format!("cannot chmod private root: {e}"))))?;
        let base = std::fs::canonicalize(&raw)
            .map_err(|e| cleanup(AaError::internal(format!("cannot canonicalize private root: {e}"))))?;
        let root = PrivateRoot {
            home: base.join("home"),
            cwd: base.join("cwd"),
            tmp: base.join("tmp"),
            base,
        };
        // Any failure below drops `root`, which removes the tree.
        check_private_dir(&root.base)?;
        for d in [&root.home, &root.cwd, &root.tmp] {
            mkdir_private(d)?;
            check_private_dir(d)?;
        }
        Ok(root)
    }

    /// PATH, HOME, TMPDIR, LANG=C.UTF-8.
    pub fn base_env(&self, path: &str) -> std::collections::BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), path.to_string());
        env.insert("HOME".to_string(), self.home.to_string_lossy().into_owned());
        env.insert("TMPDIR".to_string(), self.tmp.to_string_lossy().into_owned());
        env.insert("LANG".to_string(), "C.UTF-8".to_string());
        env
    }

    /// The layout a root at `base` has (used by `plan()`, which must not touch the filesystem).
    pub(crate) fn layout(base: &Path) -> (PathBuf, PathBuf, PathBuf) {
        (base.join("home"), base.join("cwd"), base.join("tmp"))
    }
}

impl Drop for PrivateRoot {
    /// Best-effort remove; failures logged.
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.base)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.base.display(), error = %e, "failed to remove private root");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_layout_permissions_and_cleanup() {
        let parent = tempfile::tempdir().unwrap();
        let root = PrivateRoot::create(parent.path()).unwrap();
        assert!(root.base.is_absolute());
        assert_eq!(root.base, std::fs::canonicalize(&root.base).unwrap());
        for d in [&root.base, &root.home, &root.cwd, &root.tmp] {
            let md = std::fs::symlink_metadata(d).unwrap();
            assert!(md.is_dir());
            assert_eq!(md.mode() & 0o777, 0o700, "{}", d.display());
        }
        assert!(root.home.starts_with(&root.base));
        let env = root.base_env("/usr/bin:/bin");
        assert_eq!(env["PATH"], "/usr/bin:/bin");
        assert_eq!(env["HOME"], root.home.to_string_lossy());
        assert_eq!(env["TMPDIR"], root.tmp.to_string_lossy());
        assert_eq!(env["LANG"], "C.UTF-8");
        assert_eq!(env.len(), 4);
        std::fs::write(root.cwd.join("f"), b"x").unwrap();
        let base = root.base.clone();
        drop(root);
        assert!(!base.exists());
    }

    #[test]
    fn two_roots_are_distinct() {
        let parent = tempfile::tempdir().unwrap();
        let a = PrivateRoot::create(parent.path()).unwrap();
        let b = PrivateRoot::create(parent.path()).unwrap();
        assert_ne!(a.base, b.base);
    }

    #[test]
    fn rejects_group_writable_dir() {
        let parent = tempfile::tempdir().unwrap();
        let d = parent.path().join("open");
        std::fs::create_dir(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(check_private_dir(&d).is_err());
        let link = parent.path().join("link");
        std::os::unix::fs::symlink(&d, &link).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(check_private_dir(&d).is_ok());
        assert!(check_private_dir(&link).is_err());
    }
}
