//! Frozen cross-crate traits and structs (build-plan §3.0.3). Implementations live in other crates:
//! `ProcessRunner`/`Binding` in aa-exec, `Harvester` in aa-harvest, `AuditSink` in aa-store.

pub mod audit;
pub mod binding;
pub mod exec;
pub mod harvest;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use audit::*;
pub use binding::*;
pub use exec::*;
pub use harvest::*;
