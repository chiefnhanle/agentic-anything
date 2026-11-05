//! Process execution contract (frozen; implemented by `aa-exec::Supervisor`).

use crate::pb;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

/// OS-enforced confinement request (Seatbelt on macOS).
#[derive(Debug, Clone, Default)]
pub struct ConfinementSpec {
    pub deny_network: bool,
    pub writable: Vec<PathBuf>,
    pub exec_allow: Vec<PathBuf>,
}

/// One process run. Never a shell.
#[derive(Debug, Clone)]
pub struct RunSpec {
    /// argv[0] must be an absolute realpath; never a shell.
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    /// The COMPLETE child env (the runner calls env_clear first).
    pub env: BTreeMap<String, String>,
    /// `None` => /dev/null.
    pub stdin: Option<Vec<u8>>,
    pub deadline: Duration,
    pub max_stdout: u64,
    pub max_stderr_excerpt: usize,
    pub confinement: Option<ConfinementSpec>,
    /// Exported last as `AA_RUN_ID`.
    pub run_id: String,
}

/// What a run produced and what was actually in force.
#[derive(Debug, Clone)]
pub struct RunOutput {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: Vec<u8>,
    pub stdout_total: u64,
    pub stdout_truncated: bool,
    pub stderr_excerpt: String,
    pub stderr_total: u64,
    pub wall: Duration,
    pub timed_out: bool,
    pub cleanup: pb::CleanupStatus,
    pub confinement_denials: Vec<String>,
    /// What was actually in force.
    pub isolation: pb::Isolation,
}

/// Dropping the returned future MUST kill the owned process group (drop guard).
#[async_trait::async_trait]
pub trait ProcessRunner: Send + Sync {
    async fn run(&self, spec: RunSpec) -> crate::AaResult<RunOutput>;
    /// Best isolation this runner can provide on this host (CONFINED if a Confiner is available).
    fn max_isolation(&self) -> pb::Isolation;
}
