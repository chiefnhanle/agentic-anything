//! Overlay (`target.aa.yaml`, `schema: aa.overlay/v1`) serde types mirroring architecture §7.2; unknown keys
//! are rejected.
//!
//! `Overlay` and the two functions are frozen (build-plan §3.1). The nested types follow the §7.3 git example.
//! [`parse_overlay`] also validates every enum-like name (scope kinds, arg styles, predicate ops, effect fields),
//! so a typo fails at parse time instead of silently dropping a curation.

use crate::pb;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    pub schema: String,
    pub target: String,
    pub display_name: Option<String>,
    pub mcp_prefix: Option<String>,
    pub locator: Option<Locator>,
    pub authorization: Option<String>,
    #[serde(default)]
    pub harvest: Vec<HarvestSpec>,
    #[serde(default)]
    pub defaults: Defaults,
    pub state_handle: Option<StateHandle>,
    #[serde(default)]
    pub exclude: Vec<Exclude>,
    #[serde(default)]
    pub fixtures: Vec<FixtureSpec>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, CapabilityOverlay>,
    #[serde(default)]
    pub assumed: Vec<Assumed>,
}

/// Mirrors `pb::TargetLocator` (exactly one should be set).
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub executable: Option<String>,
    pub python_console_script: Option<String>,
    pub bundle_path: Option<String>,
    pub service_origin: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HarvestSpec {
    /// e.g. `"help-text@1"`.
    pub harvester: String,
    /// Curated include-list of native commands.
    #[serde(default)]
    pub commands: Vec<String>,
    /// e.g. `["{command}", "-h"]`.
    #[serde(default)]
    pub help_argv: Vec<String>,
    #[serde(default)]
    pub options: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default)]
    pub binding: BindingDefaults,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BindingDefaults {
    #[serde(default)]
    pub global_args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub deadline_ms: Option<u32>,
    pub max_output_bytes: Option<u64>,
    pub usage_exit_code: Option<i32>,
    pub usage_stderr_marker: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StateHandle {
    /// e.g. `"--project"`.
    pub flag: String,
    /// Input property name when agent-supplied.
    pub param: Option<String>,
    #[serde(default)]
    pub engine_supplied: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Exclude {
    pub capability: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureSpec {
    pub id: String,
    #[serde(default)]
    pub steps: Vec<FixtureStepSpec>,
    /// Capability id whose success marks the fixture ready.
    pub ready: Option<String>,
}

/// Exactly one of `argv` | `file` | `capability` | `include_fixture`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureStepSpec {
    pub argv: Option<Vec<String>>,
    /// Relative to the root (argv steps only).
    pub cwd: Option<String>,
    pub file: Option<FileStepSpec>,
    pub capability: Option<CapabilityCallSpec>,
    pub include_fixture: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileStepSpec {
    pub path: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCallSpec {
    pub id: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityOverlay {
    /// Partial binding override (operator-only).
    pub binding: Option<BindingOverride>,
    /// Shorthand for `binding.fixed_args`.
    #[serde(default)]
    pub fixed_args: Vec<String>,
    pub cwd_pointer: Option<String>,
    /// Raise only: field name -> value.
    #[serde(default)]
    pub effects: BTreeMap<String, bool>,
    /// Raise only.
    #[serde(default)]
    pub branches: Vec<BranchSpec>,
    #[serde(default)]
    pub scopes: Vec<ScopeSpec>,
    pub oracle: Option<OracleSpec>,
    #[serde(default)]
    pub examples: Vec<ExampleSpec>,
    #[serde(default)]
    pub core: bool,
    pub use_when: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BindingOverride {
    pub command_path: Option<Vec<String>>,
    pub fixed_args: Option<Vec<String>>,
    pub args: Option<Vec<ArgSpec>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ArgSpec {
    pub pointer: String,
    /// `ArgStyle` name, e.g. `"OPTION"`.
    pub style: String,
    pub flag: Option<String>,
    pub position: Option<u32>,
    /// JSON Schema of the input property, for a parameter the harvest did not list (e.g. `git log --max-count`,
    /// which `git log -h` omits). Absent: the harvested property schema, else a default for the style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BranchSpec {
    pub pointer: String,
    /// Triggering value; absent = "present and truthy".
    pub equals: Option<serde_json::Value>,
    #[serde(default)]
    pub effects: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ScopeSpec {
    pub pointer: String,
    /// `ScopeKind` name, e.g. `"READ_PATH"`.
    pub kind: String,
}

/// Exactly one of `read_back` | `negative` | `file`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct OracleSpec {
    pub read_back: Option<OracleCallSpec>,
    pub negative: Option<OracleCallSpec>,
    pub file: Option<FileOracleSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct OracleCallSpec {
    pub capability: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub predicates: Vec<PredicateSpec>,
    /// Negative oracles only: the bogus-id control input.
    pub bogus_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PredicateSpec {
    pub pointer: String,
    /// `PredicateOp` name, e.g. `"CONTAINS"`.
    pub op: String,
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileOracleSpec {
    pub path_pointer: String,
    /// Hex of the expected magic prefix.
    pub magic_prefix: Option<String>,
    pub min_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExampleSpec {
    pub input: serde_json::Value,
    pub fixture: Option<String>,
}

/// Mirror of an `AssumeEffects` audit entry; counts only if `audit_seq` verifies in the chain.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Assumed {
    pub field: String,
    pub value: bool,
    #[serde(default)]
    pub glob: String,
    pub reason: String,
    pub audit_seq: u64,
}

/// The `schema:` value of an overlay.
pub const OVERLAY_SCHEMA: &str = "aa.overlay/v1";

fn bad(msg: impl Into<String>) -> crate::AaError {
    crate::AaError::new(
        pb::ErrorCode::ConfigurationInvalid,
        msg,
        "fix target.aa.yaml (schema aa.overlay/v1, architecture §7.2) and recompile: aa discover <target>",
    )
}

fn enum_name<T>(kind: &str, prefix: &str, s: &str, f: impl Fn(&str) -> Option<T>) -> crate::AaResult<T> {
    let up = s.trim().to_ascii_uppercase().replace('-', "_");
    let full = format!("{prefix}{}", up.trim_start_matches(prefix));
    f(&full).ok_or_else(|| bad(format!("unknown {kind} {s:?}")))
}

/// `READ_PATH` / `SCOPE_KIND_READ_PATH` / `read_path`.
pub fn scope_kind(s: &str) -> crate::AaResult<pb::ScopeKind> {
    match enum_name("scope kind", "SCOPE_KIND_", s, pb::ScopeKind::from_str_name)? {
        pb::ScopeKind::Unspecified => Err(bad(format!("scope kind {s:?} is not allowed"))),
        k => Ok(k),
    }
}

/// `OPTION` / `ARG_STYLE_OPTION` / `option`.
pub fn arg_style(s: &str) -> crate::AaResult<pb::ArgStyle> {
    match enum_name("arg style", "ARG_STYLE_", s, pb::ArgStyle::from_str_name)? {
        pb::ArgStyle::Unspecified => Err(bad(format!("arg style {s:?} is not allowed"))),
        k => Ok(k),
    }
}

/// `CONTAINS` / `PREDICATE_OP_CONTAINS` / `contains`.
pub fn predicate_op(s: &str) -> crate::AaResult<pb::PredicateOp> {
    match enum_name("predicate op", "PREDICATE_OP_", s, pb::PredicateOp::from_str_name)? {
        pb::PredicateOp::Unspecified => Err(bad(format!("predicate op {s:?} is not allowed"))),
        k => Ok(k),
    }
}

fn pointer(what: &str, p: &str) -> crate::AaResult<()> {
    if p.starts_with('/') && p.len() > 1 {
        Ok(())
    } else {
        Err(bad(format!("{what} {p:?} must be a JSON pointer like \"/repo\"")))
    }
}

fn effect_field(what: &str, f: &str) -> crate::AaResult<()> {
    crate::effects::field_name(f)
        .map(|_| ())
        .map_err(|_| bad(format!("{what}: unknown effect field {f:?}")))
}

fn validate_oracle_call(id: &str, o: &OracleCallSpec, negative: bool) -> crate::AaResult<()> {
    if o.capability.trim().is_empty() {
        return Err(bad(format!("capabilities.{id}.oracle: capability is required")));
    }
    if o.predicates.is_empty() {
        return Err(bad(format!(
            "capabilities.{id}.oracle: at least one predicate is required"
        )));
    }
    for p in &o.predicates {
        pointer(&format!("capabilities.{id}.oracle predicate pointer"), &p.pointer)?;
        predicate_op(&p.op)?;
    }
    if !negative && o.bogus_input.is_some() {
        return Err(bad(format!(
            "capabilities.{id}.oracle: bogus_input is only valid on a negative oracle"
        )));
    }
    Ok(())
}

/// Structural validation beyond serde (`deny_unknown_fields` already rejected unknown keys).
pub fn validate_overlay(o: &Overlay) -> crate::AaResult<()> {
    if o.schema != OVERLAY_SCHEMA {
        return Err(bad(format!("overlay schema {:?} is not {OVERLAY_SCHEMA:?}", o.schema)));
    }
    crate::ids::validate_target_id(&o.target).map_err(|e| bad(e.message))?;
    if let Some(p) = &o.mcp_prefix {
        let ok = !p.is_empty() && p.len() <= 40 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !ok {
            return Err(bad(format!("mcp_prefix {p:?} must match [a-zA-Z0-9_-]{{1,40}}")));
        }
    }
    if let Some(l) = &o.locator {
        let n = [
            &l.executable,
            &l.python_console_script,
            &l.bundle_path,
            &l.service_origin,
        ]
        .iter()
        .filter(|v| v.is_some())
        .count();
        if n > 1 {
            return Err(bad(
                "locator: set exactly one of executable | python_console_script | bundle_path | service_origin",
            ));
        }
    }
    for h in &o.harvest {
        if h.harvester.trim().is_empty() {
            return Err(bad("harvest[].harvester is required"));
        }
    }
    if let Some(sh) = o.state_handle.as_ref().filter(|sh| !sh.flag.starts_with('-')) {
        return Err(bad(format!(
            "state_handle.flag {:?} must be an option like --project",
            sh.flag
        )));
    }
    let prefix = format!("{}.", o.target);
    for e in &o.exclude {
        if !e.capability.starts_with(&prefix) {
            return Err(bad(format!(
                "exclude: {:?} is not a capability of target {}",
                e.capability, o.target
            )));
        }
    }
    let mut fixture_ids = std::collections::BTreeSet::new();
    for f in &o.fixtures {
        if f.id.trim().is_empty() || !fixture_ids.insert(f.id.as_str()) {
            return Err(bad(format!("fixtures: id {:?} is empty or duplicated", f.id)));
        }
        for (i, s) in f.steps.iter().enumerate() {
            let n = [
                s.argv.is_some(),
                s.file.is_some(),
                s.capability.is_some(),
                s.include_fixture.is_some(),
            ]
            .iter()
            .filter(|b| **b)
            .count();
            if n != 1 {
                return Err(bad(format!(
                    "fixtures.{}.steps[{i}]: set exactly one of argv | file | capability | include_fixture",
                    f.id
                )));
            }
            if s.argv.as_ref().is_some_and(Vec::is_empty) {
                return Err(bad(format!("fixtures.{}.steps[{i}]: argv is empty", f.id)));
            }
            if s.cwd.is_some() && s.argv.is_none() {
                return Err(bad(format!(
                    "fixtures.{}.steps[{i}]: cwd is only valid with argv",
                    f.id
                )));
            }
        }
    }
    for (id, c) in &o.capabilities {
        if !id.starts_with(&prefix) {
            return Err(bad(format!(
                "capabilities: {id:?} is not a capability of target {}",
                o.target
            )));
        }
        for f in c.effects.keys() {
            effect_field(&format!("capabilities.{id}.effects"), f)?;
        }
        for b in &c.branches {
            pointer(&format!("capabilities.{id}.branches pointer"), &b.pointer)?;
            for f in b.effects.keys() {
                effect_field(&format!("capabilities.{id}.branches"), f)?;
            }
        }
        for s in &c.scopes {
            pointer(&format!("capabilities.{id}.scopes pointer"), &s.pointer)?;
            scope_kind(&s.kind)?;
        }
        if let Some(p) = &c.cwd_pointer {
            pointer(&format!("capabilities.{id}.cwd_pointer"), p)?;
        }
        if let Some(b) = &c.binding {
            if b.command_path.as_ref().is_some_and(Vec::is_empty) {
                return Err(bad(format!("capabilities.{id}.binding.command_path is empty")));
            }
            for a in b.args.iter().flatten() {
                pointer(&format!("capabilities.{id}.binding.args pointer"), &a.pointer)?;
                let style = arg_style(&a.style)?;
                if style != pb::ArgStyle::Positional && a.flag.as_deref().is_none_or(|f| !f.starts_with('-')) {
                    return Err(bad(format!(
                        "capabilities.{id}.binding.args {}: a non-positional arg needs a flag",
                        a.pointer
                    )));
                }
                if a.schema.as_ref().is_some_and(|s| !s.is_object()) {
                    return Err(bad(format!(
                        "capabilities.{id}.binding.args {}: schema must be a JSON Schema object",
                        a.pointer
                    )));
                }
            }
        }
        if let Some(or) = &c.oracle {
            let n = [or.read_back.is_some(), or.negative.is_some(), or.file.is_some()]
                .iter()
                .filter(|b| **b)
                .count();
            if n != 1 {
                return Err(bad(format!(
                    "capabilities.{id}.oracle: set exactly one of read_back | negative | file"
                )));
            }
            if let Some(rb) = &or.read_back {
                validate_oracle_call(id, rb, false)?;
            }
            if let Some(ng) = &or.negative {
                validate_oracle_call(id, ng, true)?;
                if ng.bogus_input.is_none() {
                    return Err(bad(format!(
                        "capabilities.{id}.oracle.negative: bogus_input is required"
                    )));
                }
            }
            if let Some(f) = &or.file {
                pointer(&format!("capabilities.{id}.oracle.file.path_pointer"), &f.path_pointer)?;
                if let Some(m) = &f.magic_prefix {
                    hex_bytes(m)?;
                }
            }
        }
        for ex in &c.examples {
            if !ex.input.is_object() {
                return Err(bad(format!("capabilities.{id}.examples: input must be an object")));
            }
        }
    }
    for a in &o.assumed {
        effect_field("assumed", &a.field)?;
        if a.reason.trim().is_empty() {
            return Err(bad(format!("assumed {}: a reason is required", a.field)));
        }
    }
    Ok(())
}

/// Decodes a hex string (the file-oracle magic prefix).
pub fn hex_bytes(s: &str) -> crate::AaResult<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err(bad(format!("magic_prefix {s:?} is not hex")));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| bad(format!("magic_prefix {s:?} is not hex"))))
        .collect()
}

/// Parses and validates `target.aa.yaml`; unknown keys are rejected.
pub fn parse_overlay(yaml: &str) -> crate::AaResult<Overlay> {
    let o: Overlay =
        serde_yaml_ng::from_str(yaml).map_err(|e| bad(format!("target.aa.yaml is not a valid overlay: {e}")))?;
    validate_overlay(&o)?;
    Ok(o)
}

fn yaml_str(s: &str) -> String {
    // A JSON string literal is a valid YAML double-quoted scalar.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// A starter overlay for a freshly registered target (YAML text that `parse_overlay` accepts).
pub fn starter_overlay(target: &pb::Target, harvesters: &[String]) -> String {
    use std::fmt::Write as _;
    let mut y = String::new();
    let _ = writeln!(
        y,
        "# Starter overlay written by `aa target add`; curate it, then run: aa discover {}",
        target.id
    );
    let _ = writeln!(y, "schema: {OVERLAY_SCHEMA}");
    let _ = writeln!(y, "target: {}", yaml_str(&target.id));
    if !target.display_name.is_empty() {
        let _ = writeln!(y, "display_name: {}", yaml_str(&target.display_name));
    }
    let prefix = if target.mcp_prefix.is_empty() {
        target.id.replace('.', "_")
    } else {
        target.mcp_prefix.clone()
    };
    let prefix: String = prefix
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if !prefix.is_empty() {
        let _ = writeln!(y, "mcp_prefix: {}", yaml_str(&prefix));
    }
    let files = target.identity.as_ref().map(|i| i.files.as_slice()).unwrap_or_default();
    if let Some(exe) = files.iter().find(|f| f.role == "executable" && !f.realpath.is_empty()) {
        let _ = writeln!(y, "locator: {{ executable: {} }}", yaml_str(&exe.realpath));
    }
    if !target.authorization_statement.is_empty() {
        let _ = writeln!(y, "authorization: {}", yaml_str(&target.authorization_statement));
    }
    let mut hs: Vec<String> = harvesters
        .iter()
        .map(|h| h.trim())
        .filter(|h| !h.is_empty())
        .map(|h| {
            if h.contains('@') {
                h.to_string()
            } else {
                format!("{h}@1")
            }
        })
        .collect();
    hs.dedup();
    if hs.is_empty() {
        let _ = writeln!(y, "harvest: []");
    } else {
        let _ = writeln!(y, "harvest:");
        for h in hs {
            let _ = writeln!(y, "  - harvester: {}", yaml_str(&h));
        }
    }
    let _ = writeln!(
        y,
        "# defaults: {{ binding: {{ global_args: [], env: {{}}, deadline_ms: 30000, usage_exit_code: 2 }} }}"
    );
    let _ = writeln!(
        y,
        "# exclude: [ {{ capability: {}.<id>, reason: \"...\" }} ]",
        target.id
    );
    let _ = writeln!(
        y,
        "# capabilities: {{ {}.<id>: {{ scopes: [ {{ pointer: /path, kind: READ_PATH }} ], examples: [] }} }}",
        target.id
    );
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT: &str = include_str!("../../tests/fixtures/git.target.aa.yaml");

    #[test]
    fn parses_the_architecture_git_overlay() {
        let o = parse_overlay(GIT).unwrap();
        assert_eq!(o.target, "git");
        assert_eq!(o.fixtures.len(), 2);
        assert_eq!(o.capabilities.len(), 5);
        assert_eq!(o.defaults.binding.usage_exit_code, Some(129));
        let del = &o.capabilities["git.branch.delete"];
        assert_eq!(
            del.binding.as_ref().unwrap().command_path.as_deref(),
            Some(&["branch".to_string()][..])
        );
        assert!(del.oracle.as_ref().unwrap().negative.is_some());
    }

    #[test]
    fn rejects_unknown_keys_and_names() {
        assert!(parse_overlay("schema: aa.overlay/v1\ntarget: git\nbogus: 1\n").is_err());
        assert!(parse_overlay("schema: aa.overlay/v2\ntarget: git\n").is_err());
        let bad_scope = "schema: aa.overlay/v1\ntarget: git\ncapabilities:\n  git.status:\n    scopes: [ { pointer: /repo, kind: READ_ONLY } ]\n";
        assert!(parse_overlay(bad_scope).is_err());
        let bad_field =
            "schema: aa.overlay/v1\ntarget: git\ncapabilities:\n  git.status:\n    effects: { writes_fs: true }\n";
        assert!(parse_overlay(bad_field).is_err());
        let other = "schema: aa.overlay/v1\ntarget: git\ncapabilities:\n  gh.status: {}\n";
        assert!(parse_overlay(other).is_err());
    }

    #[test]
    fn starter_round_trips() {
        let t = pb::Target {
            id: "cli-anything.libreoffice".into(),
            display_name: "LibreOffice \"harness\"".into(),
            authorization_statement: "owner: my files".into(),
            identity: Some(pb::TargetIdentity {
                files: vec![pb::FileIdentity {
                    role: "executable".into(),
                    realpath: "/opt/x: y/bin".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let y = starter_overlay(&t, &["click-info".into(), "help-text@1".into()]);
        let o = parse_overlay(&y).unwrap();
        assert_eq!(o.target, t.id);
        assert_eq!(o.mcp_prefix.as_deref(), Some("cli-anything_libreoffice"));
        assert_eq!(o.harvest.len(), 2);
        assert_eq!(o.harvest[0].harvester, "click-info@1");
        assert_eq!(o.locator.unwrap().executable.as_deref(), Some("/opt/x: y/bin"));
        assert_eq!(o.authorization.as_deref(), Some("owner: my files"));
    }
}
