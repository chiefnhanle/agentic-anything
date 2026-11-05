//! Helpers shared by the harvesters: private scratch roots, run specs, candidate skeletons, small JSON utilities.

use aa_core::{
    AaError, AaResult,
    api::{ConfinementSpec, RunOutput, RunSpec},
    digest, ids, pb,
};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

/// Default per-call binding deadline written into harvested `cli_argv` bindings (overlay defaults override).
pub(crate) const DEFAULT_DEADLINE_MS: u32 = 30_000;
/// Default stdout cap written into harvested bindings.
pub(crate) const DEFAULT_MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn target_text(
    text: impl Into<String>,
    origin: pb::ContentOrigin,
    source: impl Into<String>,
) -> pb::TargetText {
    pb::TargetText {
        text: text.into(),
        origin: origin as i32,
        source: source.into(),
    }
}

/// RFC 8785 canonical JSON text (what `input_schema_json` holds).
pub(crate) fn canonical_json(v: &Value) -> AaResult<String> {
    String::from_utf8(digest::jcs(v)?).map_err(|e| AaError::internal(format!("JCS output is not UTF-8: {e}")))
}

/// `/name` with RFC 6901 escaping.
pub(crate) fn json_pointer(name: &str) -> String {
    format!("/{}", name.replace('~', "~0").replace('/', "~1"))
}

/// Input property name for a native option/placeholder: lowercase, `-`/space/`.` -> `_`, other chars dropped.
pub(crate) fn property_name(native: &str) -> String {
    let mut out = String::new();
    for c in native.trim_start_matches('-').chars() {
        match c {
            'a'..='z' | '0'..='9' | '_' => out.push(c),
            'A'..='Z' => out.push(c.to_ascii_lowercase()),
            '-' | ' ' | '.' | '/' => out.push('_'),
            _ => {}
        }
    }
    out
}

pub(crate) fn identity_file<'a>(target: &'a pb::Target, role: &str) -> Option<&'a pb::FileIdentity> {
    target.identity.as_ref()?.files.iter().find(|f| f.role == role)
}

