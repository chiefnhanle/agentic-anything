//! Sessions: a private 0700 workspace (engine-supplied state handles such as CLI-Anything `--project`), a
//! PathScope extension, an authority ceiling and the taint bit (architecture §5.6).

use crate::status::lock;
use aa_core::{AaError, AaResult, pb};
use std::{
    collections::HashMap,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct Session {
    pub id: String,
    pub workspace: PathBuf,
    pub target_ids: Vec<String>,
    pub ceiling: pb::AuthorityTier,
    pub read_roots: Vec<PathBuf>,
    pub owner_uid: u32,
    pub ephemeral: bool,
    tainted: AtomicBool,
}

impl Session {
    pub fn tainted(&self) -> bool {
        self.tainted.load(Ordering::SeqCst)
    }

    pub fn taint(&self) {
        self.tainted.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub(crate) struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
}

pub(crate) struct NewSession {
    pub target_ids: Vec<String>,
    pub ceiling: pb::AuthorityTier,
    pub read_roots: Vec<PathBuf>,
    pub owner_uid: u32,
    pub ephemeral: bool,
}

impl Sessions {
    pub fn open(&self, sessions_dir: &Path, spec: NewSession) -> AaResult<Arc<Session>> {
        let id = aa_core::ids::new_session_id();
        std::fs::create_dir_all(sessions_dir)?;
        let workspace = sessions_dir.join(&id);
        std::fs::DirBuilder::new().mode(0o700).create(&workspace)?;
        // The umask may have narrowed the mode further; make it exactly 0700.
        std::fs::set_permissions(&workspace, std::fs::Permissions::from_mode(0o700))?;
        let workspace = std::fs::canonicalize(&workspace)?;
        let s = Arc::new(Session {
            id: id.clone(),
            workspace,
            target_ids: spec.target_ids,
            ceiling: spec.ceiling,
            read_roots: spec.read_roots,
            owner_uid: spec.owner_uid,
            ephemeral: spec.ephemeral,
            tainted: AtomicBool::new(false),
        });
        if !s.ephemeral {
            lock(&self.map).insert(id, Arc::clone(&s));
        }
        Ok(s)
    }

    /// Only the uid that opened a session can use it; anything else looks like an unknown id.
    pub fn get(&self, id: &str, uid: u32) -> AaResult<Arc<Session>> {
        lock(&self.map)
            .get(id)
            .filter(|s| s.owner_uid == uid)
            .cloned()
            .ok_or_else(|| {
                crate::ports::not_found(
                    format!("unknown session {id:?}"),
                    "aa invoke without --session, or open a new session",
                )
            })
    }

    /// Removes the session; deletes its workspace unless `keep`. Returns the retained path, if any.
    pub fn close(&self, id: &str, uid: u32, keep: bool) -> AaResult<Option<PathBuf>> {
        let s = self.get(id, uid)?;
        lock(&self.map).remove(id);
        if keep {
            Ok(Some(s.workspace.clone()))
        } else {
            remove_workspace(&s);
            Ok(None)
        }
    }
}

pub(crate) fn remove_workspace(s: &Session) {
    if let Err(e) = std::fs::remove_dir_all(&s.workspace)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(session = %s.id, error = %e, "could not remove session workspace");
    }
}

/// Removes an ephemeral session's workspace when the invocation ends.
pub(crate) struct EphemeralGuard(pub Arc<Session>);

impl Drop for EphemeralGuard {
    fn drop(&mut self) {
        if self.0.ephemeral {
            remove_workspace(&self.0);
        }
    }
}

pub(crate) fn invalid_root(p: &Path) -> AaError {
    AaError::new(
        pb::ErrorCode::PolicyDenied,
        format!("read root {} is outside the policy's read/write roots", p.display()),
        "add the directory to [paths] read_roots in policy.toml",
    )
    .with_denial(pb::DenialReason::OutsidePathScope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn open_get_close() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sessions::default();
        let spec = |ephemeral| NewSession {
            target_ids: vec![],
            ceiling: pb::AuthorityTier::Unknown,
            read_roots: vec![],
            owner_uid: 7,
            ephemeral,
        };
        let a = s.open(dir.path(), spec(false)).unwrap();
        assert!(a.id.starts_with("ses_"));
        assert_eq!(std::fs::metadata(&a.workspace).unwrap().mode() & 0o777, 0o700);
        assert!(s.get(&a.id, 7).is_ok());
        assert!(s.get(&a.id, 8).is_err(), "other uids cannot see the session");
        assert!(!a.tainted());
        a.taint();
        assert!(s.get(&a.id, 7).unwrap().tainted());
        assert_eq!(s.close(&a.id, 7, true).unwrap(), Some(a.workspace.clone()));
        assert!(a.workspace.exists());
        assert!(s.get(&a.id, 7).is_err());

        let b = s.open(dir.path(), spec(false)).unwrap();
        assert_eq!(s.close(&b.id, 7, false).unwrap(), None);
        assert!(!b.workspace.exists());

        let e = s.open(dir.path(), spec(true)).unwrap();
        assert!(s.get(&e.id, 7).is_err(), "ephemeral sessions are not registered");
        let ws = e.workspace.clone();
        drop(EphemeralGuard(e));
        assert!(!ws.exists());
    }
}
