//! Content-addressed store: `cas/sha256/ab/cdef...`, immutable.
//!
//! Blobs are written once (temp file, fsync, no-clobber rename) and made read-only (0400). Reads re-hash the
//! bytes, so a corrupted blob is reported instead of served.

use crate::fsutil::{self, io_err};
use aa_core::{AaError, AaResult, digest::sha256_hex, pb};
use std::{fs, io, io::Write, os::unix::fs::PermissionsExt, path::PathBuf};

/// The workspace CAS rooted at `<workspace>/cas/sha256`.
pub struct Cas {
    root: PathBuf,
}

impl Cas {
    /// `root` is the `sha256` directory; it must already exist (the workspace creates it).
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Stores `bytes` and returns their lowercase-hex sha256. Idempotent: an existing blob is left untouched.
    pub fn put(&self, bytes: &[u8]) -> AaResult<String> {
        let digest = sha256_hex(bytes);
        let path = self.path_of(&digest);
        if path.is_file() {
            return Ok(digest);
        }
        let fan = path
            .parent()
            .ok_or_else(|| AaError::internal("CAS path has no parent"))?
            .to_path_buf();
        fsutil::ensure_private_dir(&fan, fsutil::LOOSE_WRITABLE)?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".tmp-")
            .tempfile_in(&fan)
            .map_err(|e| io_err(format_args!("cannot create a temp file in {}", fan.display()), &e))?;
        tmp.write_all(bytes)
            .and_then(|()| tmp.as_file().sync_all())
            .and_then(|()| tmp.as_file().set_permissions(fs::Permissions::from_mode(0o400)))
            .map_err(|e| io_err(format_args!("cannot write CAS blob {digest}"), &e))?;
        match tmp.persist_noclobber(&path) {
            Ok(_) => {}
            // A concurrent writer stored the same content first: same digest, same bytes.
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_err(format_args!("cannot store CAS blob {digest}"), &e.error)),
        }
        fsutil::fsync_dir(&fan);
        Ok(digest)
    }

    /// The blob with this digest. Missing => `UNRESOLVED_SELECTOR`; content not matching its digest => `INTERNAL`.
    pub fn get(&self, sha256: &str) -> AaResult<Vec<u8>> {
        validate_digest(sha256)?;
        let path = self.path_of(sha256);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(AaError::new(
                    pb::ErrorCode::UnresolvedSelector,
                    format!("no CAS blob with sha256 {sha256}"),
                    "re-run the step that produced it (e.g. aa discover <target>)",
                ));
            }
            Err(e) => return Err(io_err(format_args!("cannot read CAS blob {sha256}"), &e)),
        };
        let actual = sha256_hex(&bytes);
        if actual != sha256 {
            return Err(AaError::internal(format!(
                "CAS blob {sha256} is corrupted (content hashes to {actual})"
            )));
        }
        Ok(bytes)
    }

    /// `true` if a blob with this (well-formed) digest is stored.
    pub fn contains(&self, sha256: &str) -> bool {
        validate_digest(sha256).is_ok() && self.path_of(sha256).is_file()
    }

    fn path_of(&self, sha256: &str) -> PathBuf {
        let (fan, rest) = sha256.split_at(2);
        self.root.join(fan).join(rest)
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &std::path::Path {
        &self.root
    }
}

fn validate_digest(sha256: &str) -> AaResult<()> {
    if sha256.len() == 64 && sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(AaError::invalid(format!(
            "{sha256:?} is not a sha256 digest (64 lowercase hex characters)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cas() -> (tempfile::TempDir, Cas) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sha256");
        fs::create_dir(&root).unwrap();
        (dir, Cas::new(root))
    }

    #[test]
    fn put_get_contains_roundtrip() {
        let (_d, cas) = cas();
        let d = cas.put(b"hello").unwrap();
        assert_eq!(d, sha256_hex(b"hello"));
        assert!(cas.contains(&d));
        assert_eq!(cas.get(&d).unwrap(), b"hello");
        // Idempotent.
        assert_eq!(cas.put(b"hello").unwrap(), d);
        let path = cas.root().join(&d[..2]).join(&d[2..]);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o400);
        // Empty blobs are fine.
        let e = cas.put(b"").unwrap();
        assert_eq!(cas.get(&e).unwrap(), b"");
    }

    #[test]
    fn missing_malformed_and_corrupted() {
        let (_d, cas) = cas();
        let missing = sha256_hex(b"nope");
        assert!(!cas.contains(&missing));
        assert_eq!(cas.get(&missing).unwrap_err().code, pb::ErrorCode::UnresolvedSelector);
        for bad in ["", "../etc/passwd", &missing.to_uppercase(), &missing[..63]] {
            assert!(!cas.contains(bad));
            assert_eq!(cas.get(bad).unwrap_err().code, pb::ErrorCode::InvalidRequest);
        }
        let d = cas.put(b"original").unwrap();
        let path = cas.root().join(&d[..2]).join(&d[2..]);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"tampered").unwrap();
        assert_eq!(cas.get(&d).unwrap_err().code, pb::ErrorCode::Internal);
    }
}
