//! L0 (static) lane: closed schema, refs resolve, names unique, effects decided-or-withheld.
//!
//! Findings use the most specific withheld reason: `SCHEMA_INVALID` for contract/structure problems,
//! `EFFECTS_UNKNOWN` for an undecided field without a hint, `MIXED_AUTHORITY` for unresolved branches,
//! `NO_ORACLE` for an oracle that does not reference a servable READ, `NOT_VERIFIED` for broken fixtures.

use crate::manifest::compile::{capability_digest, is_mode_split, pointer_property};
use crate::{admission, effects, pb};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Recorded on every L0 record.
pub const NON_CLAIM: &str = "runtime behaviour is not observed by the static lane";

fn finding(reason: pb::WithheldReason, hint: impl Into<String>) -> pb::Withheld {
    pb::Withheld {
        reason: reason as i32,
        hint: hint.into(),
        evidence_ids: Vec::new(),
    }
}

fn mcp_name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= crate::ids::MCP_NAME_MAX
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn cli(cap: &pb::Capability) -> Option<&pb::CliArgvBinding> {
    match cap.binding.as_ref()?.kind.as_ref()? {
        pb::binding::Kind::Cli(c) => Some(c),
        _ => None,
    }
}

fn check_schema(cap: &pb::Capability, out: &mut Vec<pb::Withheld>) -> BTreeSet<String> {
    let schema: Value = match serde_json::from_str(&cap.input_schema_json) {
        Ok(v) => v,
        Err(e) => {
            out.push(finding(
                pb::WithheldReason::SchemaInvalid,
                format!("input schema is not JSON: {e}"),
            ));
            return BTreeSet::new();
        }
    };
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            "input schema root must be {\"type\": \"object\"}",
        ));
    }
    if schema.get("additionalProperties") != Some(&Value::Bool(false)) {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            "input schema is open: root additionalProperties must be false",
        ));
    }
    if let Err(e) = jsonschema::validator_for(&schema) {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            format!("input schema does not compile: {e}"),
        ));
    }
    let props: BTreeSet<String> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for r in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
        if let Some(r) = r.as_str().filter(|r| !props.contains(*r)) {
            out.push(finding(
                pb::WithheldReason::SchemaInvalid,
                format!("required property {r:?} is not defined"),
            ));
        }
    }
    props
}

fn check_pointer(what: &str, p: &str, props: &BTreeSet<String>, out: &mut Vec<pb::Withheld>) {
    if !pointer_property(p).is_some_and(|n| props.contains(&n)) {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            format!("{what} pointer {p:?} does not resolve into the input schema"),
        ));
    }
}

/// Every fixture reachable from `start`; reports unknown ids and include cycles.
fn check_fixture(
    start: &str,
    fixtures: &BTreeMap<&str, &pb::Fixture>,
    caps: &BTreeSet<&str>,
    out: &mut Vec<pb::Withheld>,
) {
    fn visit<'a>(
        id: &'a str,
        fixtures: &BTreeMap<&'a str, &'a pb::Fixture>,
        caps: &BTreeSet<&str>,
        stack: &mut Vec<&'a str>,
        done: &mut BTreeSet<&'a str>,
        out: &mut Vec<pb::Withheld>,
    ) {
        if let Some(pos) = stack.iter().position(|s| *s == id) {
            let mut cycle: Vec<&str> = stack[pos..].to_vec();
            cycle.push(id);
            out.push(finding(
                pb::WithheldReason::NotVerified,
                format!("fixture include cycle: {}", cycle.join(" -> ")),
            ));
            return;
        }
        if done.contains(id) {
            return;
        }
        let Some(f) = fixtures.get(id) else {
            out.push(finding(
                pb::WithheldReason::NotVerified,
                format!("fixture {id:?} is not defined"),
            ));
            return;
        };
        if !f.ready_capability_id.is_empty() && !caps.contains(f.ready_capability_id.as_str()) {
            out.push(finding(
                pb::WithheldReason::NotVerified,
                format!(
                    "fixture {id:?}: ready capability {:?} is not in the lock",
                    f.ready_capability_id
                ),
            ));
        }
        stack.push(id);
        for step in &f.steps {
            match &step.step {
                Some(pb::fixture_step::Step::IncludeFixture(inc)) => {
                    let inc: &'a str = inc.as_str();
                    visit(inc, fixtures, caps, stack, done, out);
                }
                Some(pb::fixture_step::Step::Capability(c)) if !caps.contains(c.capability_id.as_str()) => {
                    out.push(finding(
                        pb::WithheldReason::NotVerified,
                        format!(
                            "fixture {id:?}: step capability {:?} is not in the lock",
                            c.capability_id
                        ),
                    ));
                }
                Some(pb::fixture_step::Step::Argv(a)) if a.argv.is_empty() => {
                    out.push(finding(
                        pb::WithheldReason::NotVerified,
                        format!("fixture {id:?}: empty argv step"),
                    ));
                }
                None => out.push(finding(
                    pb::WithheldReason::NotVerified,
                    format!("fixture {id:?}: empty step"),
                )),
                _ => {}
            }
        }
        stack.pop();
        done.insert(id);
    }
    visit(start, fixtures, caps, &mut Vec::new(), &mut BTreeSet::new(), out);
}

