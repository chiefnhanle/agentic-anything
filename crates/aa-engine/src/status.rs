//! Kill switch state (architecture §5.6) and in-flight action tracking.
//!
//! The halt state is held in memory behind a mutex, persisted on every change (fail-safe: a persistence
//! failure still halts in memory), and broadcast on a `watch` channel so that in-flight dispatches drop their
//! futures (killing the owned process group) as soon as the brake is pulled.

use crate::ports::HaltPersist;
use aa_core::{AaError, AaResult, pb};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::watch;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct HaltCtl {
    state: Mutex<pb::HaltState>,
    tx: watch::Sender<pb::HaltState>,
    persist: Arc<dyn HaltPersist>,
}

impl HaltCtl {
    pub fn new(initial: pb::HaltState, persist: Arc<dyn HaltPersist>) -> Self {
        let (tx, _rx) = watch::channel(initial.clone());
        Self {
            state: Mutex::new(initial),
            tx,
            persist,
        }
    }

    pub fn current(&self) -> pb::HaltState {
        lock(&self.state).clone()
    }

    pub fn is_halted(&self) -> bool {
        lock(&self.state).halted
    }

    pub fn subscribe(&self) -> watch::Receiver<pb::HaltState> {
        self.tx.subscribe()
    }

    pub fn sentinel_present(&self) -> bool {
        self.persist.sentinel_present()
    }

    /// Engages the brake. Every call bumps the epoch, so an `Unhalt` prepared against an older halt is
    /// refused. Narrower scopes are recorded but enforced engine-wide in slice 1 (the conservative reading).
    pub fn halt(&self, source: pb::HaltSource, scope: pb::HaltScope, subject: &str, reason: &str) -> pb::HaltState {
        let next = {
            let mut s = lock(&self.state);
            s.halted = true;
            s.halt_epoch += 1;
            s.scope = scope as i32;
            s.scope_subject = subject.to_string();
            s.source = source as i32;
            s.reason = reason.to_string();
            s.clone()
        };
        if let Err(e) = self.persist.save(&next) {
            tracing::error!(error = %e, "could not persist halt state; halted in memory only");
        }
        self.tx.send_replace(next.clone());
        next
    }

    /// Releases the brake for exactly the current epoch.
    pub fn unhalt(&self, epoch: u64) -> AaResult<pb::HaltState> {
        if self.persist.sentinel_present() {
            return Err(AaError::new(
                pb::ErrorCode::EngineHalted,
                "the HALT sentinel file is present; remove it before unhalting",
                "rm <AA_HOME>/state/HALT && aa unhalt --epoch <N>",
            ));
        }
        let next = {
            let mut s = lock(&self.state);
            if !s.halted {
                return Err(AaError::invalid("the engine is not halted")
                    .with_recovery(pb::RecoveryKind::InspectInputContract, "aa status"));
            }
            if s.halt_epoch != epoch {
                return Err(AaError::new(
                    pb::ErrorCode::InvalidRequest,
                    format!("stale halt epoch {epoch}; the current epoch is {}", s.halt_epoch),
                    "aa status, then aa unhalt --epoch <current halt_epoch>",
                ));
            }
            s.halted = false;
            s.source = pb::HaltSource::Unspecified as i32;
            s.scope = pb::HaltScope::Engine as i32;
            s.scope_subject.clear();
            s.reason.clear();
            s.clone()
        };
        self.persist.save(&next)?;
        self.tx.send_replace(next.clone());
        Ok(next)
    }
}

/// Why an in-flight action was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopCause {
    CancelRpc,
}

/// Registry of in-flight dispatches (for `Cancel` and the `Halt` response count).
#[derive(Default)]
pub(crate) struct Inflight {
    map: Mutex<HashMap<String, watch::Sender<Option<StopCause>>>>,
}

pub(crate) struct InflightGuard<'a> {
    owner: &'a Inflight,
    id: String,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        lock(&self.owner.map).remove(&self.id);
    }
}

impl Inflight {
    pub fn register(&self, action_id: &str) -> (InflightGuard<'_>, watch::Receiver<Option<StopCause>>) {
        let (tx, rx) = watch::channel(None);
        lock(&self.map).insert(action_id.to_string(), tx);
        (
            InflightGuard {
                owner: self,
                id: action_id.to_string(),
            },
            rx,
        )
    }

    pub fn contains(&self, action_id: &str) -> bool {
        lock(&self.map).contains_key(action_id)
    }

    pub fn len(&self) -> usize {
        lock(&self.map).len()
    }

    /// `true` if the action was in flight and has been told to stop.
    pub fn cancel(&self, action_id: &str, cause: StopCause) -> bool {
        lock(&self.map).get(action_id).is_some_and(|tx| {
            tx.send_replace(Some(cause));
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MemHalt;

    #[test]
    fn epochs_and_stale_unhalt() {
        let persist = Arc::new(MemHalt::default());
        let h = HaltCtl::new(pb::HaltState::default(), persist.clone());
        let mut rx = h.subscribe();
        let s1 = h.halt(pb::HaltSource::Rpc, pb::HaltScope::Engine, "", "first");
        assert!(s1.halted && s1.halt_epoch == 1);
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        let s2 = h.halt(pb::HaltSource::Rpc, pb::HaltScope::Engine, "", "second");
        assert_eq!(s2.halt_epoch, 2);
        assert_eq!(h.unhalt(1).unwrap_err().code, pb::ErrorCode::InvalidRequest);
        assert!(h.is_halted());
        let s3 = h.unhalt(2).unwrap();
        assert!(!s3.halted && s3.halt_epoch == 2);
        assert!(!persist.load().halted);
        assert!(h.unhalt(2).is_err(), "not halted any more");
        persist.set_sentinel(true);
        h.halt(pb::HaltSource::SentinelFile, pb::HaltScope::Engine, "", "file");
        assert_eq!(h.unhalt(3).unwrap_err().code, pb::ErrorCode::EngineHalted);
    }

    #[test]
    fn inflight_cancel() {
        let i = Inflight::default();
        let (g, rx) = i.register("act_1");
        assert_eq!(i.len(), 1);
        assert!(i.cancel("act_1", StopCause::CancelRpc));
        assert_eq!(*rx.borrow(), Some(StopCause::CancelRpc));
        assert!(!i.cancel("act_2", StopCause::CancelRpc));
        drop(g);
        assert_eq!(i.len(), 0);
    }
}
