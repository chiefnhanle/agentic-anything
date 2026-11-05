//! Small filesystem helpers shared by the store modules: private directories, atomic writes, ownership checks.

use aa_core::{AaError, AaResult, pb};
use std::{
    fs::{self, DirBuilder, File},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::Path,
};

/// Group/other write bits: never acceptable on any store directory.
pub(crate) const LOOSE_WRITABLE: u32 = 0o022;
/// Any group/other bit: not acceptable on private directories (state, sessions, scratch, audit) and secrets.
pub(crate) const ANY_GROUP_OTHER: u32 = 0o077;

pub(crate) fn io_err(context: impl std::fmt::Display, e: &io::Error) -> AaError {
    AaError::internal(format!("{context}: {e}"))
}

pub(crate) fn config_err(message: impl Into<String>, remediation: impl Into<String>) -> AaError {
    AaError::new(pb::ErrorCode::ConfigurationInvalid, message, remediation)
}

pub(crate) fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Ensures `path` is a real (non-symlink) directory owned by the current user whose mode has none of the
/// `forbidden` bits. Creates it (and missing parents) with mode 0700 when absent.
pub(crate) fn ensure_private_dir(path: &Path, forbidden: u32) -> AaResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .map_err(|e| io_err(format_args!("cannot create {}", path.display()), &e))?;
        }
        Err(e) => return Err(io_err(format_args!("cannot stat {}", path.display()), &e)),
    }
    check_dir(path, forbidden)
}

/// Refuses a symlink, a non-directory, a foreign owner, or a mode with any of the `forbidden` bits.
pub(crate) fn check_dir(path: &Path, forbidden: u32) -> AaResult<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_err(format_args!("cannot stat {}", path.display()), &e))?;
    let shown = path.display();
    if meta.file_type().is_symlink() {
        return Err(config_err(
            format!("{shown} is a symlink; the workspace refuses symlinked directories"),
            "point --workspace at the real directory, not a symlink",
        ));
    }
    if !meta.is_dir() {
        return Err(config_err(
            format!("{shown} is not a directory"),
            "move the file away or choose another --workspace",
        ));
    }
    if meta.uid() != euid() {
        return Err(config_err(
            format!(
                "{shown} is owned by uid {} but the engine runs as uid {}",
                meta.uid(),
                euid()
            ),
            format!("use a workspace owned by the engine user (chown {} {shown})", euid()),
        ));
    }
    let mode = meta.permissions().mode() & 0o7777;
    if mode & forbidden != 0 {
        return Err(config_err(
            format!("{shown} has loose mode {mode:04o}"),
            format!("chmod 700 {shown}"),
        ));
    }
    Ok(())
}

/// `dir` itself if its last component is `name`, otherwise `dir/name` (so both the workspace root and the
/// subdirectory itself are accepted by `AuditLog::open` / `HaltStore::open`).
pub(crate) fn resolve_subdir(dir: &Path, name: &str) -> std::path::PathBuf {
    if dir.file_name().is_some_and(|n| n == name) {
        dir.to_path_buf()
    } else {
        dir.join(name)
    }
}

/// Reads a whole file; `None` when it does not exist.
pub(crate) fn read_optional(path: &Path) -> AaResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(format_args!("cannot read {}", path.display()), &e)),
    }
}

/// Best-effort fsync of a directory (makes renames/creates durable). Some filesystems refuse directory fsync.
pub(crate) fn fsync_dir(dir: &Path) {
    if let Ok(f) = File::open(dir)
        && let Err(e) = f.sync_all()
    {
        tracing::debug!(dir = %dir.display(), error = %e, "directory fsync not supported");
    }
}

/// Atomically replaces `path` with `bytes` (temp file in the same directory, fsync, rename, fsync dir).
/// The file is created 0600 and the temp file is removed on failure.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> AaResult<()> {
    let dir = path
        .parent()
        .ok_or_else(|| AaError::internal(format!("{} has no parent directory", path.display())))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempfile_in(dir)
        .map_err(|e| io_err(format_args!("cannot create a temp file in {}", dir.display()), &e))?;
    tmp.write_all(bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|e| io_err(format_args!("cannot write {}", path.display()), &e))?;
    tmp.persist(path)
        .map_err(|e| io_err(format_args!("cannot rename into {}", path.display()), &e.error))?;
    fsync_dir(dir);
    Ok(())
}

/// Proto3-JSON (pbjson) canonicalized with JCS, plus a trailing newline: the on-disk form of every record file.
pub(crate) fn canonical_json_line<T: serde::Serialize + ?Sized>(v: &T) -> AaResult<Vec<u8>> {
    let mut bytes = aa_core::digest::jcs(v)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Parses a proto3-JSON record file, naming the file in the error.
pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(path: &Path, bytes: &[u8]) -> AaResult<T> {
    serde_json::from_slice(bytes).map_err(|e| {
        AaError::new(
            pb::ErrorCode::ConfigurationInvalid,
            format!("{} is not valid: {e}", path.display()),
            format!("inspect or restore {}", path.display()),
        )
    })
}