/// Absolute paths of every pinned file the harvest may exec (the confinement exec allow-list).
pub(crate) fn exec_allow(target: &pb::Target) -> Vec<PathBuf> {
    target
        .identity
        .as_ref()
        .map(|id| {
            id.files
                .iter()
                .filter(|f| f.role == "executable" || f.role == "interpreter" || f.role.starts_with("backend:"))
                .map(|f| PathBuf::from(&f.realpath))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn require_file<'a>(target: &'a pb::Target, role: &str, harvester: &str) -> AaResult<&'a pb::FileIdentity> {
    identity_file(target, role)
        .filter(|f| !f.realpath.is_empty())
        .ok_or_else(|| {
            AaError::new(
                pb::ErrorCode::ConfigurationInvalid,
                format!("{harvester}: target '{}' has no pinned '{role}' file", target.id),
                format!(
                    "re-register the target so its {role} is pinned: aa target add {} …",
                    target.id
                ),
            )
        })
}

/// INTROSPECT runs target code: refuse without the registration's authorization statement (ADR 0009).
pub(crate) fn require_authorization(target: &pb::Target, harvester: &str) -> AaResult<()> {
    if target.authorization_statement.trim().is_empty() {
        return Err(AaError::new(
            pb::ErrorCode::PolicyDenied,
            format!(
                "{harvester} runs target code and target '{}' has no authorization statement",
                target.id
            ),
            format!(
                "re-register with a statement: aa target add {} --authorization \"…\"",
                target.id
            ),
        ));
    }
    Ok(())
}

pub(crate) fn opt_str<'a>(options: &'a Value, key: &str) -> Option<&'a str> {
    options.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub(crate) fn opt_strings(options: &Value, key: &str) -> Vec<String> {
    match options.get(key) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// An option that may only narrow a built-in bound (`min(value, cap)`).
pub(crate) fn opt_bounded(options: &Value, key: &str, cap: u64) -> u64 {
    options.get(key).and_then(Value::as_u64).map_or(cap, |v| v.min(cap))
}

/// A private, 0700, realpath'd scratch root for one harvest run (`home/`, `tmp/`, `cwd/`), removed on drop.
pub(crate) struct Scratch {
    _dir: tempfile::TempDir,
    pub base: PathBuf,
    pub home: PathBuf,
    pub tmp: PathBuf,
    pub cwd: PathBuf,
}

impl Scratch {
    pub(crate) fn create(parent: &Path) -> AaResult<Self> {
        std::fs::create_dir_all(parent)?;
        let dir = tempfile::Builder::new().prefix("aa-harvest-").tempdir_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let base = dir.path().canonicalize()?;
        let (home, tmp, cwd) = (base.join("home"), base.join("tmp"), base.join("cwd"));
        for d in [&home, &tmp, &cwd] {
            std::fs::create_dir(d)?;
        }
        Ok(Self {
            _dir: dir,
            base,
            home,
            tmp,
            cwd,
        })
    }

    /// Scrubbed child env: fixed PATH, private HOME/TMPDIR, C locale, plus harvester-specific extras.
    pub(crate) fn env(&self, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut env = BTreeMap::from([
            ("PATH".to_owned(), "/usr/bin:/bin:/usr/sbin:/sbin".to_owned()),
            ("HOME".to_owned(), self.home.display().to_string()),
            ("TMPDIR".to_owned(), self.tmp.display().to_string()),
            ("LANG".to_owned(), "C".to_owned()),
            ("LC_ALL".to_owned(), "C".to_owned()),
            ("NO_COLOR".to_owned(), "1".to_owned()),
        ]);
        for (k, v) in extra {
            env.insert((*k).to_owned(), (*v).to_owned());
        }
        env
    }

    pub(crate) fn spec(
        &self,
        argv: Vec<String>,
        env: BTreeMap<String, String>,
        deadline: Duration,
        max_stdout: u64,
        target: &pb::Target,
        allow_network: bool,
    ) -> RunSpec {
        RunSpec {
            argv,
            cwd: self.cwd.clone(),
            env,
            stdin: None,
            deadline,
            max_stdout,
            max_stderr_excerpt: 64 * 1024,
            confinement: Some(ConfinementSpec {
                deny_network: !allow_network,
                writable: vec![self.base.clone()],
                exec_allow: exec_allow(target),
            }),
            run_id: ids::new_action_id(),
        }
    }
}

/// Maps a failed or unusable run to a typed error (timeouts, truncation, signals).
pub(crate) fn check_run(harvester: &str, what: &str, out: &RunOutput) -> AaResult<()> {
    if out.timed_out {
        return Err(AaError::new(
            pb::ErrorCode::Timeout,
            format!("{harvester}: {what} timed out after {:?}", out.wall),
            "narrow the harvest (overlay harvest[].commands) and retry: aa discover <target>",
        ));
    }
    if out.stdout_truncated {
        return Err(AaError::new(
            pb::ErrorCode::ResourceConstraint,
            format!(
                "{harvester}: {what} exceeded the stdout cap ({} bytes)",
                out.stdout_total
            ),
            "narrow the harvest (overlay harvest[].commands) and retry: aa discover <target>",
        ));
    }
    if let Some(sig) = out.signal {
        return Err(AaError::new(
            pb::ErrorCode::ExecutionStateUnknown,
            format!("{harvester}: {what} was killed by signal {sig}"),
            "inspect the confinement denials in the audit record: aa audit --last 5",
        ));
    }
    Ok(())
}

pub(crate) fn raw(
    harvester: &str,
    name: &str,
    media_type: &str,
    uri: String,
    authority: pb::DiscoveryAuthority,
    bytes: Vec<u8>,
) -> aa_core::api::RawHarvest {
    aa_core::api::RawHarvest {
        harvester: harvester.to_owned(),
        name: name.to_owned(),
        media_type: media_type.to_owned(),
        uri,
        authority: authority as i32,
        bytes,
    }
}

/// The raw artifacts a harvester produced (matched by `<id>@` prefix).
pub(crate) fn raws_for<'a>(
    id: &str,
    raw: &'a [aa_core::api::RawHarvest],
) -> impl Iterator<Item = &'a aa_core::api::RawHarvest> + 'a {
    let prefix = format!("{id}@");
    raw.iter().filter(move |r| r.harvester.starts_with(&prefix))
}

