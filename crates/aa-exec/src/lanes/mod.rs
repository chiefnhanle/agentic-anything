//! Effectful verification lanes: L1 (parser) and L3 (sandbox diff), plus fixture building.
//!
//! Conventions shared by the lanes:
//! - Every run happens in a fresh [`PrivateRoot`]. Fixtures are built in `root.cwd`, and example inputs refer
//!   to it with `{"$root": "/rel/path"}`, which resolves to `<root.cwd>/rel/path`.
//! - An engine-supplied state handle (CLI-Anything `--project`) is `<root.base>/state/project.json`; the
//!   `state/` directory is watched as `scope:state_handle`.
//! - Runs go through `binding.plan()` with `scratch_parent = root.base`, so the planned HOME/TMPDIR/cwd are the
//!   root's own directories, and then through `ctx.runner`.
//! - When the runner can confine (`max_isolation() == CONFINED`), every run is confined: network denied, writes
//!   limited to the root, exec limited to the pinned identity files.

mod fixture;
mod parser;
mod sandbox_diff;

pub use fixture::build_fixture;
pub use parser::run_parser_lane;
pub use sandbox_diff::run_sandbox_diff_lane;

use crate::{
    root::PrivateRoot,
    snapshot::{TreeSnapshot, diff, snapshot_or_empty},
};
use aa_core::{
    AaError, AaResult,
    api::{ConfinementSpec, InvokeCtx, RunOutput},
    pb,
};
use serde_json::Value;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct LaneCtx<'a> {
    pub lock: &'a aa_core::pb::CapabilityLock,
    pub binding: &'a dyn aa_core::api::Binding,
    pub runner: &'a dyn aa_core::api::ProcessRunner,
    pub scratch_parent: &'a std::path::Path,
    pub engine_version: &'a str,
}

/// Non-claim present on every L3 record.
pub const NON_CLAIM_REMOTE: &str = "cloud/remote state not observed";
/// Non-claim added when a lane ran at `ISOLATION_PROCESS`.
pub const NON_CLAIM_PROCESS: &str = "network and out-of-root writes not observed";

pub(crate) fn host() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub(crate) fn now() -> pbjson_types::Timestamp {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    pbjson_types::Timestamp {
        seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(d.subsec_nanos()).unwrap_or(0),
    }
}

/// A record skeleton bound to the current lock and identity digests.
pub(crate) fn record(ctx: &LaneCtx<'_>, cap: &pb::Capability, lane: pb::Lane) -> pb::VerificationRecord {
    pb::VerificationRecord {
        capability_id: cap.id.clone(),
        capability_digest: cap.capability_digest.clone(),
        lane: lane as i32,
        status: pb::LaneStatus::NotRun as i32,
        lock_digest: ctx.lock.lock_digest.clone(),
        identity_digest: identity(ctx).identity_digest.clone(),
        isolation: pb::Isolation::NotProven as i32,
        host: host(),
        engine_version: ctx.engine_version.to_string(),
        ran_at: Some(now()),
        ..Default::default()
    }
}

/// Sets `record_id = "vr_" + sha256(JCS(record with record_id and signal sources blank))`, then points every
/// OBSERVED signal's `source` at it (`verify:<record_id>`).
pub(crate) fn finalize(mut rec: pb::VerificationRecord) -> pb::VerificationRecord {
    let id = aa_core::digest::self_digest(&rec, |r| {
        r.record_id.clear();
        for s in &mut r.effect_signals {
            s.source.clear();
        }
    })
    .unwrap_or_else(|_| aa_core::digest::sha256_hex(format!("{rec:?}").as_bytes()));
    rec.record_id = format!("vr_{id}");
    for s in &mut rec.effect_signals {
        s.source = format!("verify:{}", rec.record_id);
    }
    rec
}

pub(crate) fn finish(
    mut rec: pb::VerificationRecord,
    status: pb::LaneStatus,
    reason: impl Into<String>,
) -> pb::VerificationRecord {
    rec.status = status as i32;
    rec.skip_reason = reason.into();
    finalize(rec)
}

pub(crate) fn identity<'a>(ctx: &'a LaneCtx<'_>) -> &'a pb::TargetIdentity {
    static EMPTY: std::sync::OnceLock<pb::TargetIdentity> = std::sync::OnceLock::new();
    ctx.lock
        .target
        .as_ref()
        .and_then(|t| t.identity.as_ref())
        .unwrap_or_else(|| EMPTY.get_or_init(pb::TargetIdentity::default))
}

