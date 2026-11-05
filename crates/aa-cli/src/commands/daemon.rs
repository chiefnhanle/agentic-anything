//! `aa daemon [--foreground]`: starts the sibling `aad` with this client's runtime dir.
//!
//! `--foreground` replaces this process with `aad` (exec, never a shell). Otherwise `aad` is started detached
//! (own process group, stdin/stdout/stderr redirected) and `aa` waits until `agent.sock` answers `GetStatus`.

use super::{absolute, engine::get_status};
use crate::connect::ClientConfig;
use crate::exit::CliError;
use crate::output::{CmdResult, Reply, to_json};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, clap::Args)]
pub struct DaemonArgs {
    /// Run in the foreground (exec `aad`).
    #[arg(long)]
    pub foreground: bool,
    /// Workspace root passed to `aad --workspace` (default: $AA_HOME).
    #[arg(long)]
    pub workspace: Option<PathBuf>,
    /// Policy file passed to `aad --policy`.
    #[arg(long)]
    pub policy: Option<PathBuf>,
    /// Log filter passed to `aad --log`, e.g. `info`.
    #[arg(long)]
    pub log: Option<String>,
    /// Background mode: append aad's stdout/stderr to this file (default: discarded).
    #[arg(long)]
    pub log_file: Option<PathBuf>,
    /// Background mode: seconds to wait for the engine to answer.
    #[arg(long, default_value_t = 10)]
    pub wait_secs: u64,
}

/// `aad` next to the running `aa`, else the first `aad` on PATH.
pub fn find_aad() -> Option<PathBuf> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let p = dir.join("aad");
        if is_executable(&p) {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("aad"))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// The `aad` argv after the program name.
pub fn aad_args(a: &DaemonArgs, cfg: &ClientConfig) -> Result<Vec<std::ffi::OsString>, CliError> {
    let mut args: Vec<std::ffi::OsString> = vec!["--runtime-dir".into(), cfg.runtime_dir.clone().into()];
    if let Some(w) = &a.workspace {
        args.push("--workspace".into());
        args.push(absolute(w)?.into());
    }
    if let Some(p) = &a.policy {
        args.push("--policy".into());
        args.push(absolute(p)?.into());
    }
    if let Some(l) = &a.log {
        args.push("--log".into());
        args.push(l.into());
    }
    Ok(args)
}

pub async fn run(a: DaemonArgs, cfg: &ClientConfig) -> CmdResult {
    let aad = find_aad().ok_or_else(|| CliError::Unreachable("cannot find `aad` next to `aa` or on PATH".into()))?;
    let args = aad_args(&a, cfg)?;
    let mut cmd = std::process::Command::new(&aad);
    cmd.args(&args);

    if a.foreground {
        use std::os::unix::process::CommandExt as _;
        let err = cmd.exec();
        return Err(CliError::Unreachable(format!("cannot exec {}: {err}", aad.display())).into());
    }

    if let Ok(status) = get_status(cfg).await {
        return Ok(Reply::new(
            json!({ "alreadyRunning": true, "status": to_json(&status) }),
        ));
    }

    use std::os::unix::process::CommandExt as _;
    cmd.stdin(std::process::Stdio::null()).process_group(0);
    match &a.log_file {
        Some(p) => {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| CliError::Usage(format!("cannot open --log-file {}: {e}", p.display())))?;
            let f2 = f
                .try_clone()
                .map_err(|e| CliError::Usage(format!("cannot open --log-file: {e}")))?;
            cmd.stdout(f).stderr(f2);
        }
        None => {
            cmd.stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| CliError::Unreachable(format!("cannot start {}: {e}", aad.display())))?;

    let deadline = Instant::now() + Duration::from_secs(a.wait_secs);
    loop {
        if let Ok(status) = get_status(cfg).await {
            return Ok(Reply::new(json!({
                "alreadyRunning": false,
                "pid": child.id(),
                "aad": aad.to_string_lossy(),
                "status": to_json(&status),
            })));
        }
        if let Ok(Some(exit)) = child.try_wait() {
            return Err(CliError::Unreachable(format!(
                "aad exited during startup ({exit}); run `aa daemon --foreground` to see why"
            ))
            .into());
        }
        if Instant::now() >= deadline {
            return Err(CliError::Unreachable(format!(
                "aad (pid {}) did not answer on {} within {}s",
                child.id(),
                cfg.agent_socket().display(),
                a.wait_secs
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_carry_runtime_dir() {
        let cfg = ClientConfig {
            runtime_dir: PathBuf::from("/rt"),
        };
        let a = DaemonArgs {
            foreground: true,
            workspace: Some(PathBuf::from("/ws")),
            policy: None,
            log: Some("info".into()),
            log_file: None,
            wait_secs: 1,
        };
        let v: Vec<String> = aad_args(&a, &cfg)
            .unwrap()
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(v, ["--runtime-dir", "/rt", "--workspace", "/ws", "--log", "info"]);
    }
}
