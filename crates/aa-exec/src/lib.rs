//! `aa-exec`: process supervision kernel, OS confinement, the `cli_argv` binding and the L1/L3 lanes.
//!
//! Public signatures are frozen by build-plan §3.2. Additive extras: `SupervisorConfig: Default`,
//! `identity::file_identity_as_invoked`, `IdentityCache: Default`, and the lane constants in `lanes`.
//!
//! No `unsafe` in this crate; process groups, kill-on-drop and the signal-mask reset come from `process-wrap`.

pub mod cli_argv;
pub mod confine;
pub mod identity;
pub mod lanes;
pub mod root;
pub mod snapshot;
pub mod supervisor;

pub use cli_argv::{CliArgvBinding, build_argv, normalize_output};
pub use confine::{Confiner, NoConfiner, SeatbeltConfiner, default_confiner, seatbelt_profile};
pub use identity::{IdentityCache, file_identity, identity_digest};
pub use root::PrivateRoot;
pub use snapshot::{TreeSnapshot, diff, snapshot};
pub use supervisor::{Supervisor, SupervisorConfig};