fn check_oracle_ref(id: &str, lock: &pb::CapabilityLock, out: &mut Vec<pb::Withheld>) {
    match lock.capabilities.iter().find(|c| c.id == id) {
        None => out.push(finding(
            pb::WithheldReason::NoOracle,
            format!("oracle capability {id:?} is not in the lock"),
        )),
        Some(c) => {
            let servable = c
                .admission
                .as_ref()
                .is_some_and(|a| a.state == pb::AdmissionState::Servable as i32);
            if c.level != pb::EffectLevel::Read as i32 || !servable {
                out.push(finding(
                    pb::WithheldReason::NoOracle,
                    format!("oracle capability {id:?} is not a servable READ (verify it first)"),
                ));
            }
        }
    }
}

fn json_ok(what: &str, s: &str, out: &mut Vec<pb::Withheld>) {
    if !s.is_empty() && serde_json::from_str::<Value>(s).is_err() {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            format!("{what} is not JSON"),
        ));
    }
}

/// The L0 findings for one capability (empty = PASSED).
pub fn findings(lock: &pb::CapabilityLock, cap: &pb::Capability) -> Vec<pb::Withheld> {
    let mut out = Vec::new();
    let target = lock.target.clone().unwrap_or_default();

    // Closed schema and pointer resolution.
    let props = check_schema(cap, &mut out);
    for s in &cap.scopes {
        check_pointer("scope", &s.pointer, &props, &mut out);
    }
    for b in &cap.branches {
        check_pointer("branch", &b.pointer, &props, &mut out);
        if effects::branch_unresolved(b) {
            out.push(finding(
                pb::WithheldReason::MixedAuthority,
                format!("branch on {} is unresolved (mixed authority)", b.pointer),
            ));
        }
    }
    let withheld = cap
        .admission
        .as_ref()
        .map(|a| a.withheld.as_slice())
        .unwrap_or_default();
    if let Some(w) = withheld
        .iter()
        .find(|w| w.reason == pb::WithheldReason::MixedAuthority as i32)
    {
        out.push(finding(
            pb::WithheldReason::MixedAuthority,
            format!("unresolved mixed authority: {}", w.hint),
        ));
    }

    // Binding resolves.
    match cap.binding.as_ref().and_then(|b| b.kind.as_ref()) {
        None => out.push(finding(pb::WithheldReason::SchemaInvalid, "capability has no binding")),
        Some(pb::binding::Kind::Cli(c)) => {
            for a in &c.args {
                check_pointer("binding arg", &a.pointer, &props, &mut out);
                if a.style == pb::ArgStyle::Unspecified as i32 {
                    out.push(finding(
                        pb::WithheldReason::SchemaInvalid,
                        format!("binding arg {} has no style", a.pointer),
                    ));
                }
            }
            if !c.cwd_pointer.is_empty() {
                check_pointer("cwd", &c.cwd_pointer, &props, &mut out);
            }
            if !c.state_handle_pointer.is_empty() {
                check_pointer("state handle", &c.state_handle_pointer, &props, &mut out);
            }
            let files = target.identity.as_ref().map(|i| i.files.as_slice()).unwrap_or_default();
            if c.executable_role.is_empty() || (!files.is_empty() && !files.iter().any(|f| f.role == c.executable_role))
            {
                out.push(finding(
                    pb::WithheldReason::SchemaInvalid,
                    format!(
                        "binding executable role {:?} is not a pinned identity file",
                        c.executable_role
                    ),
                ));
            }
            if c.kill_scope != pb::KillScope::OwnedGroup as i32 {
                out.push(finding(
                    pb::WithheldReason::SchemaInvalid,
                    "cli_argv bindings must use KILL_SCOPE_OWNED_GROUP",
                ));
            }
            if is_mode_split(cap) {
                let harvested = lock
                    .capabilities
                    .iter()
                    .filter(|o| !is_mode_split(o))
                    .filter_map(cli)
                    .any(|o| o.command_path == c.command_path);
                if !harvested {
                    out.push(finding(
                        pb::WithheldReason::SchemaInvalid,
                        format!(
                            "mode split command_path {:?} matches no harvested native path",
                            c.command_path.join(" ")
                        ),
                    ));
                }
            }
        }
        Some(_) => out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            "only cli_argv bindings are supported in slice 1",
        )),
    }

    // Oracles reference a servable READ.
    for o in &cap.oracles {
        match &o.kind {
            Some(pb::oracle::Kind::ReadBack(rb)) => {
                check_oracle_ref(&rb.capability_id, lock, &mut out);
                json_ok("oracle args template", &rb.args_template_json, &mut out);
            }
            Some(pb::oracle::Kind::Negative(ng)) => {
                check_oracle_ref(&ng.capability_id, lock, &mut out);
                json_ok("oracle args template", &ng.args_template_json, &mut out);
                json_ok("oracle bogus input", &ng.bogus_input_json, &mut out);
            }
            Some(pb::oracle::Kind::File(f)) => check_pointer("file oracle", &f.path_pointer, &props, &mut out),
            None => out.push(finding(pb::WithheldReason::NoOracle, "empty oracle")),
        }
    }

    // Examples and fixtures.
    let fixtures: BTreeMap<&str, &pb::Fixture> = target.fixtures.iter().map(|f| (f.id.as_str(), f)).collect();
    let cap_ids: BTreeSet<&str> = lock.capabilities.iter().map(|c| c.id.as_str()).collect();
    let mut used = BTreeSet::new();
    for e in &cap.examples {
        if !serde_json::from_str::<Value>(&e.input_json).is_ok_and(|v| v.is_object()) {
            out.push(finding(
                pb::WithheldReason::SchemaInvalid,
                "example input is not a JSON object",
            ));
        }
        if !e.fixture_id.is_empty() {
            used.insert(e.fixture_id.clone());
        }
    }
    for f in &used {
        check_fixture(f, &fixtures, &cap_ids, &mut out);
    }

    // Every target field decided or withheld with a hint.
    let stored = cap.effects.unwrap_or_default();
    let hints: Vec<&str> = withheld
        .iter()
        .filter(|w| w.reason == pb::WithheldReason::EffectsUnknown as i32)
        .map(|w| w.hint.as_str())
        .collect();
    for f in effects::unknown_target_fields(&stored) {
        if !hints.iter().any(|h| h.contains(f)) {
            out.push(finding(
                pb::WithheldReason::EffectsUnknown,
                format!("{f} is UNKNOWN and no withheld hint names it"),
            ));
        }
    }

    // Derived fields recompute.
    let e = effects::combine(&cap.signals, admission::has_write_path_param(cap));
    let binding = cap.binding.clone().unwrap_or_default();
    if cap.effects.as_ref() != Some(&e)
        || cap.level != effects::level(&e) as i32
        || cap.tier != effects::tier(&e, &binding) as i32
        || cap.hints.as_ref() != Some(&effects::hints(&e))
    {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            "derived effects/level/tier/hints do not recompute",
        ));
    }
    match capability_digest(cap) {
        Ok(d) if d == cap.capability_digest => {}
        _ => out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            "capability_digest does not recompute",
        )),
    }

    // Names unique.
    let name = cap
        .projections
        .as_ref()
        .map(|p| p.mcp_name.as_str())
        .unwrap_or_default();
    if !mcp_name_ok(name) {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            format!("MCP name {name:?} is invalid"),
        ));
    } else if lock
        .capabilities
        .iter()
        .filter(|c| c.projections.as_ref().is_some_and(|p| p.mcp_name == name))
        .count()
        > 1
    {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            format!("duplicate MCP name {name:?}"),
        ));
    }
    if lock.capabilities.iter().filter(|c| c.id == cap.id).count() > 1 {
        out.push(finding(
            pb::WithheldReason::SchemaInvalid,
            format!("duplicate capability id {:?}", cap.id),
        ));
    }

    out.sort_by(|a, b| (a.reason, &a.hint).cmp(&(b.reason, &b.hint)));
    out.dedup();
    out
}