/// The lane's confinement for a run in `root`, or `None` when the runner cannot confine.
pub(crate) fn confinement(ctx: &LaneCtx<'_>, root: &PrivateRoot) -> Option<ConfinementSpec> {
    (ctx.runner.max_isolation() == pb::Isolation::Confined).then(|| ConfinementSpec {
        deny_network: true,
        writable: vec![root.base.clone()],
        exec_allow: identity(ctx)
            .files
            .iter()
            .map(|f| PathBuf::from(&f.realpath))
            .filter(|p| p.is_absolute())
            .collect(),
    })
}

/// The lane's engine-supplied state handle inside `root`.
pub(crate) fn state_handle(root: &PrivateRoot) -> PathBuf {
    root.base.join("state").join("project.json")
}

pub(crate) fn state_dir(root: &PrivateRoot) -> PathBuf {
    root.base.join("state")
}

/// A fresh root with its `state/` directory.
pub(crate) fn fresh_root(ctx: &LaneCtx<'_>) -> AaResult<PrivateRoot> {
    let root = PrivateRoot::create(ctx.scratch_parent)?;
    let st = state_dir(&root);
    std::fs::create_dir(&st)?;
    std::fs::set_permissions(&st, std::fs::Permissions::from_mode(0o700))?;
    Ok(root)
}

/// The InvokeCtx for a lane run of `cap` in `root`.
pub(crate) fn invoke_ctx(ctx: &LaneCtx<'_>, cap: &pb::Capability, root: &PrivateRoot) -> InvokeCtx {
    let engine_state = crate::cli_argv::cli_binding(cap)
        .ok()
        .is_some_and(|b| !b.state_handle_flag.is_empty() && b.state_handle_engine_supplied);
    InvokeCtx {
        action_id: aa_core::ids::new_action_id(),
        identity: identity(ctx).clone(),
        state_handle: engine_state.then(|| state_handle(root)),
        scratch_parent: root.base.clone(),
        deadline: std::time::Duration::ZERO,
        dry_run: false,
        confinement: confinement(ctx, root),
    }
}

/// A relative path that stays inside its base (no absolute paths, no `..`).
pub(crate) fn safe_rel(rel: &str) -> AaResult<PathBuf> {
    let p = Path::new(rel.trim_start_matches('/'));
    if rel.contains('\0')
        || p.components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(AaError::invalid(format!("path {rel:?} escapes the private root")));
    }
    Ok(p.to_path_buf())
}

/// Replaces every `{"$root": "/rel"}` object with the string `<root.cwd>/rel`.
pub(crate) fn resolve_root_refs(v: &Value, root: &PrivateRoot) -> AaResult<Value> {
    Ok(match v {
        Value::Object(m) if m.len() == 1 && m.contains_key("$root") => {
            let rel = m["$root"]
                .as_str()
                .ok_or_else(|| AaError::invalid("$root must be a string path"))?;
            let rel = safe_rel(rel)?;
            Value::String(root.cwd.join(rel).to_string_lossy().trim_end_matches('/').to_string())
        }
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| Ok((k.clone(), resolve_root_refs(v, root)?)))
                .collect::<AaResult<_>>()?,
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve_root_refs(x, root)).collect::<AaResult<_>>()?),
        other => other.clone(),
    })
}

pub(crate) fn parse_input(json: &str) -> AaResult<Value> {
    if json.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(json).map_err(|e| AaError::invalid(format!("example input is not JSON: {e}")))
}

/// The directories a lane watches in `root`, with their `ObservedFileEffect.root` labels.
pub(crate) fn watched(root: &PrivateRoot) -> Vec<(PathBuf, &'static str)> {
    vec![
        (root.home.clone(), "private:home"),
        (root.cwd.clone(), "private:cwd"),
        (state_dir(root), "scope:state_handle"),
    ]
}

