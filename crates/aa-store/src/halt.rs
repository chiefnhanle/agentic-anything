//! Kill-switch persistence: `state/halt.json` + `state/HALT` sentinel.
//!
//! `HaltStore::open` accepts either the state directory itself (a path whose last component is `state`) or
//! the workspace root (then `<root>/state` is used); both resolve to `<workspace>/state`. `halt.json` holds the last saved
//! [`pb::HaltState`] as canonical proto3-JSON. The `HALT` sentinel is an operator-side halt source
//! (`touch <state>/HALT`); the engine polls [`HaltStore::sentinel_present`].
//!
//! Fail-safe: an unreadable or corrupt `halt.json` loads as *halted* (source `HALT_SOURCE_PERSISTED`), so a
//! damaged state file can never silently unhalt the engine.

use crate::fsutil;
use aa_core::{AaResult, pb};
use std::{fs, io, path::PathBuf};

const HALT_JSON: &str = "halt.json";
const SENTINEL: &str = "HALT";

pub struct HaltStore {
    dir: PathBuf,
}

impl HaltStore {
    /// Opens (creating 0700 if absent) the state directory (see the module docs for how `dir` resolves).
    /// Refuses a symlink, a foreign owner or any group/other permission bit.
    pub fn open(dir: &std::path::Path) -> AaResult<Self> {
        let dir = fsutil::resolve_subdir(dir, "state");
        fsutil::ensure_private_dir(&dir, fsutil::ANY_GROUP_OTHER)?;
        Ok(Self { dir })
    }

    /// The persisted state. Missing file => not halted, epoch 0. Unreadable/corrupt => halted (fail-safe).
    pub fn load(&self) -> pb::HaltState {
        let path = self.dir.join(HALT_JSON);
        let corrupt = |why: String| {
            tracing::error!(path = %path.display(), %why, "halt state unreadable; loading as halted");
            pb::HaltState {
                halted: true,
                halt_epoch: 0,
                scope: pb::HaltScope::Engine as i32,
                scope_subject: String::new(),
                source: pb::HaltSource::Persisted as i32,
                reason: format!("{} is unreadable ({why}); halted fail-safe", path.display()),
            }
        };
        match fsutil::read_optional(&path) {
            Ok(None) => pb::HaltState::default(),
            Ok(Some(bytes)) => match fsutil::parse_json::<pb::HaltState>(&path, &bytes) {
                Ok(s) => s,
                Err(e) => corrupt(e.message),
            },
            Err(e) => corrupt(e.message),
        }
    }

    /// Atomically persists `s` (fsynced). Saving a *not halted* state also removes the `HALT` sentinel, so an
    /// operator unhalt clears the sentinel halt source instead of being re-halted by it immediately.
    pub fn save(&self, s: &pb::HaltState) -> AaResult<()> {
        let path = self.dir.join(HALT_JSON);
        fsutil::atomic_write(&path, &fsutil::canonical_json_line(s)?)?;
        if !s.halted {
            let sentinel = self.dir.join(SENTINEL);
            match fs::remove_file(&sentinel) {
                Ok(()) => fsutil::fsync_dir(&self.dir),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(fsutil::io_err(format_args!("cannot remove {}", sentinel.display()), &e)),
            }
        }
        Ok(())
    }

    /// `true` if `<state>/HALT` exists (any file type, including a dangling symlink).
    pub fn sentinel_present(&self) -> bool {
        fs::symlink_metadata(self.dir.join(SENTINEL)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn default_save_load_and_sentinel() {
        let d = tempfile::tempdir().unwrap();
        let state = d.path().join("state");
        let h = HaltStore::open(&state).unwrap();
        assert_eq!(fs::metadata(&state).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(h.load(), pb::HaltState::default());
        assert!(!h.sentinel_present());

        let halted = pb::HaltState {
            halted: true,
            halt_epoch: 7,
            scope: pb::HaltScope::Engine as i32,
            scope_subject: String::new(),
            source: pb::HaltSource::Rpc as i32,
            reason: "demo".into(),
        };
        h.save(&halted).unwrap();
        assert_eq!(HaltStore::open(&state).unwrap().load(), halted);

        fs::write(state.join("HALT"), b"").unwrap();
        assert!(h.sentinel_present());
        // Saving a halted state keeps the sentinel.
        h.save(&halted).unwrap();
        assert!(h.sentinel_present());
        let unhalted = pb::HaltState {
            halted: false,
            halt_epoch: 7,
            ..Default::default()
        };
        h.save(&unhalted).unwrap();
        assert!(!h.sentinel_present());
        assert_eq!(h.load(), unhalted);
    }

    #[test]
    fn corrupt_state_loads_halted() {
        let d = tempfile::tempdir().unwrap();
        let h = HaltStore::open(d.path()).unwrap();
        fs::write(d.path().join("state").join(HALT_JSON), b"{not json").unwrap();
        let s = h.load();
        assert!(s.halted);
        assert_eq!(s.source, pb::HaltSource::Persisted as i32);
    }

    #[test]
    fn refuses_loose_state_dir() {
        let d = tempfile::tempdir().unwrap();
        let state = d.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            HaltStore::open(d.path()).err().unwrap().code,
            pb::ErrorCode::ConfigurationInvalid
        );
    }
}
