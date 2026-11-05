//! Deterministic compile: lock = compile(harvest candidates, overlay, rule versions, verification records).
//!
//! Per capability, compile:
//! 1. merges the harvested candidate with the overlay (binding defaults, state handle, per-capability binding
//!    overrides, `cwd_pointer`, scopes, oracles, examples, core, `use_when`);
//! 2. closes the input schema (root `additionalProperties: false`) and derives the TEXT_STDOUT output schema;
//! 3. characterizes it (rule tables, raise only) and adds verified operator assumptions;
//! 4. computes `capability_digest` over the **pre-verification** content (lane-sourced `verify:*` signals are
//!    excluded, otherwise verification would invalidate its own records);
//! 5. folds in the observations of matching verification records and computes admission (or carries the previous
//!    admission over while the capability digest and identity are unchanged);
//! 6. derives level, tier, hints, MCP names and the lock digest.
//!
//! Mode splits: an overlay capability id that no candidate produced, whose `binding.command_path` is given, is a
//! new capability on that native command (`git.branch.list` on `branch`). It inherits only the native
//! description, the overlay-listed `args`, and the native dry-run; the native candidate it splits is withheld
//! `MIXED_AUTHORITY` unless excluded. L0 fails a split whose `command_path` matches no harvested native path.

use super::Overlay;
use super::overlay::{self, ArgSpec, CapabilityOverlay, FixtureSpec, OracleCallSpec, OracleSpec};
use crate::characterize::{self, Characterized, click, cobra, rules};
use crate::{AaError, AaResult, admission, digest, effects, ids, pb};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub struct CompileInput<'a> {
    /// Identity already pinned by the engine.
    pub target: pb::Target,
    pub overlay: &'a Overlay,
    pub overlay_sha256: String,
    pub sources: Vec<pb::SourceRef>,
    /// From `Harvester::candidates` over CAS bytes.
    pub candidates: Vec<crate::api::Candidate>,
    /// Existing records; admission only counts matching digests.
    pub verification: &'a [pb::VerificationRecord],
    /// To verify `assumed[].audit_seq` (ADMIN_EVENT_KIND_EFFECT_ASSUMED).
    pub audit: &'a dyn crate::api::AuditSink,
}

pub struct CompileOutput {
    pub lock: pb::CapabilityLock,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

/// Default binding deadline when neither the harvester nor the overlay sets one.
pub const DEFAULT_DEADLINE_MS: u32 = 30_000;
/// Default stdout cap when neither the harvester nor the overlay sets one.
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
/// `provenance.field_confidence` key marking an overlay-authored (mode-split) binding.
pub const SPLIT_MARKER: &str = "binding";

fn cfg(msg: impl Into<String>) -> AaError {
    AaError::new(
        pb::ErrorCode::ConfigurationInvalid,
        msg,
        "fix target.aa.yaml and recompile: aa discover <target>",
    )
}

fn canonical(v: &Value) -> AaResult<String> {
    String::from_utf8(digest::jcs(v)?).map_err(|e| AaError::internal(format!("JCS output is not UTF-8: {e}")))
}

/// The engine-normalized TEXT_STDOUT output schema: `{"text": str, "lines": [str]}`.
pub fn text_output_schema() -> String {
    canonical(&json!({
        "type": "object",
        "properties": {"text": {"type": "string"}, "lines": {"type": "array", "items": {"type": "string"}}}
    }))
    .unwrap_or_default()
}

/// The property name a top-level JSON pointer designates (`/repo` -> `repo`), RFC 6901 unescaped.
pub fn pointer_property(pointer: &str) -> Option<String> {
    let rest = pointer.strip_prefix('/')?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest.replace("~1", "/").replace("~0", "~"))
}

fn is_split(cap: &pb::Capability) -> bool {
    cap.provenance
        .as_ref()
        .is_some_and(|p| p.native_ids.is_empty() && p.field_confidence.contains_key(SPLIT_MARKER))
}

/// Whether a capability was declared by an overlay mode split (not harvested).
pub fn is_mode_split(cap: &pb::Capability) -> bool {
    is_split(cap)
}

fn cli_mut(cap: &mut pb::Capability) -> Option<&mut pb::CliArgvBinding> {
    match cap.binding.as_mut()?.kind.as_mut()? {
        pb::binding::Kind::Cli(c) => Some(c),
        _ => None,
    }
}

