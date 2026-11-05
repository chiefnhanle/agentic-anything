//! `$AA_HOME` layout (architecture §3.2).
//!
//! ```text
//! <root>/
//!   targets/<target-id>/target.aa.yaml           overlay (human-authored)
//!   targets/<target-id>/target.json              registered target (pinned identity)
//!   targets/<target-id>/capabilities.lock.json   canonical lock (aa_core::manifest::lock_to_json)
//!   targets/<target-id>/harvest/index.json       SourceRef list -> CAS digests
//!   targets/<target-id>/verification/<cap-id>/vr_<sha>.json
//!   cas/sha256/ab/cdef...                        immutable blobs
//!   sessions/  scratch/  audit/  state/          private (0700)
//! ```
//!
//! Every directory the workspace creates is 0700. An existing root, `targets/` or `cas/` may be group/other
//! readable (a committed project-local workspace) but never group/other writable; `sessions/`, `scratch/`,
//! `audit/` and `state/` must have no group/other bits at all. Symlinked directories and foreign owners are
//! refused. Record files are written atomically (temp + fsync + rename) as canonical proto3-JSON lines.

use crate::{
    cas::Cas,
    fsutil::{self, ANY_GROUP_OTHER, LOOSE_WRITABLE, io_err},
};
use aa_core::{AaError, AaResult, digest, ids::validate_target_id, pb};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

const OVERLAY: &str = "target.aa.yaml";
const TARGET_JSON: &str = "target.json";
const LOCK_JSON: &str = "capabilities.lock.json";
const HMAC_KEY: &str = "hmac.key";

pub struct Workspace {
    root: PathBuf,
    cas: Cas,
}

impl Workspace {
    /// Creates layout 0700; refuses foreign owner/loose mode.
    pub fn open(root: &Path) -> AaResult<Self> {
        // The root itself must not be a symlink (checked before anything is created or canonicalized).
        fsutil::ensure_private_dir(root, LOOSE_WRITABLE)?;
        let root = fs::canonicalize(root).map_err(|e| io_err(format_args!("cannot resolve {}", root.display()), &e))?;
        for (sub, forbidden) in [
            ("targets", LOOSE_WRITABLE),
            ("cas", LOOSE_WRITABLE),
            ("cas/sha256", LOOSE_WRITABLE),
            ("sessions", ANY_GROUP_OTHER),
            ("scratch", ANY_GROUP_OTHER),
            ("audit", ANY_GROUP_OTHER),
            ("state", ANY_GROUP_OTHER),
        ] {
            fsutil::ensure_private_dir(&root.join(sub), forbidden)?;
        }
        let cas = Cas::new(root.join("cas").join("sha256"));
        Ok(Self { root, cas })
    }

    /// The canonical (symlink-free) workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/targets/<target_id>`. Pure path arithmetic: callers pass a validated id (every reader/writer
    /// below validates).
    pub fn target_dir(&self, target_id: &str) -> PathBuf {
        self.root.join("targets").join(target_id)
    }

