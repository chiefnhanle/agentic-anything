//! `aa-store`: on-disk workspace, content-addressed store, JSONL audit chain and halt state (architecture §12).
//!
//! - [`Workspace`]: the `$AA_HOME` layout (targets, locks, harvest indexes, verification records, sessions,
//!   scratch, the HMAC key), created 0700 and refused when symlinked, foreign-owned or loosely permissioned.
//! - [`Cas`]: immutable sha256-addressed blobs.
//! - [`AuditLog`]: the hash-chained JSONL audit log with HEAD mirroring and boot-time intent recovery; it
//!   implements [`aa_core::api::AuditSink`].
//! - [`HaltStore`]: persisted kill-switch state and the `HALT` sentinel.

pub mod audit;
pub mod cas;
mod fsutil;
pub mod halt;
pub mod workspace;

pub use audit::{AuditLog, AuditQuery, BootReport};
pub use cas::Cas;
pub use halt::HaltStore;
pub use workspace::Workspace;
