//! Audit sink contract (frozen; implemented by `aa-store::AuditLog`).

use crate::pb;

pub trait AuditSink: Send + Sync {
    /// Append + fsync. Err => caller MUST NOT dispatch (AUDIT_UNAVAILABLE).
    fn append_intent(
        &self,
        principal: &pb::Principal,
        halt_epoch: u64,
        intent: pb::ActionIntent,
    ) -> crate::AaResult<u64>;
    fn append_action(
        &self,
        principal: &pb::Principal,
        halt_epoch: u64,
        record: pb::ActionRecord,
    ) -> crate::AaResult<u64>;
    fn append_admin(&self, principal: &pb::Principal, halt_epoch: u64, event: pb::AdminEvent) -> crate::AaResult<u64>;
    /// `(seq, entry_sha256)` of the last entry; `(0, "")` when empty.
    fn head(&self) -> (u64, String);
    /// Used by compile to verify `assumed[].audit_seq`.
    fn entry(&self, seq: u64) -> crate::AaResult<Option<pb::AuditEntry>>;
}
