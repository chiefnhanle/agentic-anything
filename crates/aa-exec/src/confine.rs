//! OS confinement (macOS Seatbelt via `/usr/bin/sandbox-exec`).
//!
//! The profile shape is architecture §9 (verified on this Mac 2026-11-05):
//!
//! ```scheme
//! (version 1)
//! (allow default)
//! (deny network-outbound (require-not (remote unix-socket)))
//! (deny file-write* (require-not (require-any (subpath "<private-root>") (literal "/dev/null") <write scopes…>)))
//! (deny process-exec* (require-not (require-any <literal realpaths of pinned executables…>)))
//! ```
//!
//! Seatbelt matches resolved vnode paths, so every path is emitted both as given and canonicalized (when the
//! two differ and the path exists), e.g. `/tmp/x` and `/private/tmp/x`.

use aa_core::{AaError, AaResult, api::ConfinementSpec, pb};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
/// File-name prefix of the profile written into the run's scratch directory.
pub const PROFILE_PREFIX: &str = "aa-seatbelt-";

pub trait Confiner: Send + Sync {
    fn available(&self) -> bool;
    /// Returns the wrapped argv (e.g. `["/usr/bin/sandbox-exec","-f",<profile>,…argv]`) and writes the profile
    /// into `scratch`.
    fn wrap(
        &self,
        argv: Vec<String>,
        spec: &aa_core::api::ConfinementSpec,
        scratch: &std::path::Path,
    ) -> aa_core::AaResult<Vec<String>>;
}

/// macOS; `available()` = /usr/bin/sandbox-exec exists AND a self-test denies a write.
pub struct SeatbeltConfiner;

pub struct NoConfiner;

fn unavailable(msg: impl Into<String>) -> AaError {
    AaError::new(
        pb::ErrorCode::IsolationNotProven,
        msg,
        "confinement is unavailable on this host; check `aa status` (confinement_available)",
    )
}

