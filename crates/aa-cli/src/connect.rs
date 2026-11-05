//! Client connection to the engine sockets (ADR 0005).
//!
//! The runtime dir is `$AA_RUNTIME_DIR` when set, otherwise the platform default:
//! - macOS: `$TMPDIR/aa/`, falling back to `/tmp/aa-<uid>/` when `operator.sock` inside it would exceed the
//!   ~100-byte `sun_path` budget;
//! - Linux: `$XDG_RUNTIME_DIR/aa/`, falling back to `/tmp/aa-<uid>/`.

use crate::exit::CliError;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Environment variable naming the runtime dir that holds `agent.sock` and `operator.sock`.
pub const RUNTIME_DIR_ENV: &str = "AA_RUNTIME_DIR";
/// Socket file names inside the runtime dir.
pub const AGENT_SOCKET: &str = "agent.sock";
pub const OPERATOR_SOCKET: &str = "operator.sock";
/// Longest socket path we will use from the default location (macOS `sun_path` is 104 bytes, NUL included).
pub const MAX_SOCKET_PATH: usize = 100;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// AA_RUNTIME_DIR or platform default.
pub struct ClientConfig {
    pub runtime_dir: std::path::PathBuf,
}

impl ClientConfig {
    pub fn from_env() -> Self {
        let runtime_dir = match std::env::var_os(RUNTIME_DIR_ENV) {
            Some(v) if !v.is_empty() => absolutize(Path::new(&v)),
            _ => default_runtime_dir(),
        };
        ClientConfig { runtime_dir }
    }

    pub fn agent_socket(&self) -> std::path::PathBuf {
        self.runtime_dir.join(AGENT_SOCKET)
    }

    pub fn operator_socket(&self) -> std::path::PathBuf {
        self.runtime_dir.join(OPERATOR_SOCKET)
    }
}

/// The platform default runtime dir (used when `AA_RUNTIME_DIR` is unset or empty).
pub fn default_runtime_dir() -> PathBuf {
    let preferred = if cfg!(target_os = "macos") {
        std::env::var_os("TMPDIR")
            .filter(|v| !v.is_empty())
            .map(|t| PathBuf::from(t).join("aa"))
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(|t| PathBuf::from(t).join("aa"))
    };
    match preferred {
        Some(dir) if dir.join(OPERATOR_SOCKET).as_os_str().len() <= MAX_SOCKET_PATH => dir,
        _ => fallback_runtime_dir(current_uid()),
    }
}

/// `/tmp/aa-<uid>`.
pub fn fallback_runtime_dir(uid: Option<u32>) -> PathBuf {
    match uid {
        Some(uid) => PathBuf::from(format!("/tmp/aa-{uid}")),
        None => PathBuf::from("/tmp/aa"),
    }
}

/// The effective uid without `unsafe` or extra dependencies: `/proc/self` on Linux; otherwise the owner of a
/// freshly created probe file in the temp dir (new files are owned by the euid).
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    if let Ok(m) = std::fs::metadata("/proc/self") {
        return Some(m.uid());
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let probe = std::env::temp_dir().join(format!(".aa-uid-probe-{}-{nanos}", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .ok()?;
    let uid = file.metadata().ok().map(|m| m.uid());
    drop(file);
    let _ = std::fs::remove_file(&probe);
    uid
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

/// The `unix://` URI tonic parses natively.
pub fn socket_uri(path: &Path) -> String {
    format!("unix://{}", path.display())
}

/// tonic native unix endpoint: `Endpoint::from_shared(format!("unix://{}", path))?.connect().await`.
pub async fn connect(cfg: &ClientConfig, operator: bool) -> Result<tonic::transport::Channel, CliError> {
    let (path, which) = if operator {
        (cfg.operator_socket(), OPERATOR_SOCKET)
    } else {
        (cfg.agent_socket(), AGENT_SOCKET)
    };
    if std::fs::symlink_metadata(&path).is_err() {
        return Err(CliError::Unreachable(format!(
            "no {which} at {} (is aad running? start it with `aa daemon`, or set {RUNTIME_DIR_ENV})",
            path.display()
        )));
    }
    let endpoint = tonic::transport::Endpoint::from_shared(socket_uri(&path))
        .map_err(|e| CliError::Unreachable(format!("invalid socket path {}: {e}", path.display())))?
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("aa-cli/", env!("CARGO_PKG_VERSION")))
        .map_err(|e| CliError::Unreachable(format!("invalid user agent: {e}")))?;
    endpoint
        .connect()
        .await
        .map_err(|e| CliError::Unreachable(format!("cannot connect to {}: {}", path.display(), error_chain(&e))))
}

/// `e: source: source...` on one line.
pub(crate) fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(src) = cur {
        let msg = src.to_string();
        if !s.contains(&msg) {
            s.push_str(": ");
            s.push_str(&msg);
        }
        cur = src.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_live_in_runtime_dir() {
        let cfg = ClientConfig {
            runtime_dir: PathBuf::from("/r/t"),
        };
        assert_eq!(cfg.agent_socket(), PathBuf::from("/r/t/agent.sock"));
        assert_eq!(cfg.operator_socket(), PathBuf::from("/r/t/operator.sock"));
        assert_eq!(socket_uri(&cfg.agent_socket()), "unix:///r/t/agent.sock");
    }

    #[test]
    fn fallback_uses_uid() {
        assert_eq!(fallback_runtime_dir(Some(501)), PathBuf::from("/tmp/aa-501"));
        assert!(current_uid().is_some());
    }

    #[test]
    fn default_respects_socket_budget() {
        let d = default_runtime_dir();
        assert!(d.is_absolute());
        assert!(d.join(OPERATOR_SOCKET).as_os_str().len() <= MAX_SOCKET_PATH || d.starts_with("/tmp"));
    }

    #[tokio::test]
    async fn missing_socket_is_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ClientConfig {
            runtime_dir: dir.path().to_path_buf(),
        };
        for operator in [false, true] {
            match connect(&cfg, operator).await {
                Err(CliError::Unreachable(m)) => assert!(m.contains(".sock"), "{m}"),
                other => panic!("expected Unreachable, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn non_socket_file_is_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(AGENT_SOCKET), b"not a socket").unwrap();
        let cfg = ClientConfig {
            runtime_dir: dir.path().to_path_buf(),
        };
        assert!(matches!(connect(&cfg, false).await, Err(CliError::Unreachable(_))));
    }
}