pub(crate) fn snapshots(w: &[(PathBuf, &'static str)]) -> Vec<TreeSnapshot> {
    w.iter().map(|(p, _)| snapshot_or_empty(p, &[])).collect()
}

pub(crate) fn diffs(w: &[(PathBuf, &'static str)], before: &[TreeSnapshot]) -> Vec<pb::ObservedFileEffect> {
    w.iter()
        .zip(before)
        .flat_map(|((p, label), b)| diff(b, &snapshot_or_empty(p, &[]), label))
        .collect()
}

/// One lane run: fresh root, fixture, resolved input, plan (with optional extra args inserted before the
/// positional separator), snapshots around the run.
pub(crate) struct LaneRun {
    pub out: RunOutput,
    pub observed: Vec<pb::ObservedFileEffect>,
}

pub(crate) async fn run_example(
    ctx: &LaneCtx<'_>,
    cap: &pb::Capability,
    input: &Value,
    fixture_id: &str,
    extra: Option<&[String]>,
) -> AaResult<LaneRun> {
    let root = fresh_root(ctx)?;
    if !fixture_id.is_empty() {
        build_fixture(ctx, fixture_id, &root).await?;
    }
    let input = resolve_root_refs(input, &root)?;
    let ictx = invoke_ctx(ctx, cap, &root);
    let mut spec = ctx.binding.plan(cap, &input, &ictx)?;
    if let Some(extra) = extra {
        let b = crate::cli_argv::cli_binding(cap)?;
        let start = crate::cli_argv::prefix_len(cap, b, &spec.argv, &ictx);
        let at = spec.argv[start..]
            .iter()
            .position(|a| a == "--")
            .map_or(spec.argv.len(), |i| start + i);
        spec.argv.splice(at..at, extra.iter().cloned());
    }
    let w = watched(&root);
    let before = snapshots(&w);
    let out = ctx.runner.run(spec).await?;
    let observed = diffs(&w, &before);
    drop(root);
    Ok(LaneRun { out, observed })
}

/// Declared value of an effect field.
pub(crate) fn declared(e: Option<&pb::Effects>, field: &str) -> pb::Tri {
    let Some(e) = e else { return pb::Tri::Unknown };
    let v = match field {
        "mutates_target" => e.mutates_target,
        "writes_filesystem" => e.writes_filesystem,
        "launches_process" => e.launches_process,
        "accesses_network" => e.accesses_network,
        _ => return pb::Tri::Unknown,
    };
    pb::Tri::try_from(v).unwrap_or(pb::Tri::Unknown)
}

pub(crate) fn signal(field: &str, value: pb::Tri, isolation: pb::Isolation, rationale: String) -> pb::EffectSignal {
    pb::EffectSignal {
        field: field.to_string(),
        value: value as i32,
        confidence: pb::Confidence::Observed as i32,
        source: String::new(),
        rationale,
        evidence_ids: Vec::new(),
        isolation: isolation as i32,
    }
}

pub(crate) fn finding(reason: pb::WithheldReason, hint: String) -> pb::Withheld {
    pb::Withheld {
        reason: reason as i32,
        hint,
        evidence_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_refs_and_safe_paths() {
        let parent = tempfile::tempdir().unwrap();
        let root = PrivateRoot::create(parent.path()).unwrap();
        let v = serde_json::json!({"repo": {"$root": "/repo"}, "n": 1, "list": [{"$root": "a/b"}], "o": {"$root": "/x", "y": 1}});
        let r = resolve_root_refs(&v, &root).unwrap();
        assert_eq!(r["repo"], root.cwd.join("repo").to_string_lossy().as_ref());
        assert_eq!(r["list"][0], root.cwd.join("a/b").to_string_lossy().as_ref());
        assert_eq!(r["n"], 1);
        assert!(r["o"].is_object());
        assert!(resolve_root_refs(&serde_json::json!({"$root": "../escape"}), &root).is_err());
        assert!(resolve_root_refs(&serde_json::json!({"$root": 3}), &root).is_err());
        assert_eq!(
            resolve_root_refs(&serde_json::json!({"$root": "/"}), &root).unwrap(),
            root.cwd.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn record_ids_are_content_addressed_and_sources_point_at_them() {
        let mut rec = pb::VerificationRecord {
            capability_id: "x.y".into(),
            effect_signals: vec![signal(
                "writes_filesystem",
                pb::Tri::False,
                pb::Isolation::Confined,
                "r".into(),
            )],
            ..Default::default()
        };
        let a = finalize(rec.clone());
        assert!(a.record_id.starts_with("vr_"));
        assert_eq!(a.effect_signals[0].source, format!("verify:{}", a.record_id));
        assert_eq!(finalize(a.clone()).record_id, a.record_id);
        rec.capability_id = "x.z".into();
        assert_ne!(finalize(rec).record_id, a.record_id);
    }
}
