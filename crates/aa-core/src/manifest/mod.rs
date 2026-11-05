//! The capability manifest: overlay (human YAML), deterministic compile, canonical lock (ADR 0003).

pub mod compile;
pub mod lock;
pub mod overlay;

pub use compile::{CompileInput, CompileOutput, capability_digest, compile};
pub use lock::{finalize_digest, lock_from_json, lock_to_json};
pub use overlay::*;
