//! `aa-core`: the pure, transport-free core of agentic-anything (ADR 0013).
//!
//! No tokio, tonic, hyper, rusqlite or process-wrap may enter this crate's normal dependency closure;
//! `tests/dependency_direction.rs` enforces that.

pub use aa_proto::v1 as pb;

// WP0 foundation.
pub mod api;
pub mod digest;
pub mod error;
pub mod ids;

// WP1 semantics.
pub mod admission;
pub mod characterize;
pub mod effects;
pub mod lanes;
pub mod manifest;
pub mod policy;

pub use error::{AaError, AaResult};
