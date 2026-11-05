//! Engine configuration from environment (`AA_HOME`, `AA_RUNTIME_DIR`, `AA_AGENT_UIDS`) and `aad` arguments.

use aa_core::{AaError, AaResult, pb};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// AA_HOME.
    pub workspace: std::path::PathBuf,
    /// Contains agent.sock, operator.sock, engine.lock.
    pub runtime_dir: std::path::PathBuf,
    /// Missing => built-in default (deny all but observe of registered targets).
    pub policy_path: std::path::PathBuf,
    /// Default `[geteuid()]`.
    pub agent_uids: Vec<u32>,
    pub engine_version: String,
}

impl EngineConfig {
    /// AA_HOME, AA_RUNTIME_DIR, --policy.
    ///
    /// Precedence: explicit argument, then environment, then the platform default (architecture §3.2).
    /// `AA_AGENT_UIDS` (comma-separated) overrides the default agent uid list `[geteuid()]`.
    pub fn from_env_and_args(args: &AadArgs) -> AaResult<Self> {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = env("HOME");
        let workspace = match args.workspace.clone().or_else(|| env("AA_HOME")) {
            Some(p) => p,
            None => home
                .as_ref()
                .map(|h| h.join(".local/share/agentic-anything"))
                .ok_or_else(|| config_error("cannot determine the workspace: set AA_HOME or pass --workspace"))?,
        };
        let runtime_dir = match args.runtime_dir.clone().or_else(|| env("AA_RUNTIME_DIR")) {
            Some(p) => p,
            None => default_runtime_dir(),
        };
        let policy_path = match args.policy.clone() {
            Some(p) => p,
            None => home
                .as_ref()
                .map(|h| h.join(".config/agentic-anything/policy.toml"))
                .unwrap_or_else(|| PathBuf::from("/nonexistent/agentic-anything/policy.toml")),
        };
        let agent_uids = match std::env::var("AA_AGENT_UIDS") {
            Ok(v) if !v.trim().is_empty() => parse_uids(&v)?,
            _ => vec![euid()],
        };
        Ok(Self {
            workspace: absolute(&workspace)?,
            runtime_dir: absolute(&runtime_dir)?,
            policy_path: absolute(&policy_path)?,
            agent_uids,
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }
}

#[derive(Debug, clap::Parser)]
#[command(name = "aad", about = "agentic-anything engine daemon")]
pub struct AadArgs {
    /// Workspace root (default: $AA_HOME or ~/.local/share/agentic-anything).
    #[arg(long)]
    pub workspace: Option<std::path::PathBuf>,
    /// Runtime dir holding the sockets (default: $AA_RUNTIME_DIR or the platform default).
    #[arg(long)]
    pub runtime_dir: Option<std::path::PathBuf>,
    /// Policy file (default: ~/.config/agentic-anything/policy.toml).
    #[arg(long)]
    pub policy: Option<std::path::PathBuf>,
    /// tracing-subscriber env-filter directive, e.g. "info".
    #[arg(long)]
    pub log: Option<String>,
}

pub(crate) fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// macOS: `$TMPDIR/aa`; Linux: `$XDG_RUNTIME_DIR/aa`; fallback `/tmp/aa-<uid>` (architecture §3.2).
fn default_runtime_dir() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let preferred = if cfg!(target_os = "macos") {
        var("TMPDIR")
    } else {
        var("XDG_RUNTIME_DIR")
    };
    match preferred {
        Some(p) => p.join("aa"),
        None => PathBuf::from(format!("/tmp/aa-{}", euid())),
    }
}

fn parse_uids(v: &str) -> AaResult<Vec<u32>> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u32>()
                .map_err(|_| config_error(format!("AA_AGENT_UIDS: {s:?} is not a uid")))
        })
        .collect()
}

fn absolute(p: &Path) -> AaResult<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(p))
    }
}

pub(crate) fn config_error(message: impl Into<String>) -> AaError {
    AaError::new(
        pb::ErrorCode::ConfigurationInvalid,
        message,
        "fix the aad configuration (aad --help) and restart: aa daemon",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_args_win_and_are_absolute() {
        let args = AadArgs {
            workspace: Some("/tmp/aa-ws".into()),
            runtime_dir: Some("rt".into()),
            policy: Some("/etc/aa/policy.toml".into()),
            log: None,
        };
        let cfg = EngineConfig::from_env_and_args(&args).unwrap();
        assert_eq!(cfg.workspace, PathBuf::from("/tmp/aa-ws"));
        assert!(cfg.runtime_dir.is_absolute() && cfg.runtime_dir.ends_with("rt"));
        assert_eq!(cfg.policy_path, PathBuf::from("/etc/aa/policy.toml"));
        assert!(!cfg.agent_uids.is_empty());
        assert_eq!(cfg.engine_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn uid_list_parsing() {
        assert_eq!(parse_uids("501, 502,").unwrap(), vec![501, 502]);
        assert_eq!(parse_uids("x").unwrap_err().code, pb::ErrorCode::ConfigurationInvalid);
    }
}