pub fn run_l0(lock: &pb::CapabilityLock, cap: &pb::Capability, engine_version: &str) -> pb::VerificationRecord {
    let findings = findings(lock, cap);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut rec = pb::VerificationRecord {
        record_id: String::new(),
        capability_id: cap.id.clone(),
        capability_digest: cap.capability_digest.clone(),
        lane: pb::Lane::Static as i32,
        status: if findings.is_empty() {
            pb::LaneStatus::Passed
        } else {
            pb::LaneStatus::Failed
        } as i32,
        skip_reason: String::new(),
        lock_digest: lock.lock_digest.clone(),
        identity_digest: lock
            .target
            .as_ref()
            .and_then(|t| t.identity.as_ref())
            .map(|i| i.identity_digest.clone())
            .unwrap_or_default(),
        isolation: pb::Isolation::None as i32,
        host: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        engine_version: engine_version.to_string(),
        non_claims: vec![NON_CLAIM.to_string()],
        findings,
        ran_at: Some(pbjson_types::Timestamp {
            seconds: i64::try_from(now.as_secs()).unwrap_or(i64::MAX),
            nanos: i32::try_from(now.subsec_nanos()).unwrap_or(0),
        }),
        ..Default::default()
    };
    let id = crate::digest::digest(&rec).unwrap_or_default();
    rec.record_id = format!("vr_{id}");
    rec
}
