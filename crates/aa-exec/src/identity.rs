//! Identity pinning: realpath + sha256 + version banner per file, re-verified before every dispatch.
//!
//! [`IdentityCache::verify`] keys each pinned file by its stat tuple `(dev, ino, size, mtime, ctime)` and only
//! rehashes when the tuple changed. `ctime` is included on top of the spec's `(dev, ino, size, mtime)` because
//! `mtime` can be restored with `touch -r` after an in-place rewrite, while `ctime` cannot be set by a user.

use aa_core::{AaError, AaResult, pb};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    os::unix::{fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const BANNER_DEADLINE: Duration = Duration::from_secs(5);
const BANNER_MAX: usize = 200;

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Runs `[realpath] + version_argv` (never a shell) in a scratch dir with a scrubbed env, its own process group
/// and a 5 s deadline, and returns the first non-empty line of stdout (or stderr), trimmed and bounded.
fn version_banner(exe: &Path, args: &[&str]) -> AaResult<String> {
    let scratch = tempfile::tempdir().map_err(|e| AaError::internal(format!("cannot create scratch dir: {e}")))?;
    let home = scratch.path().to_path_buf();
    let mut child = std::process::Command::new(exe)
        .args(args)
        .current_dir(&home)
        .env_clear()
        .env("PATH", crate::supervisor::DEFAULT_PATH)
        .env("HOME", &home)
        .env("TMPDIR", &home)
        .env("LANG", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| AaError::internal(format!("cannot run {} for its version banner: {e}", exe.display())))?;
    let pgid = child.id() as i32;
    fn drain(r: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            if let Some(mut r) = r {
                let mut buf = [0u8; 8192];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let room = (64 * 1024usize).saturating_sub(kept.len()).min(n);
                    kept.extend_from_slice(&buf[..room]);
                }
            }
            kept
        })
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let until = Instant::now() + BANNER_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), nix::sys::signal::Signal::SIGKILL);
                let _ = child.wait();
                return Err(AaError::new(
                    pb::ErrorCode::Timeout,
                    format!("version banner of {} timed out", exe.display()),
                    "check the executable, or register it without a version argv",
                ));
            }
        }
    }
    // Leftover group members would hold the pipes open; the group is ours to clean up.
    let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), nix::sys::signal::Signal::SIGKILL);
    let o = out
        .join()
        .map_err(|_| AaError::internal("version banner reader panicked"))?;
    let e = err
        .join()
        .map_err(|_| AaError::internal("version banner reader panicked"))?;
    let pick = |b: &[u8]| {
        String::from_utf8_lossy(b)
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(BANNER_MAX).collect::<String>())
    };
    Ok(pick(&o).or_else(|| pick(&e)).unwrap_or_default())
}

pub fn file_identity(
    role: &str,
    path: &std::path::Path,
    version_argv: Option<&[&str]>,
) -> aa_core::AaResult<aa_core::pb::FileIdentity> {
    if role.is_empty() {
        return Err(AaError::invalid("identity role must not be empty"));
    }
    let realpath = std::fs::canonicalize(path).map_err(|e| {
        AaError::new(
            pb::ErrorCode::TargetUnavailable,
            format!("cannot resolve {}: {e}", path.display()),
            "check the path given to aa target add",
        )
    })?;
    let md = std::fs::metadata(&realpath)?;
    if !md.is_file() {
        return Err(AaError::invalid(format!(
            "{} is not a regular file",
            realpath.display()
        )));
    }
    let sha256 = sha256_file(&realpath)?;
    let version_banner = match version_argv {
        Some(args) => version_banner(&realpath, args)?,
        None => String::new(),
    };
    Ok(pb::FileIdentity {
        role: role.to_string(),
        realpath: realpath.to_string_lossy().into_owned(),
        sha256,
        version_banner,
    })
}

/// Like [`file_identity`], but keeps the absolute `path` as given instead of its realpath (the sha256 is still
/// of the resolved file). For interpreters that locate their environment from the invoked path: exec'ing the
/// realpath of `<venv>/bin/python` loses the venv (verified on this Mac with uv + Homebrew Python 3.14).
pub fn file_identity_as_invoked(
    role: &str,
    path: &std::path::Path,
    version_argv: Option<&[&str]>,
) -> aa_core::AaResult<aa_core::pb::FileIdentity> {
    if !path.is_absolute() {
        return Err(AaError::invalid(format!("{} is not an absolute path", path.display())));
    }
    let mut id = file_identity(role, path, None)?;
    id.realpath = path.to_string_lossy().into_owned();
    if let Some(args) = version_argv {
        id.version_banner = version_banner(path, args)?;
    }
    Ok(id)
}