    /// Registered target ids (directories under `targets/` with a valid id), sorted.
    pub fn list_targets(&self) -> AaResult<Vec<String>> {
        let dir = self.root.join("targets");
        let mut out = Vec::new();
        for ent in fs::read_dir(&dir).map_err(|e| io_err(format_args!("cannot list {}", dir.display()), &e))? {
            let ent = ent.map_err(|e| io_err(format_args!("cannot list {}", dir.display()), &e))?;
            let is_dir = ent.file_type().is_ok_and(|t| t.is_dir());
            if let Some(name) = ent.file_name().to_str()
                && is_dir
                && validate_target_id(name).is_ok()
            {
                out.push(name.to_owned());
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn read_overlay(&self, target_id: &str) -> AaResult<Option<String>> {
        let path = self.existing_target_dir(target_id)?.join(OVERLAY);
        fsutil::read_optional(&path)?
            .map(|b| {
                String::from_utf8(b).map_err(|_| AaError::invalid(format!("{} is not valid UTF-8", path.display())))
            })
            .transpose()
    }

    /// Atomic; returns sha256.
    pub fn write_overlay(&self, target_id: &str, yaml: &str) -> AaResult<String> {
        let dir = self.ensure_target_dir(target_id)?;
        fsutil::atomic_write(&dir.join(OVERLAY), yaml.as_bytes())?;
        Ok(digest::sha256_hex(yaml.as_bytes()))
    }

    /// target.json (pinned identity).
    pub fn read_target(&self, target_id: &str) -> AaResult<Option<pb::Target>> {
        let path = self.existing_target_dir(target_id)?.join(TARGET_JSON);
        let Some(bytes) = fsutil::read_optional(&path)? else {
            return Ok(None);
        };
        let target: pb::Target = fsutil::parse_json(&path, &bytes)?;
        if target.id != target_id {
            return Err(AaError::new(
                pb::ErrorCode::ConfigurationInvalid,
                format!(
                    "{} records target id {:?}, expected {target_id:?}",
                    path.display(),
                    target.id
                ),
                format!("inspect or restore {}", path.display()),
            ));
        }
        Ok(Some(target))
    }

    pub fn write_target(&self, target: &pb::Target) -> AaResult<()> {
        let dir = self.ensure_target_dir(&target.id)?;
        fsutil::atomic_write(&dir.join(TARGET_JSON), &fsutil::canonical_json_line(target)?)
    }

    /// Parsed and validated by `aa_core::manifest::lock_from_json` (schema and `lock_digest`).
    pub fn read_lock(&self, target_id: &str) -> AaResult<Option<pb::CapabilityLock>> {
        let path = self.existing_target_dir(target_id)?.join(LOCK_JSON);
        fsutil::read_optional(&path)?
            .map(|b| aa_core::manifest::lock_from_json(&b))
            .transpose()
    }

    /// Canonical bytes via `aa_core::manifest::lock_to_json`. Returns the sha256 of the bytes written (the file
    /// digest; the lock's own `lock_digest` is a field of `lock`).
    pub fn write_lock(&self, lock: &pb::CapabilityLock) -> AaResult<String> {
        let target_id = lock
            .target
            .as_ref()
            .map(|t| t.id.as_str())
            .ok_or_else(|| AaError::invalid("capability lock has no target"))?;
        let dir = self.ensure_target_dir(target_id)?;
        let bytes = aa_core::manifest::lock_to_json(lock)?;
        fsutil::atomic_write(&dir.join(LOCK_JSON), &bytes)?;
        Ok(digest::sha256_hex(&bytes))
    }

    /// `harvest/index.json`: a JSON array of proto3-JSON `SourceRef`s, canonical.
    pub fn write_harvest_index(&self, target_id: &str, sources: &[pb::SourceRef]) -> AaResult<()> {
        let dir = self.ensure_target_dir(target_id)?.join("harvest");
        fsutil::ensure_private_dir(&dir, LOOSE_WRITABLE)?;
        fsutil::atomic_write(&dir.join("index.json"), &fsutil::canonical_json_line(sources)?)
    }

    /// Empty when the target has never been harvested.
    pub fn read_harvest_index(&self, target_id: &str) -> AaResult<Vec<pb::SourceRef>> {
        let path = self.existing_target_dir(target_id)?.join("harvest").join("index.json");
        match fsutil::read_optional(&path)? {
            None => Ok(Vec::new()),
            Some(b) => fsutil::parse_json(&path, &b),
        }
    }

    /// `verification/<capability_id>/<record_id>.json`. The target is the registered target whose id is the
    /// longest `.`-delimited prefix of `rec.capability_id`. An empty `record_id` is filled with
    /// `"vr_" + sha256(JCS(rec with record_id = ""))`.
    pub fn write_verification(&self, rec: &pb::VerificationRecord) -> AaResult<()> {
        validate_file_component(&rec.capability_id, "capability id")?;
        let target_id = self.target_of_capability(&rec.capability_id)?;
        let mut rec = rec.clone();
        if rec.record_id.is_empty() {
            rec.record_id = format!("vr_{}", digest::self_digest(&rec, |r| r.record_id.clear())?);
        }
        validate_file_component(&rec.record_id, "verification record id")?;
        let dir = self
            .ensure_target_dir(&target_id)?
            .join("verification")
            .join(&rec.capability_id);
        fsutil::ensure_private_dir(&dir, LOOSE_WRITABLE)?;
        fsutil::atomic_write(
            &dir.join(format!("{}.json", rec.record_id)),
            &fsutil::canonical_json_line(&rec)?,
        )
    }

    /// Every verification record of the target, ordered by (capability id, record id).
    pub fn read_verifications(&self, target_id: &str) -> AaResult<Vec<pb::VerificationRecord>> {
        let root = self.existing_target_dir(target_id)?.join("verification");
        let mut files = Vec::new();
        for cap_dir in sorted_entries(&root)? {
            if !fs::symlink_metadata(&cap_dir).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            for f in sorted_entries(&cap_dir)? {
                let is_record = f
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".json") && !n.starts_with('.'));
                if is_record && fs::symlink_metadata(&f).is_ok_and(|m| m.is_file()) {
                    files.push(f);
                }
            }
        }
        files
            .iter()
            .map(|f| {
                let bytes = fs::read(f).map_err(|e| io_err(format_args!("cannot read {}", f.display()), &e))?;
                fsutil::parse_json(f, &bytes)
            })
            .collect()
    }

    pub fn cas(&self) -> &Cas {
        &self.cas
    }

    /// `<root>/sessions` (0700): the engine creates one 0700 directory per session inside.
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Parent for private roots.
    pub fn scratch_dir(&self) -> PathBuf {
        self.root.join("scratch")
    }

    /// state/hmac.key, created 0600 on first use.
    ///
    /// Created atomically: 32 bytes from `/dev/urandom` go to a 0600 temp file which is then hard-linked into
    /// place (fails if another process won the race, in which case its key is used). An existing key with any
    /// group/other permission bit, a foreign owner or a length other than 32 bytes is refused.
    pub fn hmac_key(&self) -> AaResult<[u8; 32]> {
        let state = self.root.join("state");
        let path = state.join(HMAC_KEY);
        if fs::symlink_metadata(&path).is_err() {
            let mut key = [0u8; 32];
            fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut key))
                .map_err(|e| io_err("cannot read /dev/urandom", &e))?;
            // NamedTempFile: unique name, mode 0600, removed on drop.
            let linked = tempfile::Builder::new()
                .prefix(".hmac.key.")
                .tempfile_in(&state)
                .and_then(|mut tmp| {
                    tmp.write_all(&key)?;
                    tmp.as_file().sync_all()?;
                    match fs::hard_link(tmp.path(), &path) {
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                        other => other,
                    }
                });
            linked.map_err(|e| io_err(format_args!("cannot create {}", path.display()), &e))?;
            fsutil::fsync_dir(&state);
        }
        read_key(&path)
    }

    fn existing_target_dir(&self, target_id: &str) -> AaResult<PathBuf> {
        validate_target_id(target_id)?;
        Ok(self.target_dir(target_id))
    }

    fn ensure_target_dir(&self, target_id: &str) -> AaResult<PathBuf> {
        let dir = self.existing_target_dir(target_id)?;
        fsutil::ensure_private_dir(&dir, LOOSE_WRITABLE)?;
        Ok(dir)
    }

    fn target_of_capability(&self, capability_id: &str) -> AaResult<String> {
        self.list_targets()?
            .into_iter()
            .filter(|t| {
                capability_id
                    .strip_prefix(t.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
            .max_by_key(String::len)
            .ok_or_else(|| {
                AaError::new(
                    pb::ErrorCode::UnresolvedSelector,
                    format!("no registered target owns capability {capability_id:?}"),
                    "register the target first: aa target add",
                )
            })
    }
}

fn read_key(path: &Path) -> AaResult<[u8; 32]> {
    let shown = path.display();
    let meta = fs::symlink_metadata(path).map_err(|e| io_err(format_args!("cannot stat {shown}"), &e))?;
    if !meta.is_file() {
        return Err(fsutil::config_err(
            format!("{shown} is not a regular file"),
            format!("remove {shown}; a new key is generated"),
        ));
    }
    let mode = meta.permissions().mode() & 0o7777;
    if meta.uid() != fsutil::euid() || mode & ANY_GROUP_OTHER != 0 {
        return Err(fsutil::config_err(
            format!(
                "{shown} has owner uid {} / mode {mode:04o}; it must be the engine user's, 0600",
                meta.uid()
            ),
            format!("chmod 600 {shown}"),
        ));
    }
    let bytes = fs::read(path).map_err(|e| io_err(format_args!("cannot read {shown}"), &e))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        fsutil::config_err(
            format!("{shown} holds {} bytes, expected 32", bytes.len()),
            format!("restore {shown} (args commitments made with the old key will no longer compare equal)"),
        )
    })
}

/// A single safe path component: `[A-Za-z0-9._@+-]`, non-empty, not starting with `.`.
fn validate_file_component(s: &str, what: &str) -> AaResult<()> {
    let ok = !s.is_empty()
        && !s.starts_with('.')
        && s.len() <= 255
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@' | b'+'));
    if ok {
        Ok(())
    } else {
        Err(AaError::invalid(format!(
            "{what} {s:?} cannot be used as a file name (allowed: [A-Za-z0-9._@+-], not starting with '.')"
        )))
    }
}

/// Entries of `dir` sorted by name; empty when `dir` does not exist.
fn sorted_entries(dir: &Path) -> AaResult<Vec<PathBuf>> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(format_args!("cannot list {}", dir.display()), &e)),
    };
    let mut out = rd
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| io_err(format_args!("cannot list {}", dir.display()), &e))?;
    out.sort();
    Ok(out)
}
