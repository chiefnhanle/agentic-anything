//! One module per command family. Each command returns a [`CmdResult`]; [`crate::output::Output::emit`] renders
//! it and computes the exit code.

pub mod audit;
pub mod catalog;
pub mod daemon;
pub mod distill;
pub mod engine;
pub mod invoke;
pub mod target;

use crate::Command;
use crate::connect::{ClientConfig, connect};
use crate::exit::CliError;
use crate::output::{CmdResult, Output};
use aa_proto::v1 as pb;
use std::path::{Path, PathBuf};

pub async fn dispatch(command: Command, cfg: &ClientConfig, out: &Output) -> CmdResult {
    match command {
        Command::Daemon(a) => daemon::run(a, cfg).await,
        Command::Status => engine::status(cfg).await,
        Command::Halt(a) => engine::halt(a, cfg).await,
        Command::Unhalt(a) => engine::unhalt(a, cfg).await,
        Command::Target { command } => target::run_target(command, cfg).await,
        Command::Import { command } => target::run_import(command, cfg).await,
        Command::Discover(a) => distill::discover(a, cfg, out).await,
        Command::Verify(a) => distill::verify(a, cfg, out).await,
        Command::List(a) => catalog::list(a, cfg).await,
        Command::Describe(a) => catalog::describe(a, cfg).await,
        Command::Invoke(a) => invoke::run(a, cfg, out).await,
        Command::Audit(a) => audit::run(a, cfg).await,
    }
}

/// The agent-socket channel (the default for every non-operator command).
pub(crate) async fn agent(cfg: &ClientConfig) -> Result<tonic::transport::Channel, CliError> {
    connect(cfg, false).await
}

/// Makes a command-line path absolute against the client's cwd (the engine has a different cwd). Does not resolve
/// symlinks: identity pinning is the engine's job.
pub(crate) fn absolute(p: &Path) -> Result<PathBuf, CliError> {
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    let cwd =
        std::env::current_dir().map_err(|e| CliError::Usage(format!("cannot read the current directory: {e}")))?;
    Ok(cwd.join(p))
}

/// Client-side copy of the target id grammar `[a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*` (the engine re-validates).
pub(crate) fn valid_target_id(id: &str) -> bool {
    let mut segments = id.split('.');
    let Some(first) = segments.next() else { return false };
    let seg_ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if !seg_ok(first) || !first.as_bytes()[0].is_ascii_alphanumeric() {
        return false;
    }
    segments.all(seg_ok)
}

/// An engine-shaped error authored by the client (e.g. a stream that ended without a terminal event).
pub(crate) fn client_error(
    code: pb::ErrorCode,
    message: impl Into<String>,
    remediation: impl Into<String>,
) -> pb::Error {
    let category = match code {
        pb::ErrorCode::InvalidRequest | pb::ErrorCode::UnresolvedSelector => pb::ErrorCategory::InvalidInput,
        pb::ErrorCode::ConfirmationRequired => pb::ErrorCategory::Authorization,
        pb::ErrorCode::Cancelled => pb::ErrorCategory::Cancelled,
        _ => pb::ErrorCategory::Execution,
    };
    pb::Error {
        code: code as i32,
        category: category as i32,
        message: message.into(),
        remediation: remediation.into(),
        ..Default::default()
    }
}

/// `"WITHHELD_REASON_MIXED_AUTHORITY"` -> `"MIXED_AUTHORITY"`.
pub(crate) fn short_name<'a>(full: &'a str, prefix: &str) -> &'a str {
    full.strip_prefix(prefix).unwrap_or(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_id_grammar() {
        for ok in ["git", "cli-anything.libreoffice", "a1", "fixture-notes", "x.y-z.0"] {
            assert!(valid_target_id(ok), "{ok}");
        }
        for bad in [
            "", "-git", "Git", "git..x", "git.", ".git", "../etc", "a/b", "a_b", "git x",
        ] {
            assert!(!valid_target_id(bad), "{bad}");
        }
    }

    #[test]
    fn short_names() {
        assert_eq!(
            short_name("WITHHELD_REASON_MIXED_AUTHORITY", "WITHHELD_REASON_"),
            "MIXED_AUTHORITY"
        );
        assert_eq!(short_name("X", "WITHHELD_REASON_"), "X");
    }
}