pub fn identity_digest(id: &aa_core::pb::TargetIdentity) -> aa_core::AaResult<String> {
    aa_core::digest::self_digest(id, |i| i.identity_digest.clear())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatKey {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl StatKey {
    fn of(md: &std::fs::Metadata) -> Self {
        Self {
            dev: md.dev(),
            ino: md.ino(),
            size: md.size(),
            mtime: (md.mtime(), md.mtime_nsec()),
            ctime: (md.ctime(), md.ctime_nsec()),
        }
    }
}

/// (dev, ino, size, mtime) -> sha256.
pub struct IdentityCache {
    entries: Mutex<HashMap<PathBuf, (StatKey, String)>>,
}

impl Default for IdentityCache {
    fn default() -> Self {
        Self::new()
    }
}

fn changed(msg: String) -> AaError {
    AaError::new(
        pb::ErrorCode::TargetIdentityChanged,
        msg,
        "the target changed since it was pinned; re-run discovery and verification: aa discover <target> && aa verify <target>",
    )
    .with_recovery(pb::RecoveryKind::Redistill, "DistillService.Harvest")
}

#[allow(clippy::new_without_default)]
impl IdentityCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// TARGET_IDENTITY_CHANGED + REDISTILL on drift.
    pub fn verify(&self, id: &aa_core::pb::TargetIdentity) -> aa_core::AaResult<()> {
        for f in &id.files {
            let p = PathBuf::from(&f.realpath);
            // Identity is content: the pinned path is resolved (a venv's `bin/python` symlink stays valid as long
            // as what it resolves to hashes to the pinned sha256), and the resolved file is stat-cached.
            let canon = std::fs::canonicalize(&p)
                .map_err(|e| changed(format!("pinned file {} ({}) is gone: {e}", f.realpath, f.role)))?;
            let md = std::fs::symlink_metadata(&canon)
                .map_err(|e| changed(format!("cannot stat pinned file {}: {e}", f.realpath)))?;
            if !md.is_file() {
                return Err(changed(format!(
                    "pinned file {} is no longer a regular file",
                    f.realpath
                )));
            }
            let p = canon;
            let key = StatKey::of(&md);
            let cached = {
                let entries = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                entries.get(&p).filter(|(k, _)| *k == key).map(|(_, sha)| sha.clone())
            };
            let sha = match cached {
                Some(sha) => sha,
                None => {
                    let sha = sha256_file(&p).map_err(|e| changed(format!("cannot hash {}: {e}", f.realpath)))?;
                    // Re-stat: if the file changed while hashing, do not cache the result.
                    if std::fs::symlink_metadata(&p).map(|m| StatKey::of(&m)).ok() == Some(key) {
                        self.entries
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(p.clone(), (key, sha.clone()));
                    }
                    sha
                }
            };
            if sha != f.sha256 {
                return Err(changed(format!(
                    "pinned file {} ({}) changed: sha256 {} != pinned {}",
                    f.realpath, f.role, sha, f.sha256
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_identity_pins_realpath_sha_and_banner() {
        let id = file_identity("executable", Path::new("/bin/sh"), None).unwrap();
        assert_eq!(id.role, "executable");
        assert!(Path::new(&id.realpath).is_absolute());
        assert_eq!(id.sha256.len(), 64);
        assert!(id.version_banner.is_empty());

        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        std::fs::write(&f, b"abc").unwrap();
        let link = d.path().join("l");
        std::os::unix::fs::symlink(&f, &link).unwrap();
        let id = file_identity("data", &link, None).unwrap();
        assert_eq!(id.realpath, std::fs::canonicalize(&f).unwrap().to_string_lossy());
        assert_eq!(
            id.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(file_identity("x", d.path(), None).is_err());
        assert!(file_identity("", &f, None).is_err());
    }

    #[test]
    fn banner_from_real_git_when_present() {
        let Ok(git) = which::which("git") else {
            eprintln!("SKIPPED: git not on PATH");
            return;
        };
        let id = file_identity("executable", &git, Some(&["--version"])).unwrap();
        assert!(id.version_banner.starts_with("git version"), "{}", id.version_banner);
    }

    #[test]
    fn identity_digest_ignores_its_own_field() {
        let mut id = pb::TargetIdentity {
            files: vec![pb::FileIdentity {
                role: "executable".into(),
                realpath: "/x".into(),
                sha256: "00".into(),
                version_banner: String::new(),
            }],
            ..Default::default()
        };
        let a = identity_digest(&id).unwrap();
        id.identity_digest = a.clone();
        assert_eq!(identity_digest(&id).unwrap(), a);
        id.files[0].sha256 = "01".into();
        assert_ne!(identity_digest(&id).unwrap(), a);
    }
}
