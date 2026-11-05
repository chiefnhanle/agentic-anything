//! Loading the operator policy file (`aa-policy/1`, architecture §5.5) and deriving the PathScope roots.
//!
//! The rule evaluator is `aa_core::policy` (through [`Semantics`]); the `[paths]` table is read here because
//! the frozen `Policy` type is opaque.

use crate::{
    config::config_error,
    ports::{PolicyEval, Semantics},
};
use aa_core::{AaResult, policy::paths::PathScope};
use serde::Deserialize;
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Used when the policy file is missing: deny everything except observing registered targets.
pub const BUILTIN_POLICY: &str = r#"format = "aa-policy/1"
default = "deny"

[[rule]]
id = "builtin-observe-registered"
effect = "allow"
when = { tier = "observe", target_registered = true }
"#;

/// Always denied, whatever the policy says (architecture §5.5 example deny list).
const ALWAYS_DENY: &[&str] = &[
    "~/.ssh",
    "~/.aws",
    "~/.gnupg",
    "~/.config/agentic-anything",
    "~/Library/Keychains",
];

#[derive(Debug, Default, Deserialize)]
struct PolicyDoc {
    #[serde(default)]
    paths: PathsDoc,
}

#[derive(Debug, Default, Deserialize)]
struct PathsDoc {
    #[serde(default)]
    write_roots: Vec<String>,
    #[serde(default)]
    read_roots: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

/// Canonical roots from the policy plus the engine's own denies.
#[derive(Debug, Clone, Default)]
pub struct Roots {
    pub write_roots: Vec<PathBuf>,
    pub read_roots: Vec<PathBuf>,
    pub deny: Vec<PathBuf>,
}

impl Roots {
    /// The PathScope for one invocation: the session workspace is always readable and writable; write roots
    /// are also readable; extra session read roots were already checked at OpenSession.
    pub fn scope_for(&self, session_workspace: &Path, session_read_roots: &[PathBuf]) -> PathScope {
        let ws = canonical_or_raw(session_workspace);
        let mut write_roots = self.write_roots.clone();
        write_roots.push(ws.clone());
        let mut read_roots = self.read_roots.clone();
        read_roots.extend(self.write_roots.iter().cloned());
        read_roots.extend(session_read_roots.iter().cloned());
        read_roots.push(ws);
        PathScope {
            write_roots,
            read_roots,
            deny: self.deny.clone(),
        }
    }

    /// Whether `p` (canonical) lies inside a policy read or write root and outside every deny.
    pub fn permits_read_root(&self, p: &Path) -> bool {
        let inside = |roots: &[PathBuf]| roots.iter().any(|r| p.starts_with(r));
        !inside(&self.deny) && (inside(&self.read_roots) || inside(&self.write_roots))
    }
}

pub struct LoadedPolicy {
    pub eval: Arc<dyn PolicyEval>,
    pub digest: String,
    pub roots: Roots,
    /// The file it came from, or `None` for [`BUILTIN_POLICY`].
    pub source: Option<PathBuf>,
}

/// Reads `path` (missing => [`BUILTIN_POLICY`]). Refuses a group/other-writable file or one owned by another
/// user (architecture §5.5). `extra_deny` are engine-internal paths (state, audit, runtime dir).
pub fn load(sem: &dyn Semantics, path: &Path, extra_deny: &[PathBuf]) -> AaResult<LoadedPolicy> {
    let (text, source) = match std::fs::metadata(path) {
        Ok(md) => {
            if md.mode() & 0o022 != 0 {
                return Err(config_error(format!(
                    "policy file {} is group/other-writable; chmod go-w it",
                    path.display()
                )));
            }
            let owner = md.uid();
            if owner != crate::config::euid() && owner != 0 {
                return Err(config_error(format!(
                    "policy file {} is owned by uid {owner}, not the engine user",
                    path.display()
                )));
            }
            (std::fs::read_to_string(path)?, Some(path.to_path_buf()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BUILTIN_POLICY.to_string(), None),
        Err(e) => return Err(e.into()),
    };
    let eval = sem.load_policy(&text)?;
    let digest = sem.policy_digest(&text);
    let doc: PolicyDoc = toml::from_str(&text).map_err(|e| config_error(format!("policy [paths]: {e}")))?;
    let canon = |v: &[String]| v.iter().map(|s| canonical_or_raw(&expand_home(s))).collect::<Vec<_>>();
    let mut deny = canon(&doc.paths.deny);
    deny.extend(ALWAYS_DENY.iter().map(|s| canonical_or_raw(&expand_home(s))));
    deny.extend(extra_deny.iter().map(|p| canonical_or_raw(p)));
    Ok(LoadedPolicy {
        eval,
        digest,
        roots: Roots {
            write_roots: canon(&doc.paths.write_roots),
            read_roots: canon(&doc.paths.read_roots),
            deny,
        },
        source,
    })
}

pub fn expand_home(s: &str) -> PathBuf {
    match (s.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(s),
    }
}

/// realpath when the path exists (so `/tmp/x` matches `/private/tmp/x` on macOS); otherwise unchanged.
pub fn canonical_or_raw(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeSemantics;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn missing_file_uses_builtin_and_paths_are_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let sem = FakeSemantics::default();
        let p = load(&sem, &dir.path().join("absent.toml"), &[]).unwrap();
        assert!(p.source.is_none());
        assert!(p.roots.read_roots.is_empty());

        let roots = dir.path().join("roots");
        std::fs::create_dir(&roots).unwrap();
        let file = dir.path().join("policy.toml");
        std::fs::write(
            &file,
            format!(
                "format = \"aa-policy/1\"\n[paths]\nread_roots = [\"{}\"]\ndeny = [\"{}/secret\"]\n",
                roots.display(),
                roots.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let p = load(&sem, &file, &[]).unwrap();
        assert_eq!(p.roots.read_roots, vec![std::fs::canonicalize(&roots).unwrap()]);
        assert!(
            p.roots
                .permits_read_root(&std::fs::canonicalize(&roots).unwrap().join("a"))
        );
        assert!(!p.roots.permits_read_root(Path::new("/etc")));

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        let err = load(&sem, &file, &[]).err().unwrap();
        assert_eq!(err.code, aa_core::pb::ErrorCode::ConfigurationInvalid);
    }

    #[test]
    fn home_expansion() {
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_home("~/.ssh"), PathBuf::from(home).join(".ssh"));
        }
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
        assert_eq!(expand_home("~other"), PathBuf::from("~other"));
    }
}