fn cli(cap: &pb::Capability) -> Option<&pb::CliArgvBinding> {
    match cap.binding.as_ref()?.kind.as_ref()? {
        pb::binding::Kind::Cli(c) => Some(c),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Schema helpers
// ---------------------------------------------------------------------------

struct Schema(Map<String, Value>);

impl Schema {
    fn parse(json_text: &str, id: &str) -> AaResult<Self> {
        if json_text.trim().is_empty() {
            return Ok(Self(Map::new()).normalized());
        }
        match serde_json::from_str::<Value>(json_text) {
            Ok(Value::Object(m)) => Ok(Self(m).normalized()),
            Ok(_) => Err(cfg(format!("{id}: input schema is not a JSON object"))),
            Err(e) => Err(cfg(format!("{id}: input schema is not JSON: {e}"))),
        }
    }

    fn normalized(mut self) -> Self {
        self.0.insert("type".into(), Value::from("object"));
        if !self.0.get("properties").is_some_and(Value::is_object) {
            self.0.insert("properties".into(), Value::Object(Map::new()));
        }
        self
    }

    fn props(&mut self) -> &mut Map<String, Value> {
        match self.0.get_mut("properties") {
            Some(Value::Object(m)) => m,
            _ => unreachable!("normalized"),
        }
    }

    fn prop(&self, name: &str) -> Option<&Value> {
        self.0.get("properties")?.get(name)
    }

    fn required(&self) -> Vec<String> {
        self.0
            .get("required")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn set_required(&mut self, mut req: Vec<String>) {
        let mut seen = BTreeSet::new();
        req.retain(|r| seen.insert(r.clone()));
        if req.is_empty() {
            self.0.remove("required");
        } else {
            self.0.insert("required".into(), Value::from(req));
        }
    }

    fn remove(&mut self, name: &str) {
        self.props().remove(name);
        let req = self.required().into_iter().filter(|r| r != name).collect();
        self.set_required(req);
    }

    fn add_required_path(&mut self, name: &str) {
        if self.prop(name).is_none() {
            self.props()
                .insert(name.into(), json!({"type": "string", "format": "path"}));
            let mut req = self.required();
            req.push(name.into());
            self.set_required(req);
        }
    }

    fn close(&mut self) {
        fn close_value(v: &mut Value) {
            let Value::Object(m) = v else { return };
            let is_object = m.get("type").and_then(Value::as_str) == Some("object") || m.contains_key("properties");
            if is_object && !m.contains_key("additionalProperties") {
                m.insert("additionalProperties".into(), Value::Bool(false));
            }
            if let Some(Value::Object(props)) = m.get_mut("properties") {
                props.values_mut().for_each(close_value);
            }
            if let Some(items) = m.get_mut("items") {
                close_value(items);
            }
        }
        self.0.insert("additionalProperties".into(), Value::Bool(false));
        if let Some(Value::Object(props)) = self.0.get_mut("properties") {
            props.values_mut().for_each(close_value);
        }
    }

    fn to_json(&self) -> AaResult<String> {
        canonical(&Value::Object(self.0.clone()))
    }
}

/// The input schema of a capability whose overlay lists `binding.args`: exactly those parameters. Each property
/// is the arg's explicit `schema`, else the harvested property, else a default for its style; a harvested
/// required parameter stays required, and an added positional is required.
fn curated_schema(harvested: &Schema, args: &[ArgSpec]) -> AaResult<Schema> {
    let mut schema = Schema(Map::new()).normalized();
    let harvested_required = harvested.required();
    let mut required = Vec::new();
    for a in args {
        let Some(name) = pointer_property(&a.pointer) else {
            continue;
        };
        let style = overlay::arg_style(&a.style)?;
        let prop = match (&a.schema, harvested.prop(&name)) {
            (Some(s), _) => s.clone(),
            (None, Some(p)) => p.clone(),
            (None, None) => default_prop_schema(style),
        };
        let added = harvested.prop(&name).is_none();
        schema.props().insert(name.clone(), prop);
        if harvested_required.contains(&name) || (added && style == pb::ArgStyle::Positional) {
            required.push(name);
        }
    }
    schema.set_required(required);
    Ok(schema)
}

fn default_prop_schema(style: pb::ArgStyle) -> Value {
    match style {
        pb::ArgStyle::Flag | pb::ArgStyle::NegatableFlag => json!({"type": "boolean"}),
        pb::ArgStyle::RepeatedOption => json!({"type": "array", "items": {"type": "string"}}),
        _ => json!({"type": "string"}),
    }
}

// ---------------------------------------------------------------------------
// Overlay -> proto conversions
// ---------------------------------------------------------------------------

fn arg_mapping(a: &ArgSpec) -> AaResult<pb::ArgMapping> {
    Ok(pb::ArgMapping {
        pointer: a.pointer.clone(),
        style: overlay::arg_style(&a.style)? as i32,
        flag: a.flag.clone().unwrap_or_default(),
        position: a.position.unwrap_or(0),
        no_equals_form: false,
    })
}

fn json_or(v: &Option<Value>, empty: &str) -> AaResult<String> {
    match v {
        None | Some(Value::Null) => Ok(empty.to_string()),
        Some(v) => canonical(v),
    }
}

fn predicates(o: &OracleCallSpec) -> AaResult<Vec<pb::Predicate>> {
    o.predicates
        .iter()
        .map(|p| {
            Ok(pb::Predicate {
                pointer: p.pointer.clone(),
                op: overlay::predicate_op(&p.op)? as i32,
                value_json: json_or(&p.value, "")?,
            })
        })
        .collect()
}

fn oracle(o: &OracleSpec) -> AaResult<Option<pb::Oracle>> {
    use pb::oracle::Kind;
    let args = |c: &OracleCallSpec| {
        if c.args.is_null() {
            Ok("{}".to_string())
        } else {
            canonical(&c.args)
        }
    };
    let kind = if let Some(rb) = &o.read_back {
        Kind::ReadBack(pb::ReadBackOracle {
            capability_id: rb.capability.clone(),
            args_template_json: args(rb)?,
            predicates: predicates(rb)?,
        })
    } else if let Some(ng) = &o.negative {
        Kind::Negative(pb::NegativeOracle {
            capability_id: ng.capability.clone(),
            args_template_json: args(ng)?,
            predicates: predicates(ng)?,
            bogus_input_json: json_or(&ng.bogus_input, "")?,
        })
    } else if let Some(f) = &o.file {
        Kind::File(pb::FileOracle {
            path_pointer: f.path_pointer.clone(),
            magic_prefix: f
                .magic_prefix
                .as_deref()
                .map(overlay::hex_bytes)
                .transpose()?
                .unwrap_or_default(),
            min_size: f.min_size.unwrap_or(0),
        })
    } else {
        return Ok(None);
    };
    Ok(Some(pb::Oracle { kind: Some(kind) }))
}

fn fixture(f: &FixtureSpec) -> AaResult<pb::Fixture> {
    use pb::fixture_step::Step;
    let steps = f
        .steps
        .iter()
        .map(|s| {
            let step = if let Some(argv) = &s.argv {
                Step::Argv(pb::ArgvStep {
                    argv: argv.clone(),
                    cwd: s.cwd.clone().unwrap_or_default(),
                })
            } else if let Some(file) = &s.file {
                Step::File(pb::FileStep {
                    path: file.path.clone(),
                    content: file.content.as_bytes().to_vec(),
                })
            } else if let Some(c) = &s.capability {
                Step::Capability(pb::CapabilityCall {
                    capability_id: c.id.clone(),
                    input_json: if c.input.is_null() {
                        "{}".into()
                    } else {
                        canonical(&c.input)?
                    },
                })
            } else if let Some(inc) = &s.include_fixture {
                Step::IncludeFixture(inc.clone())
            } else {
                return Err(cfg(format!("fixtures.{}: empty step", f.id)));
            };
            Ok(pb::FixtureStep { step: Some(step) })
        })
        .collect::<AaResult<_>>()?;
    Ok(pb::Fixture {
        id: f.id.clone(),
        steps,
        ready_capability_id: f.ready.clone().unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Assumptions
// ---------------------------------------------------------------------------

struct Assumption {
    field: String,
    value: bool,
    glob: Option<globset::GlobMatcher>,
    seq: u64,
    reason: String,
}

/// Overlay `assumed[]` entries whose `audit_seq` verifies as an operator `EFFECT_ASSUMED` admin event with the
/// same field and value (and glob, when the event records one). Anything else is ignored: it can never lower.
fn verified_assumptions(ov: &Overlay, audit: &dyn crate::api::AuditSink) -> Vec<Assumption> {
    let mut out = Vec::new();
    for a in &ov.assumed {
        let Ok(Some(entry)) = audit.entry(a.audit_seq) else {
            continue;
        };
        let Some(pb::audit_entry::Body::Admin(ev)) = &entry.body else {
            continue;
        };
        let operator = entry
            .principal
            .as_ref()
            .is_some_and(|p| p.role == pb::Role::Operator as i32);
        if ev.kind != pb::AdminEventKind::EffectAssumed as i32 || !operator {
            continue;
        }
        let Ok(details) = serde_json::from_str::<Value>(&ev.details_json) else {
            continue;
        };
        let field_ok = details.get("field").and_then(Value::as_str) == Some(a.field.as_str());
        let value_ok = details.get("value").and_then(Value::as_bool) == Some(a.value);
        let glob_ok = details.get("glob").and_then(Value::as_str).is_none_or(|g| g == a.glob);
        let subject_ok =
            ev.subject.is_empty() || ev.subject == ov.target || ev.subject.starts_with(&format!("{}.", ov.target));
        if !(field_ok && value_ok && glob_ok && subject_ok) {
            continue;
        }
        let glob = if a.glob.is_empty() {
            None
        } else {
            match globset::Glob::new(&a.glob) {
                Ok(g) => Some(g.compile_matcher()),
                Err(_) => continue,
            }
        };
        out.push(Assumption {
            field: a.field.clone(),
            value: a.value,
            glob,
            seq: a.audit_seq,
            reason: a.reason.clone(),
        });
    }
    out
}

fn assumption_signals(assumptions: &[Assumption], cap_id: &str) -> Vec<pb::EffectSignal> {
    assumptions
        .iter()
        .filter(|a| a.glob.as_ref().is_none_or(|g| g.is_match(cap_id)))
        .map(|a| pb::EffectSignal {
            field: a.field.clone(),
            value: if a.value { pb::Tri::True } else { pb::Tri::False } as i32,
            confidence: pb::Confidence::Operator as i32,
            source: format!("operator:audit_seq={}", a.seq),
            rationale: a.reason.clone(),
            ..Default::default()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Per-capability build
// ---------------------------------------------------------------------------

fn harvester_of<'a>(target: &'a pb::Target, cap: &pb::Capability) -> Option<&'a str> {
    let shas = &cap.provenance.as_ref()?.source_shas;
    target
        .sources
        .iter()
        .find(|s| shas.contains(&s.sha256))
        .map(|s| s.harvester.split('@').next().unwrap_or(&s.harvester))
}

fn default_role(target: &pb::Target) -> String {
    let files = target.identity.as_ref().map(|i| i.files.as_slice()).unwrap_or_default();
    if !files.iter().any(|f| f.role == "executable") && files.iter().any(|f| f.role == "interpreter") {
        "interpreter".into()
    } else {
        "executable".into()
    }
}

/// Applies overlay defaults, the state handle and per-capability binding overrides to the binding.
fn merge_binding(
    target: &pb::Target,
    ov: &Overlay,
    oc: Option<&CapabilityOverlay>,
    cand: &crate::api::Candidate,
    cap: &mut pb::Capability,
) -> AaResult<()> {
    let harvester = harvester_of(target, cap).map(str::to_owned);
    if cap.binding.as_ref().and_then(|b| b.kind.as_ref()).is_none() {
        cap.binding = Some(pb::Binding {
            kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
                command_path: cand.native_path.clone(),
                ..Default::default()
            })),
        });
    }
    let role = default_role(target);
    let Some(c) = cli_mut(cap) else {
        return Ok(());
    };
    let d = &ov.defaults.binding;
    c.global_args.extend(d.global_args.iter().cloned());
    for (k, v) in &d.env {
        c.env.insert(k.clone(), v.clone());
    }
    c.deadline_ms = d
        .deadline_ms
        .or((c.deadline_ms > 0).then_some(c.deadline_ms))
        .unwrap_or(DEFAULT_DEADLINE_MS);
    c.max_output_bytes = d
        .max_output_bytes
        .or((c.max_output_bytes > 0).then_some(c.max_output_bytes))
        .unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let is_cobra = harvester.as_deref().is_some_and(|h| h.starts_with("cobra"));
    let default_exit = if is_cobra {
        cobra::USAGE_EXIT_CODE
    } else {
        click::USAGE_EXIT_CODE
    };
    c.usage_exit_code = d
        .usage_exit_code
        .or((c.usage_exit_code != 0).then_some(c.usage_exit_code))
        .unwrap_or(default_exit);
    if let Some(m) = &d.usage_stderr_marker {
        c.usage_stderr_marker = m.clone();
    } else if c.usage_stderr_marker.is_empty() && is_cobra {
        c.usage_stderr_marker = cobra::USAGE_STDERR_MARKER.into();
    }
    if c.kill_scope == pb::KillScope::Unspecified as i32 {
        c.kill_scope = pb::KillScope::OwnedGroup as i32;
    }
    if c.executable_role.is_empty() {
        c.executable_role = role;
    }
    if c.output == pb::OutputMode::Unspecified as i32 {
        c.output = pb::OutputMode::TextStdout as i32;
    }
    if c.errors == pb::ErrorConvention::Unspecified as i32 {
        c.errors = click::default_errors(target) as i32;
    }
    if let Some(sh) = &ov.state_handle {
        c.state_handle_flag = sh.flag.clone();
        c.state_handle_engine_supplied = sh.engine_supplied || sh.param.is_none();
        c.state_handle_pointer = match (&sh.param, c.state_handle_engine_supplied) {
            (Some(p), false) => format!("/{p}"),
            _ => String::new(),
        };
    }
    if let Some(oc) = oc {
        if let Some(b) = &oc.binding {
            if let Some(cp) = &b.command_path {
                c.command_path = cp.clone();
            }
            if let Some(args) = &b.args {
                c.args = args.iter().map(arg_mapping).collect::<AaResult<_>>()?;
            }
        }
        match oc.binding.as_ref().and_then(|b| b.fixed_args.as_ref()) {
            Some(f) => c.fixed_args = f.clone(),
            None if !oc.fixed_args.is_empty() => c.fixed_args = oc.fixed_args.clone(),
            None => {}
        }
        if let Some(p) = &oc.cwd_pointer {
            c.cwd_pointer = p.clone();
        }
    }
    Ok(())
}

fn scope_kind_of(s: &overlay::ScopeSpec) -> AaResult<pb::ScopeParam> {
    Ok(pb::ScopeParam {
        pointer: s.pointer.clone(),
        kind: overlay::scope_kind(&s.kind)? as i32,
    })
}

/// Merged scopes: deduplicated, a READ_PATH dropped where the same pointer is a WRITE_PATH, sorted.
fn merge_scopes(mut scopes: Vec<pb::ScopeParam>) -> Vec<pb::ScopeParam> {
    let writes: BTreeSet<String> = scopes
        .iter()
        .filter(|s| s.kind == pb::ScopeKind::WritePath as i32)
        .map(|s| s.pointer.clone())
        .collect();
    scopes.retain(|s| !(s.kind == pb::ScopeKind::ReadPath as i32 && writes.contains(&s.pointer)));
    scopes.sort_by(|a, b| (&a.pointer, a.kind).cmp(&(&b.pointer, b.kind)));
    scopes.dedup();
    scopes
}

struct Built {
    cap: pb::Capability,
    sticky: Vec<pb::Withheld>,
    interactive: bool,
}

fn build(
    target: &pb::Target,
    ov: &Overlay,
    cand: &crate::api::Candidate,
    assumptions: &[Assumption],
) -> AaResult<Built> {
    let mut cap = cand.capability.clone();
    if cap.id.is_empty() {
        let path: Vec<&str> = cand.native_path.iter().map(String::as_str).collect();
        cap.id = ids::capability_id(&target.id, &path);
    }
    cap.target_id = target.id.clone();
    if cap.family.is_empty() {
        cap.family = cand.native_path.first().cloned().unwrap_or_default();
    }
    let interactive = click::interactive(&cand.native_path, cand.interactive);
    cap.returns_target_content = true;
    if cap.lifetime == pb::Lifetime::Unspecified as i32 {
        cap.lifetime = if interactive {
            pb::Lifetime::Session
        } else {
            pb::Lifetime::Oneshot
        } as i32;
    }
    // Derived or overlay-only fields are never taken from a candidate.
    cap.effects = None;
    cap.signals.clear();
    cap.branches.clear();
    cap.level = 0;
    cap.tier = 0;
    cap.hints = None;
    cap.admission = None;
    cap.capability_digest.clear();
    cap.oracles.clear();
    cap.examples.clear();
    cap.core = false;

    let oc = ov.capabilities.get(&cap.id);
    merge_binding(target, ov, oc, cand, &mut cap)?;

    // Input schema: restrict to the overlay's binding.args (if any), drop browser launchers, add cwd /
    // state-handle params, close.
    let mut schema = Schema::parse(&cap.input_schema_json, &cap.id)?;
    if let Some(args) = oc.and_then(|o| o.binding.as_ref()).and_then(|b| b.args.as_ref()) {
        schema = curated_schema(&schema, args)?;
        let kept: BTreeSet<String> = args.iter().filter_map(|a| pointer_property(&a.pointer)).collect();
        // Scopes of parameters the overlay no longer exposes go with them (overlay scopes are added below).
        cap.scopes
            .retain(|s| pointer_property(&s.pointer).is_none_or(|n| kept.contains(&n)));
    }
    let mut extra_scopes = Vec::new();
    let cap_id = cap.id.clone();
    if let Some(c) = cli_mut(&mut cap) {
        // An engine-supplied state handle is never an agent input: no pointer, no pointer scope.
        let engine_supplied_handle = c.state_handle_engine_supplied;
        if engine_supplied_handle {
            c.state_handle_pointer.clear();
        }
        let dropped: Vec<pb::ArgMapping> = c.args.iter().filter(|a| rules::excluded_param(a)).cloned().collect();
        c.args.retain(|a| !rules::excluded_param(a));
        for a in &dropped {
            if let Some(name) = pointer_property(&a.pointer) {
                schema.remove(&name);
            }
        }
        if !c.cwd_pointer.is_empty() {
            let name = pointer_property(&c.cwd_pointer).ok_or_else(|| {
                cfg(format!(
                    "{cap_id}: cwd_pointer {:?} must be a top-level pointer",
                    c.cwd_pointer
                ))
            })?;
            schema.add_required_path(&name);
            extra_scopes.push(pb::ScopeParam {
                pointer: c.cwd_pointer.clone(),
                kind: pb::ScopeKind::ReadPath as i32,
            });
        }
        if !c.state_handle_pointer.is_empty() {
            if let Some(name) = pointer_property(&c.state_handle_pointer) {
                schema.add_required_path(&name);
            }
            extra_scopes.push(pb::ScopeParam {
                pointer: c.state_handle_pointer.clone(),
                kind: pb::ScopeKind::StateHandle as i32,
            });
        }
        // A dropped parameter's scopes go with it.
        let dropped_ptrs: BTreeSet<&str> = dropped.iter().map(|a| a.pointer.as_str()).collect();
        cap.scopes.retain(|s| {
            !dropped_ptrs.contains(s.pointer.as_str())
                && !(engine_supplied_handle && s.kind == pb::ScopeKind::StateHandle as i32)
        });
    }
    schema.close();
    cap.input_schema_json = schema.to_json()?;
    if let Some(c) = cli(&cap) {
        match pb::OutputMode::try_from(c.output).unwrap_or(pb::OutputMode::Unspecified) {
            pb::OutputMode::TextStdout => cap.output_schema_json = text_output_schema(),
            pb::OutputMode::None => cap.output_schema_json.clear(),
            _ => {}
        }
    }

    // Overlay curation.
    if let Some(oc) = oc {
        for s in &oc.scopes {
            extra_scopes.push(scope_kind_of(s)?);
        }
        if let Some(o) = &oc.oracle {
            cap.oracles = oracle(o)?.into_iter().collect();
        }
        cap.examples = oc
            .examples
            .iter()
            .map(|e| {
                Ok(pb::Example {
                    input_json: canonical(&e.input)?,
                    fixture_id: e.fixture.clone().unwrap_or_default(),
                    source: "overlay".into(),
                })
            })
            .collect::<AaResult<_>>()?;
        cap.core = oc.core;
        if let Some(u) = &oc.use_when {
            cap.projections.get_or_insert_with(Default::default).use_when = Some(pb::TargetText {
                text: u.clone(),
                origin: pb::ContentOrigin::Operator as i32,
                source: "overlay".into(),
            });
        }
    }

    // Provenance.
    let prov = cap.provenance.get_or_insert_with(Default::default);
    if prov.native_ids.is_empty() && !prov.field_confidence.contains_key(SPLIT_MARKER) {
        prov.native_ids.push(cand.native_path.join(" "));
    }
    prov.source_shas.sort();
    prov.source_shas.dedup();
    prov.native_ids.sort();
    prov.native_ids.dedup();
    let cli_path = cli(&cap).map(|c| c.command_path.join(" ")).unwrap_or_default();
    cap.projections.get_or_insert_with(Default::default).cli_path = cli_path;

    // Characterize (the candidate seen by the rules is the merged one).
    let merged = crate::api::Candidate {
        native_path: cand.native_path.clone(),
        capability: cap.clone(),
        native_signals: cand.native_signals.clone(),
        interactive: cand.interactive,
    };
    let Characterized {
        mut signals,
        branches,
        scopes,
        withheld,
    } = characterize::characterize(target, &merged, ov);
    signals.extend(assumption_signals(assumptions, &cap.id));
    characterize::sort_signals(&mut signals);
    cap.signals = signals;
    cap.branches = branches;
    let mut all_scopes = std::mem::take(&mut cap.scopes);
    all_scopes.extend(scopes);
    all_scopes.extend(extra_scopes);
    cap.scopes = merge_scopes(all_scopes);
    Ok(Built {
        cap,
        sticky: withheld,
        interactive,
    })
}

/// The candidate a mode split is characterized as.
fn split_candidate(
    target: &pb::Target,
    id: &str,
    oc: &CapabilityOverlay,
    base: Option<&crate::api::Candidate>,
) -> AaResult<crate::api::Candidate> {
    let command_path = oc
        .binding
        .as_ref()
        .and_then(|b| b.command_path.clone())
        .unwrap_or_default();
    let native_path: Vec<String> = id
        .strip_prefix(&format!("{}.", target.id))
        .unwrap_or(id)
        .split('.')
        .map(str::to_owned)
        .collect();
    let base_cap = base.map(|b| &b.capability);
    let base_schema = match base_cap {
        Some(c) => Schema::parse(&c.input_schema_json, &c.id)?,
        None => Schema(Map::new()).normalized(),
    };
    let mut schema = Schema(Map::new()).normalized();
    let base_required = base_schema.required();
    let mut required = Vec::new();
    for a in oc.binding.iter().flat_map(|b| b.args.iter().flatten()) {
        let Some(name) = pointer_property(&a.pointer) else {
            continue;
        };
        let style = overlay::arg_style(&a.style)?;
        let prop = a
            .schema
            .clone()
            .or_else(|| base_schema.prop(&name).cloned())
            .unwrap_or_else(|| default_prop_schema(style));
        schema.props().insert(name.clone(), prop);
        if base_required.contains(&name) || style == pb::ArgStyle::Positional {
            required.push(name);
        }
    }
    schema.set_required(required);
    let mut binding = base_cap.and_then(cli).cloned().unwrap_or_default();
    binding.command_path = command_path;
    binding.args.clear();
    binding.fixed_args.clear();
    let mut field_confidence = std::collections::HashMap::new();
    field_confidence.insert(SPLIT_MARKER.to_string(), pb::Confidence::Operator as i32);
    let capability = pb::Capability {
        id: id.to_string(),
        target_id: target.id.clone(),
        family: native_path.first().cloned().unwrap_or_default(),
        title: base_cap.and_then(|c| c.title.clone()),
        description: base_cap.and_then(|c| c.description.clone()),
        input_schema_json: schema.to_json()?,
        binding: Some(pb::Binding {
            kind: Some(pb::binding::Kind::Cli(binding)),
        }),
        dry_run: base_cap.and_then(|c| c.dry_run.clone()),
        provenance: Some(pb::Provenance {
            source_shas: base_cap
                .and_then(|c| c.provenance.as_ref())
                .map(|p| p.source_shas.clone())
                .unwrap_or_default(),
            native_ids: Vec::new(),
            field_confidence,
            proposal_ids: Vec::new(),
        }),
        ..Default::default()
    };
    Ok(crate::api::Candidate {
        native_path,
        capability,
        native_signals: Vec::new(),
        interactive: false,
    })
}

/// The capability content admission is bound to: `{schema, binding, effects, signals, branches, oracles,
/// scopes, dry_run}`, with lane-sourced (`verify:*`) signals and the effects they lower excluded.
pub fn capability_digest(cap: &pb::Capability) -> crate::AaResult<String> {
    let base: Vec<&pb::EffectSignal> = cap
        .signals
        .iter()
        .filter(|s| !s.source.starts_with("verify:"))
        .collect();
    let owned: Vec<pb::EffectSignal> = base.iter().map(|s| (*s).clone()).collect();
    let effects = effects::combine(&owned, admission::has_write_path_param(cap));
    digest::digest(&json!({
        "schema": cap.input_schema_json,
        "binding": cap.binding,
        "effects": effects,
        "signals": owned,
        "branches": cap.branches,
        "oracles": cap.oracles,
        "scopes": cap.scopes,
        "dry_run": cap.dry_run,
    }))
}

/// The compile-time (sticky) withheld reasons of an admission, sorted.
fn sticky_of(a: &pb::Admission) -> Vec<pb::Withheld> {
    let mut v: Vec<pb::Withheld> = a.withheld.iter().filter(|w| admission::is_sticky(w)).cloned().collect();
    v.sort_by(|a, b| (a.reason, &a.hint).cmp(&(b.reason, &b.hint)));
    v.dedup();
    v
}

fn withheld(reason: pb::WithheldReason, hint: impl Into<String>) -> pb::Withheld {
    pb::Withheld {
        reason: reason as i32,
        hint: hint.into(),
        evidence_ids: Vec::new(),
    }
}

/// Recomputes effects, level, tier and hints from the stored signals.
pub fn derive_projections(cap: &mut pb::Capability) {
    let e = effects::combine(&cap.signals, admission::has_write_path_param(cap));
    let binding = cap.binding.clone().unwrap_or_default();
    cap.level = effects::level(&e) as i32;
    cap.tier = effects::tier(&e, &binding) as i32;
    cap.hints = Some(effects::hints(&e));
    cap.effects = Some(e);
}

/// PURE and deterministic: same inputs => byte-identical canonical lock.
pub fn compile(input: CompileInput<'_>, previous: Option<&pb::CapabilityLock>) -> crate::AaResult<CompileOutput> {
    let ov = input.overlay;
    overlay::validate_overlay(ov)?;
    if ov.target != input.target.id {
        return Err(cfg(format!(
            "overlay is for target {:?}, not {:?}",
            ov.target, input.target.id
        )));
    }
    ids::validate_target_id(&input.target.id)?;

    // Target.
    let mut target = input.target.clone();
    if let Some(d) = &ov.display_name {
        target.display_name = d.clone();
    }
    if let Some(p) = &ov.mcp_prefix {
        target.mcp_prefix = p.clone();
    }
    if let Some(a) = ov
        .authorization
        .as_ref()
        .filter(|_| target.authorization_statement.is_empty())
    {
        target.authorization_statement = a.clone();
    }
    let mut sources = input.sources.clone();
    sources.sort_by(|a, b| {
        (&a.harvester, &a.uri, &a.sha256, &a.action_id).cmp(&(&b.harvester, &b.uri, &b.sha256, &b.action_id))
    });
    sources.dedup();
    target.sources = sources;
    target.fixtures = ov.fixtures.iter().map(fixture).collect::<AaResult<_>>()?;
    if let Some(id) = target.identity.as_mut().filter(|i| i.identity_digest.is_empty()) {
        id.identity_digest = digest::self_digest(id, |i| i.identity_digest.clear())?;
    }
    let identity_digest = target
        .identity
        .as_ref()
        .map(|i| i.identity_digest.clone())
        .unwrap_or_default();

    // Candidates by id (first after a deterministic sort wins).
    let mut cands: Vec<crate::api::Candidate> = input.candidates;
    let key = |c: &crate::api::Candidate| digest::jcs(&c.capability).unwrap_or_default();
    cands.sort_by(|a, b| (&a.native_path, key(a)).cmp(&(&b.native_path, key(b))));
    let mut by_id: BTreeMap<String, crate::api::Candidate> = BTreeMap::new();
    for c in cands {
        let id = if c.capability.id.is_empty() {
            let path: Vec<&str> = c.native_path.iter().map(String::as_str).collect();
            ids::capability_id(&target.id, &path)
        } else {
            c.capability.id.clone()
        };
        by_id.entry(id).or_insert(c);
    }
    let native_by_path: BTreeMap<Vec<String>, String> = by_id
        .iter()
        .map(|(id, c)| {
            let path = cli(&c.capability)
                .map(|b| b.command_path.clone())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| c.native_path.clone());
            (path, id.clone())
        })
        .collect();

    // Overlay-declared capabilities that no candidate produced: mode splits.
    let mut splits: BTreeMap<String, crate::api::Candidate> = BTreeMap::new();
    let mut split_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, oc) in &ov.capabilities {
        if by_id.contains_key(id) {
            continue;
        }
        let Some(cp) = oc.binding.as_ref().and_then(|b| b.command_path.clone()) else {
            return Err(cfg(format!(
                "overlay capability {id} matches no harvested candidate and declares no binding.command_path \
                 (harvested: {})",
                by_id.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        };
        let base_id = native_by_path.get(&cp).cloned();
        let base = base_id.as_ref().and_then(|b| by_id.get(b));
        splits.insert(id.clone(), split_candidate(&target, id, oc, base)?);
        if let Some(b) = base_id {
            split_of.entry(b).or_default().push(id.clone());
        }
    }

    let excluded: BTreeMap<&str, &str> = ov
        .exclude
        .iter()
        .map(|e| (e.capability.as_str(), e.reason.as_str()))
        .collect();
    let assumptions = verified_assumptions(ov, input.audit);
    let prev_caps: BTreeMap<&str, &pb::Capability> = previous
        .map(|l| l.capabilities.iter().map(|c| (c.id.as_str(), c)).collect())
        .unwrap_or_default();
    let prev_identity = previous
        .and_then(|l| l.target.as_ref())
        .and_then(|t| t.identity.as_ref())
        .map(|i| i.identity_digest.as_str());

    let mut caps = Vec::new();
    let mut omit = BTreeSet::new();
    for cand in by_id.values().chain(splits.values()) {
        let Built {
            mut cap,
            mut sticky,
            interactive,
        } = build(&target, ov, cand, &assumptions)?;
        if let Some(parts) = split_of
            .get(&cap.id)
            .filter(|_| !excluded.contains_key(cap.id.as_str()))
        {
            sticky.push(withheld(
                pb::WithheldReason::MixedAuthority,
                format!(
                    "the native command mixes modes; it is served through the overlay mode splits {}",
                    parts.join(", ")
                ),
            ));
        }
        cap.capability_digest = capability_digest(&cap)?;

        // Lane observations: matching records, else carry over from the previous lock.
        let wpp = admission::has_write_path_param(&cap);
        let matching = admission::matching_records(&cap, input.verification, "", &identity_digest);
        let prev = prev_caps.get(cap.id.as_str()).filter(|p| {
            p.capability_digest == cap.capability_digest && prev_identity == Some(identity_digest.as_str())
        });
        let carried = matching.is_empty() && prev.is_some();
        if carried {
            if let Some(p) = prev {
                cap.signals
                    .extend(p.signals.iter().filter(|s| s.source.starts_with("verify:")).cloned());
            }
        } else {
            cap.signals.extend(admission::lane_signals(&matching, wpp));
        }
        characterize::sort_signals(&mut cap.signals);
        derive_projections(&mut cap);

        let excluded_reason = excluded.get(cap.id.as_str());
        let hard = sticky
            .iter()
            .any(|w| w.reason == pb::WithheldReason::ExcludedByPolicy as i32);
        if let Some(reason) = excluded_reason {
            sticky.push(withheld(
                pb::WithheldReason::OperatorExcluded,
                format!("overlay exclude: {reason}"),
            ));
        }
        sticky.sort_by(|a, b| (a.reason, &a.hint).cmp(&(b.reason, &b.hint)));
        sticky.dedup();
        cap.admission = Some(if excluded_reason.is_some() || hard {
            pb::Admission {
                state: pb::AdmissionState::Excluded as i32,
                withheld: sticky,
                ..Default::default()
            }
        } else {
            let prior = pb::Admission {
                withheld: sticky.clone(),
                ..Default::default()
            };
            cap.admission = Some(prior);
            let fresh = admission::compute(&cap, input.verification, "", &identity_digest);
            match prev.and_then(|p| p.admission.as_ref()) {
                Some(pa)
                    if carried
                        && pa.state != pb::AdmissionState::Excluded as i32
                        && sticky_of(pa) == sticky
                        && prev.is_some_and(|p| p.level == cap.level) =>
                {
                    pa.clone()
                }
                _ => fresh,
            }
        });
        if interactive || excluded_reason.is_some() || hard {
            omit.insert(cap.id.clone());
        }
        caps.push(cap);
    }
    caps.sort_by(|a, b| a.id.cmp(&b.id));

    // MCP names (deterministic, unique).
    let prefix = if target.mcp_prefix.is_empty() {
        target.id.replace('.', "_")
    } else {
        target.mcp_prefix.clone()
    };
    let mut taken = BTreeSet::new();
    for cap in &mut caps {
        let name = ids::mcp_name(&prefix, &cap.id, &target.id, &taken);
        taken.insert(name.clone());
        let p = cap.projections.get_or_insert_with(Default::default);
        p.mcp_name = name;
        p.mcp_omit = omit.contains(&cap.id);
    }

    let any_servable = caps.iter().any(|c| {
        c.admission
            .as_ref()
            .is_some_and(|a| a.state == pb::AdmissionState::Servable as i32)
    });
    if target.status != pb::TargetStatus::Hardened as i32 {
        target.status = if any_servable {
            pb::TargetStatus::LiveVerified
        } else {
            pb::TargetStatus::Scaffolded
        } as i32;
    }
    target.families = caps
        .iter()
        .map(|c| c.family.clone())
        .filter(|f| !f.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut rule_versions = characterize::rule_versions();
    for s in &target.sources {
        if let Some((name, v)) = s.harvester.split_once('@') {
            rule_versions.insert(name.to_string(), v.to_string());
        }
    }

    let mut lock = pb::CapabilityLock {
        schema: super::lock::LOCK_SCHEMA.into(),
        target: Some(target),
        capabilities: caps,
        rule_versions: rule_versions.into_iter().collect(),
        overlay_sha256: input.overlay_sha256,
        lock_digest: String::new(),
    };
    super::lock::finalize_digest(&mut lock)?;

    let now: BTreeMap<&str, &str> = lock
        .capabilities
        .iter()
        .map(|c| (c.id.as_str(), c.capability_digest.as_str()))
        .collect();
    let before: BTreeMap<&str, &str> = previous
        .map(|l| {
            l.capabilities
                .iter()
                .map(|c| (c.id.as_str(), c.capability_digest.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let added = now
        .keys()
        .filter(|k| !before.contains_key(*k))
        .map(|k| k.to_string())
        .collect();
    let removed = before
        .keys()
        .filter(|k| !now.contains_key(*k))
        .map(|k| k.to_string())
        .collect();
    let changed = now
        .iter()
        .filter(|(k, d)| before.get(*k).is_some_and(|b| b != *d))
        .map(|(k, _)| k.to_string())
        .collect();
    Ok(CompileOutput {
        lock,
        added,
        removed,
        changed,
    })
}
