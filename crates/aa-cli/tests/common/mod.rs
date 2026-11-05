//! Shared harness for the cross-package e2e tests: a real `aad` process on a temp runtime dir and workspace, and
//! the real `aa` binary run against it.
//!
//! `aad` comes from `$AA_AAD_BIN` if set; otherwise it is built once per test process with
//! `cargo build -p aa-engine --bin aad` into the same target dir as this test (cargo releases its build lock before
//! running tests, and the build is a no-op when up to date).

#![allow(dead_code)]

use serde_json::Value;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long `aad` may take to answer `aa status` after spawn.
const STARTUP: Duration = Duration::from_secs(30);

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

pub fn aa_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aa"))
}

/// Prints a skip reason (a skip never counts as coverage; the reason says what to set to run it).
pub fn skip(test: &str, reason: &str) {
    eprintln!("SKIP {test}: {reason}");
}

pub fn aad_bin() -> PathBuf {
    static AAD: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    AAD.get_or_init(build_aad)
        .clone()
        .unwrap_or_else(|e| panic!("cannot obtain aad: {e}"))
}

fn build_aad() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("AA_AAD_BIN").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let aa = aa_bin();
    let profile_dir = aa.parent().ok_or("aa has no parent dir")?.to_path_buf();
    let sibling = profile_dir.join("aad");
    let target_dir = profile_dir.parent().ok_or("profile dir has no parent")?.to_path_buf();
    let profile = profile_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("debug")
        .to_string();
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "--quiet", "-p", "aa-engine", "--bin", "aad", "--target-dir"])
        .arg(&target_dir)
        .current_dir(repo_root());
    match profile.as_str() {
        "debug" => {}
        "release" => {
            cmd.arg("--release");
        }
        other => {
            cmd.args(["--profile", other]);
        }
    }
    let out = cmd.output().map_err(|e| format!("cannot run cargo: {e}"))?;
    if out.status.success() && sibling.exists() {
        return Ok(sibling);
    }
    if sibling.exists() {
        eprintln!(
            "warning: `cargo build -p aa-engine --bin aad` failed; using the existing {}\n{}",
            sibling.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        return Ok(sibling);
    }
    Err(format!(
        "cargo build -p aa-engine --bin aad failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// One `aa` run.
#[derive(Debug)]
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    fn from_output(o: Output) -> Self {
        use std::os::unix::process::ExitStatusExt as _;
        let code = o.status.code().unwrap_or_else(|| 128 + o.status.signal().unwrap_or(0));
        Run {
            code,
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    /// stdout as JSON (the `--full-output` envelope when that flag was used).
    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {self}"))
    }

    /// `.data` of the envelope.
    pub fn data(&self) -> Value {
        self.json()["data"].clone()
    }

    /// `.error` of the envelope.
    pub fn error(&self) -> Value {
        self.json()["error"].clone()
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit {}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

/// Runs `aa` with the given runtime dir and arguments (no engine required).
pub fn aa_with_runtime(runtime_dir: &Path, args: &[&str]) -> Run {
    let out = Command::new(aa_bin())
        .args(args)
        .env("AA_RUNTIME_DIR", runtime_dir)
        .stdin(Stdio::null())
        .output()
        .expect("run aa");
    Run::from_output(out)
}

/// A live `aad` with its own temp workspace, runtime dir and policy.
pub struct Engine {
    _tmp: tempfile::TempDir,
    /// Canonical temp root (also the policy's read root by default).
    pub root: PathBuf,
    pub runtime_dir: PathBuf,
    pub workspace: PathBuf,
    pub policy: PathBuf,
    pub log: PathBuf,
    child: Option<Child>,
}

/// `examples/policy.toml` with its `read_roots` line replaced.
pub fn policy_with_read_roots(read_roots: &[&Path]) -> String {
    let text = std::fs::read_to_string(repo_root().join("examples/policy.toml")).expect("examples/policy.toml");
    let roots: Vec<String> = read_roots
        .iter()
        .map(|p| format!("{:?}", p.to_string_lossy()))
        .collect();
    let mut replaced = false;
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("read_roots") {
                replaced = true;
                format!("read_roots = [{}]", roots.join(", "))
            } else {
                l.to_string()
            }
        })
        .collect();
    assert!(replaced, "examples/policy.toml has no read_roots line");
    lines.join("\n") + "\n"
}

fn short_tempdir() -> tempfile::TempDir {
    let t = tempfile::Builder::new().prefix("aa-e2e").tempdir().expect("tempdir");
    let canon = t.path().canonicalize().expect("canonical tempdir");
    if canon.join("rt/operator.sock").as_os_str().len() <= 100 {
        return t;
    }
    tempfile::Builder::new()
        .prefix("aa-e2e")
        .tempdir_in("/tmp")
        .expect("tempdir in /tmp")
}

impl Engine {
    /// Starts `aad` with `examples/policy.toml` and `read_roots = [<tmp>]`.
    pub fn start() -> Self {
        Self::start_with(|root| vec![root.to_path_buf()])
    }

    pub fn start_with(read_roots: impl FnOnce(&Path) -> Vec<PathBuf>) -> Self {
        let tmp = short_tempdir();
        let root = tmp.path().canonicalize().expect("canonical tempdir");
        let runtime_dir = root.join("rt");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&runtime_dir)
            .expect("runtime dir");
        let workspace = root.join("ws");
        let roots = read_roots(&root);
        let roots: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
        let policy = root.join("policy.toml");
        {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&policy)
                .unwrap();
            f.write_all(policy_with_read_roots(&roots).as_bytes()).unwrap();
        }
        let log = root.join("aad.log");
        let log_file = std::fs::File::create(&log).expect("aad log");
        let child = Command::new(aad_bin())
            .arg("--workspace")
            .arg(&workspace)
            .arg("--runtime-dir")
            .arg(&runtime_dir)
            .arg("--policy")
            .arg(&policy)
            .arg("--log")
            .arg("info")
            .env("AA_HOME", &workspace)
            .env("AA_RUNTIME_DIR", &runtime_dir)
            .stdin(Stdio::null())
            .stdout(log_file.try_clone().unwrap())
            .stderr(log_file)
            .spawn()
            .expect("spawn aad");
        let mut e = Engine {
            _tmp: tmp,
            root,
            runtime_dir,
            workspace,
            policy,
            log,
            child: Some(child),
        };
        e.wait_ready();
        e
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + STARTUP;
        loop {
            if self.runtime_dir.join("agent.sock").exists() {
                let r = self.aa(&["status"]);
                if r.code == 0 {
                    return;
                }
            }
            if let Some(status) = self.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                panic!("aad exited during startup ({status}):\n{}", self.log_text());
            }
            if Instant::now() >= deadline {
                panic!(
                    "aad did not answer `aa status` within {STARTUP:?}:\n{}",
                    self.log_text()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn operator_socket(&self) -> PathBuf {
        self.runtime_dir.join("operator.sock")
    }

    pub fn chmod(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    /// `aa <args>` against this engine (plain output).
    pub fn aa(&self, args: &[&str]) -> Run {
        let out = Command::new(aa_bin())
            .args(args)
            .env("AA_RUNTIME_DIR", &self.runtime_dir)
            .env("AA_HOME", &self.workspace)
            .stdin(Stdio::null())
            .output()
            .expect("run aa");
        Run::from_output(out)
    }

    /// `aa --json --full-output <args>`: stdout is always the envelope.
    pub fn aa_env(&self, args: &[&str]) -> Run {
        let mut all = vec!["--json", "--full-output"];
        all.extend_from_slice(args);
        self.aa(&all)
    }

    /// `aa --json --full-output <args>`, asserting the exit code.
    pub fn ok(&self, args: &[&str]) -> Value {
        let r = self.aa_env(args);
        assert_eq!(r.code, 0, "aa {args:?} failed\n{r}\n--- aad log\n{}", self.log_text());
        r.data()
    }

    /// All capability summaries of `target`, keyed by id.
    pub fn capabilities(&self, target: &str) -> std::collections::BTreeMap<String, Value> {
        let d = self.ok(&["list", target]);
        d["capabilities"]
            .as_array()
            .expect("capabilities array")
            .iter()
            .map(|c| (c["id"].as_str().unwrap_or_default().to_string(), c.clone()))
            .collect()
    }

    /// All audit entries.
    pub fn audit_all(&self) -> Vec<Value> {
        let d = self.ok(&["audit", "--last", "1000000"]);
        d["entries"].as_array().cloned().unwrap_or_default()
    }

    pub fn shutdown(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Restore permissions a test may have removed so the temp dir can be deleted.
        let _ = std::fs::set_permissions(self.operator_socket(), std::fs::Permissions::from_mode(0o600));
        self.shutdown();
    }
}

/// Proto3-JSON omits zero enum values: a missing admission state is WITHHELD.
pub fn admission(cap: &Value) -> &str {
    cap["admission"].as_str().unwrap_or("ADMISSION_STATE_WITHHELD")
}

/// The withheld reasons of a capability summary (`WITHHELD_REASON_*` names).
pub fn withheld_reasons(cap: &Value) -> Vec<String> {
    cap["withheld"]
        .as_array()
        .map(|ws| {
            ws.iter()
                .filter_map(|w| w["reason"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// A uint64 rendered by proto3-JSON (string) or as a number.
pub fn as_u64(v: &Value) -> u64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Number(n) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

/// `git` with an isolated config (no user/system config, no signing hooks).
pub fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::null())
        .status()
        .expect("run git");
    assert!(st.success(), "git {args:?} failed");
}

/// A one-commit repository at `repo`.
pub fn make_repo(repo: &Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q"]);
    std::fs::write(repo.join("README"), "hi\n").unwrap();
    git(repo, &["add", "README"]);
    git(
        repo,
        &[
            "-c",
            "user.name=me",
            "-c",
            "user.email=me@localhost",
            "commit",
            "-qm",
            "init",
        ],
    );
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}