/// Seatbelt string literal: `"` and `\` escaped; control characters as `\xHH` (callers reject them anyway).
fn sb_str(p: &str) -> String {
    let mut s = String::with_capacity(p.len() + 2);
    s.push('"');
    for c in p.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            c if c.is_control() => s.push_str(&format!("\\x{:02x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// The path as given plus its canonical form (when it resolves and differs). Relative paths are dropped.
fn path_forms(p: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if !p.is_absolute() {
        return out;
    }
    out.push(p.to_string_lossy().into_owned());
    if let Ok(c) = std::fs::canonicalize(p) {
        let c = c.to_string_lossy().into_owned();
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// Architecture §9 profile; paths escaped.
pub fn seatbelt_profile(spec: &aa_core::api::ConfinementSpec) -> String {
    let mut p = String::from("(version 1)\n(allow default)\n");
    if spec.deny_network {
        p.push_str("(deny network-outbound (require-not (remote unix-socket)))\n");
    }
    let mut writes = vec!["(literal \"/dev/null\")".to_string()];
    for w in &spec.writable {
        for f in path_forms(w) {
            let clause = format!("(subpath {})", sb_str(&f));
            if !writes.contains(&clause) {
                writes.push(clause);
            }
        }
    }
    p.push_str(&format!(
        "(deny file-write* (require-not (require-any {})))\n",
        writes.join(" ")
    ));
    let mut execs = Vec::new();
    for e in &spec.exec_allow {
        for f in path_forms(e) {
            let clause = format!("(literal {})", sb_str(&f));
            if !execs.contains(&clause) {
                execs.push(clause);
            }
        }
    }
    if execs.is_empty() {
        p.push_str("(deny process-exec*)\n");
    } else {
        p.push_str(&format!(
            "(deny process-exec* (require-not (require-any {})))\n",
            execs.join(" ")
        ));
    }
    p
}

fn check_paths(spec: &ConfinementSpec) -> AaResult<()> {
    for p in spec.writable.iter().chain(spec.exec_allow.iter()) {
        let s = p.to_string_lossy();
        if !p.is_absolute() {
            return Err(AaError::invalid(format!("confinement path {s:?} is not absolute")));
        }
        if s.chars().any(char::is_control) {
            return Err(AaError::invalid(format!(
                "confinement path {s:?} contains control characters"
            )));
        }
    }
    Ok(())
}

/// Runs the self-test once per process: a write outside the allowed root must be denied while a write inside
/// it succeeds. Never a shell: `/usr/bin/touch` is exec'd directly by sandbox-exec.
fn seatbelt_self_test() -> bool {
    if !cfg!(target_os = "macos") || !Path::new(SANDBOX_EXEC).is_file() || !Path::new("/usr/bin/touch").is_file() {
        return false;
    }
    let run = || -> std::io::Result<bool> {
        let allowed = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let allowed_dir = std::fs::canonicalize(allowed.path())?;
        let outside_dir = std::fs::canonicalize(outside.path())?;
        let spec = ConfinementSpec {
            deny_network: true,
            writable: vec![allowed_dir.clone()],
            exec_allow: vec![PathBuf::from("/usr/bin/touch")],
        };
        let profile = seatbelt_profile(&spec);
        let denied_target = outside_dir.join("aa-selftest-denied");
        let allowed_target = allowed_dir.join("aa-selftest-allowed");
        let status_of = |target: &Path| {
            std::process::Command::new(SANDBOX_EXEC)
                .arg("-p")
                .arg(&profile)
                .arg("/usr/bin/touch")
                .arg(target)
                .env_clear()
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        };
        let denied = status_of(&denied_target)?;
        let allowed_ok = status_of(&allowed_target)?;
        Ok(!denied.success() && !denied_target.exists() && allowed_ok.success() && allowed_target.exists())
    };
    match run() {
        Ok(ok) => {
            if !ok {
                tracing::warn!("seatbelt self-test failed: a write outside the root was not denied");
            }
            ok
        }
        Err(e) => {
            tracing::warn!(error = %e, "seatbelt self-test could not run");
            false
        }
    }
}

static SEATBELT_OK: OnceLock<bool> = OnceLock::new();

impl Confiner for SeatbeltConfiner {
    fn available(&self) -> bool {
        *SEATBELT_OK.get_or_init(seatbelt_self_test)
    }

    fn wrap(
        &self,
        argv: Vec<String>,
        spec: &aa_core::api::ConfinementSpec,
        scratch: &std::path::Path,
    ) -> aa_core::AaResult<Vec<String>> {
        if !self.available() {
            return Err(unavailable(
                "Seatbelt (sandbox-exec) is not available or failed its self-test",
            ));
        }
        if argv.is_empty() {
            return Err(AaError::invalid("cannot confine an empty argv"));
        }
        check_paths(spec)?;
        let profile = seatbelt_profile(spec);
        let path = write_private_file(scratch, profile.as_bytes())?;
        let mut out = vec![
            SANDBOX_EXEC.to_string(),
            "-f".to_string(),
            path.to_string_lossy().into_owned(),
        ];
        out.extend(argv);
        Ok(out)
    }
}

/// Writes `bytes` to a new, uniquely named 0600 file in `dir` and returns its path.
fn write_private_file(dir: &Path, bytes: &[u8]) -> AaResult<PathBuf> {
    use std::io::Write;
    let err =
        |e: std::io::Error| AaError::internal(format!("cannot write confinement profile in {}: {e}", dir.display()));
    let mut f = tempfile::Builder::new()
        .prefix(PROFILE_PREFIX)
        .suffix(".sb")
        .tempfile_in(dir)
        .map_err(err)?;
    f.write_all(bytes).and_then(|()| f.as_file().sync_all()).map_err(err)?;
    let (_, path) = f.keep().map_err(|e| err(e.error))?;
    Ok(path)
}

impl Confiner for NoConfiner {
    fn available(&self) -> bool {
        false
    }

    fn wrap(
        &self,
        argv: Vec<String>,
        spec: &aa_core::api::ConfinementSpec,
        scratch: &std::path::Path,
    ) -> aa_core::AaResult<Vec<String>> {
        let _ = (argv, spec, scratch);
        Err(unavailable("no confiner is available on this host"))
    }
}

pub fn default_confiner() -> std::sync::Arc<dyn Confiner> {
    if cfg!(target_os = "macos") && SeatbeltConfiner.available() {
        Arc::new(SeatbeltConfiner)
    } else {
        Arc::new(NoConfiner)
    }
}

/// Recognizes confinement denials in (a line of) the child's stderr. Seatbelt reports denials to the system
/// log, not to the child, so this relies on what the child and sandbox-exec print:
/// - `sandbox-exec: execvp() of '<path>' failed: Operation not permitted` (exit 71) → `process-exec:<path>`;
/// - `…: posix_spawn: <path>: …` (a launcher whose re-spawn was denied) → `process-exec:<path>`;
/// - any other line with `Operation not permitted` (EPERM) → `eperm:<line>` (bounded).
pub(crate) fn classify_denial(line: &str, confined: bool) -> Option<String> {
    if !confined {
        return None;
    }
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("sandbox-exec: execvp() of '")
        && let Some(end) = rest.find('\'')
    {
        return Some(format!("process-exec:{}", &rest[..end]));
    }
    // The macOS framework-Python launcher (`bin/python3.x`) re-spawns `Python.app/Contents/MacOS/Python`; a
    // denied spawn prints `python: posix_spawn: <path>: Undefined error: 0`.
    if let Some(i) = line.find("posix_spawn: ") {
        let rest = &line[i + "posix_spawn: ".len()..];
        let path = rest.split(": ").next().unwrap_or(rest);
        if path.starts_with('/') {
            return Some(format!("process-exec:{path}"));
        }
    }
    if line.contains("Operation not permitted") {
        let mut l: String = line.chars().take(200).collect();
        if l.len() < line.len() {
            l.push('…');
        }
        return Some(format!("eperm:{l}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_shape_and_escaping() {
        let spec = ConfinementSpec {
            deny_network: true,
            writable: vec![PathBuf::from("/nonexistent/aa \"q\" \\ root")],
            exec_allow: vec![PathBuf::from("/nonexistent/bin/tool")],
        };
        let p = seatbelt_profile(&spec);
        assert!(p.starts_with("(version 1)\n(allow default)\n"));
        assert!(p.contains("(deny network-outbound (require-not (remote unix-socket)))"));
        assert!(p.contains(r#"(subpath "/nonexistent/aa \"q\" \\ root")"#), "{p}");
        assert!(p.contains(r#"(literal "/dev/null")"#));
        assert!(p.contains(r#"(deny process-exec* (require-not (require-any (literal "/nonexistent/bin/tool"))))"#));

        let open = seatbelt_profile(&ConfinementSpec::default());
        assert!(!open.contains("network-outbound"));
        assert!(open.contains("(deny process-exec*)\n"));
        assert!(sb_str("a\nb").contains("\\x0a"));
    }

    #[test]
    fn canonical_forms_are_added() {
        let d = tempfile::tempdir().unwrap();
        let forms = path_forms(d.path());
        let canon = std::fs::canonicalize(d.path()).unwrap().to_string_lossy().into_owned();
        assert!(forms.contains(&canon));
        assert!(path_forms(Path::new("relative")).is_empty());
    }

    #[test]
    fn no_confiner_refuses() {
        assert!(!NoConfiner.available());
        let e = NoConfiner
            .wrap(vec!["/bin/true".into()], &ConfinementSpec::default(), Path::new("/tmp"))
            .unwrap_err();
        assert_eq!(e.code, pb::ErrorCode::IsolationNotProven);
    }

    #[test]
    fn wrap_rejects_control_chars_and_relative_paths() {
        if !SeatbeltConfiner.available() {
            eprintln!("SKIPPED: sandbox-exec unavailable on this host");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let bad = ConfinementSpec {
            writable: vec![PathBuf::from("/tmp/a\nb")],
            ..Default::default()
        };
        assert!(SeatbeltConfiner.wrap(vec!["/bin/true".into()], &bad, d.path()).is_err());
        let rel = ConfinementSpec {
            exec_allow: vec![PathBuf::from("bin/true")],
            ..Default::default()
        };
        assert!(SeatbeltConfiner.wrap(vec!["/bin/true".into()], &rel, d.path()).is_err());
        let ok = SeatbeltConfiner
            .wrap(vec!["/bin/true".into()], &ConfinementSpec::default(), d.path())
            .unwrap();
        assert_eq!(ok[0], SANDBOX_EXEC);
        assert_eq!(ok[1], "-f");
        assert!(Path::new(&ok[2]).is_file());
        assert_eq!(ok[3], "/bin/true");
    }

    #[test]
    fn denial_classification() {
        assert_eq!(
            classify_denial(
                "sandbox-exec: execvp() of '/bin/sh' failed: Operation not permitted",
                true
            )
            .as_deref(),
            Some("process-exec:/bin/sh")
        );
        assert!(
            classify_denial("touch: /x: Operation not permitted", true)
                .unwrap()
                .starts_with("eperm:")
        );
        assert!(classify_denial("touch: /x: Operation not permitted", false).is_none());
        assert!(classify_denial("all good", true).is_none());
        assert_eq!(
            classify_denial(
                "python: posix_spawn: /opt/x/Python.app/Contents/MacOS/Python: Undefined error: 0",
                true
            )
            .as_deref(),
            Some("process-exec:/opt/x/Python.app/Contents/MacOS/Python")
        );
    }
}