/// Closed object schema from ordered properties.
pub(crate) fn object_schema(properties: Map<String, Value>, required: Vec<String>) -> Value {
    let mut schema = Map::new();
    schema.insert("type".into(), Value::from("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::from(required));
    }
    schema.insert("additionalProperties".into(), Value::Bool(false));
    Value::Object(schema)
}

/// Everything a harvester knows about one candidate; effects and admission are left to `aa-core` compile.
pub(crate) struct CandidateParts {
    pub native_path: Vec<String>,
    pub title: pb::TargetText,
    pub description: Option<pb::TargetText>,
    pub schema: Value,
    pub binding: pb::CliArgvBinding,
    pub scopes: Vec<pb::ScopeParam>,
    pub dry_run: Option<pb::DryRun>,
    pub source_sha: String,
    pub input_confidence: pb::Confidence,
    pub interactive: bool,
}

pub(crate) fn candidate(target: &pb::Target, parts: CandidateParts) -> AaResult<aa_core::api::Candidate> {
    let path_refs: Vec<&str> = parts.native_path.iter().map(String::as_str).collect();
    let native_id = parts.native_path.join(" ");
    let capability = pb::Capability {
        id: ids::capability_id(&target.id, &path_refs),
        target_id: target.id.clone(),
        family: parts.native_path.first().cloned().unwrap_or_default(),
        title: Some(parts.title),
        description: parts.description,
        input_schema_json: canonical_json(&parts.schema)?,
        scopes: parts.scopes,
        binding: Some(pb::Binding {
            kind: Some(pb::binding::Kind::Cli(parts.binding)),
        }),
        dry_run: parts.dry_run,
        returns_target_content: true,
        lifetime: if parts.interactive {
            pb::Lifetime::Session
        } else {
            pb::Lifetime::Oneshot
        } as i32,
        provenance: Some(pb::Provenance {
            source_shas: vec![parts.source_sha],
            native_ids: vec![native_id.clone()],
            field_confidence: [("input_schema".to_owned(), parts.input_confidence as i32)]
                .into_iter()
                .collect(),
            proposal_ids: Vec::new(),
        }),
        projections: Some(pb::Projections {
            cli_path: native_id,
            ..Default::default()
        }),
        ..Default::default()
    };
    Ok(aa_core::api::Candidate {
        native_path: parts.native_path,
        capability,
        native_signals: Vec::new(),
        interactive: parts.interactive,
    })
}

/// File name of a pinned path, for engine-authored titles.
pub(crate) fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_names_and_pointers() {
        assert_eq!(property_name("--untracked-files"), "untracked_files");
        assert_eq!(property_name("revision-range"), "revision_range");
        assert_eq!(property_name("D"), "d");
        assert_eq!(json_pointer("a/b~c"), "/a~1b~0c");
    }

    #[test]
    fn bounded_options_only_narrow() {
        let o = serde_json::json!({"max_depth": 9, "max_calls": 10});
        assert_eq!(opt_bounded(&o, "max_depth", 4), 4);
        assert_eq!(opt_bounded(&o, "max_calls", 600), 10);
        assert_eq!(opt_bounded(&o, "missing", 600), 600);
    }

    #[test]
    fn scratch_root_is_private_and_removed() {
        let parent = tempfile::tempdir().unwrap();
        let base;
        {
            let s = Scratch::create(parent.path()).unwrap();
            base = s.base.clone();
            assert!(s.home.is_dir() && s.tmp.is_dir() && s.cwd.is_dir());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&s.base).unwrap().permissions().mode();
                assert_eq!(mode & 0o077, 0, "scratch root must be 0700");
            }
            let env = s.env(&[("X", "1")]);
            assert_eq!(env["HOME"], s.home.display().to_string());
            assert_eq!(env["X"], "1");
        }
        assert!(!base.exists());
    }
}
