//! The `cli_argv` binding: pure argv construction, output normalization, and dispatch in a private root.
//!
//! Argv hardening (architecture §14):
//! - options are always one `--opt=value` argument unless the mapping is marked `no_equals_form` (proved by L1),
//!   in which case the value is a separate argument and values starting with `-` are refused;
//! - positionals come after `--`;
//! - argv is built from the validated input object only: unknown keys are refused, NUL bytes are refused.
//!
//! Run layout: `plan()` lays the run out as `<ctx.scratch_parent>/{cwd,home,tmp}` without touching the
//! filesystem; `invoke()` creates a fresh [`PrivateRoot`] under `ctx.scratch_parent` and plans against it. The
//! lanes call `plan()` with `scratch_parent` set to an existing root's base, so the planned layout *is* that root.
//!
//! Binding `env` values may use the placeholders `{home}`, `{tmp}`, `{cwd}` and `{root}`, expanded to the run's
//! private directories (e.g. `GH_CONFIG_DIR: "{home}/.config/gh"`). `HOME`, `TMPDIR` and `AA_RUN_ID` cannot be
//! overridden by a binding.

use crate::root::PrivateRoot;
use crate::snapshot::{diff, snapshot_or_empty};
use crate::supervisor::DEFAULT_PATH;
use aa_core::{
    AaError, AaResult,
    api::{Binding, BindingKind, BindingOutput, ConfinementSpec, InvokeCtx, RunOutput, RunSpec},
    pb,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

/// Used when neither the binding nor the context sets a deadline.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);
/// Used when the binding sets no `max_output_bytes`.
pub const DEFAULT_MAX_STDOUT: u64 = 8 * 1024 * 1024;
/// Bound of the stderr excerpt kept in reports.
pub const STDERR_EXCERPT: usize = 4096;

const CONFIG_REMEDIATION: &str = "fix the capability binding in the overlay, then recompile: aa discover <target>";

fn config(msg: impl Into<String>) -> AaError {
    AaError::new(pb::ErrorCode::ConfigurationInvalid, msg, CONFIG_REMEDIATION)
}

/// The capability's `cli_argv` binding.
pub(crate) fn cli_binding(cap: &pb::Capability) -> AaResult<&pb::CliArgvBinding> {
    match cap.binding.as_ref().and_then(|b| b.kind.as_ref()) {
        Some(pb::binding::Kind::Cli(c)) => Ok(c),
        _ => Err(config(format!("capability {} has no cli_argv binding", cap.id))),
    }
}

/// The pinned file the binding executes (`executable_role`, default `"executable"`).
pub(crate) fn executable(b: &pb::CliArgvBinding, identity: &pb::TargetIdentity) -> AaResult<String> {
    let role = if b.executable_role.is_empty() {
        "executable"
    } else {
        b.executable_role.as_str()
    };
    let f = identity
        .files
        .iter()
        .find(|f| f.role == role)
        .ok_or_else(|| config(format!("no pinned identity file with role {role:?}")))?;
    if !Path::new(&f.realpath).is_absolute() {
        return Err(config(format!("pinned {role} path {:?} is not absolute", f.realpath)));
    }
    Ok(f.realpath.clone())
}

/// First segment of a JSON pointer, unescaped (`/a~1b/c` -> `a/b`).
pub(crate) fn top_key(pointer: &str) -> Option<String> {
    let rest = pointer.strip_prefix('/')?;
    let seg = rest.split('/').next().unwrap_or("");
    Some(seg.replace("~1", "/").replace("~0", "~"))
}

fn lookup<'v>(input: &'v Value, pointer: &str) -> Option<&'v Value> {
    if pointer.is_empty() {
        return None;
    }
    input.pointer(pointer).filter(|v| !v.is_null())
}

fn scalar(v: &Value, what: &str) -> AaResult<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(AaError::invalid(format!("{what} must be a string, number or boolean"))),
    }
}

fn check_flag(m: &pb::ArgMapping) -> AaResult<()> {
    if !m.flag.starts_with('-') || m.flag.contains('=') || m.flag.contains('\0') {
        return Err(config(format!(
            "argument {} has an invalid flag {:?}",
            m.pointer, m.flag
        )));
    }
    Ok(())
}

fn push_option(argv: &mut Vec<String>, m: &pb::ArgMapping, value: String) -> AaResult<()> {
    check_flag(m)?;
    if m.no_equals_form {
        if value.starts_with('-') {
            return Err(AaError::invalid(format!(
                "the value for {} starts with '-', which this option cannot take safely (no --opt=value form)",
                m.pointer
            )));
        }
        argv.push(m.flag.clone());
        argv.push(value);
    } else {
        argv.push(format!("{}={value}", m.flag));
    }
    Ok(())
}

fn style(m: &pb::ArgMapping) -> pb::ArgStyle {
    pb::ArgStyle::try_from(m.style).unwrap_or(pb::ArgStyle::Unspecified)
}

/// Number of argv elements before the mapped options (exe, interpreter/global args, state handle, dry-run,
/// command path, fixed args) for this binding and context. Used by L1 to place its probes.
pub(crate) fn prefix_len(cap: &pb::Capability, b: &pb::CliArgvBinding, argv: &[String], ctx: &InvokeCtx) -> usize {
    let mut n = 1 + b.interpreter_args.len() + b.global_args.len();
    if !b.state_handle_flag.is_empty() {
        let prefix = format!("{}=", b.state_handle_flag);
        if argv.get(n).is_some_and(|a| a.starts_with(&prefix)) {
            n += 1;
        }
    }
    if ctx.dry_run {
        n += cap.dry_run.as_ref().map_or(0, |d| d.argv.len());
    }
    (n + b.command_path.len() + b.fixed_args.len()).min(argv.len())
}

/// PURE. argv is:
///
/// ```text
/// [exe] + interpreter_args + global_args + [state_handle_flag=path] + dry_run.argv + command_path
///   + fixed_args + options("--flag=value", "--flag", repeated) + ["--"] + positionals
/// ```
///
/// Refuses: unknown input keys (schema already closed), values starting with '-' when no_equals_form, NUL bytes.
pub fn build_argv(cap: &pb::Capability, input: &serde_json::Value, ctx: &InvokeCtx) -> AaResult<Vec<String>> {
    let b = cli_binding(cap)?;
    let empty = Map::new();
    let obj = match input {
        Value::Object(m) => m,
        Value::Null => &empty,
        _ => return Err(AaError::invalid("the input must be a JSON object")),
    };
    let mut known = BTreeSet::new();
    for p in b
        .args
        .iter()
        .map(|m| m.pointer.as_str())
        .chain([b.cwd_pointer.as_str(), b.state_handle_pointer.as_str()])
    {
        if let Some(k) = top_key(p) {
            known.insert(k);
        }
    }
    if let Some(k) = obj.keys().find(|k| !known.contains(*k)) {
        return Err(AaError::invalid(format!("unknown input key {k:?}")));
    }

    let mut argv = vec![executable(b, &ctx.identity)?];
    argv.extend(b.interpreter_args.iter().cloned());
    argv.extend(b.global_args.iter().cloned());

    if !b.state_handle_flag.is_empty() {
        let path = match &ctx.state_handle {
            Some(p) => Some(p.to_string_lossy().into_owned()),
            None if !b.state_handle_engine_supplied => lookup(input, &b.state_handle_pointer)
                .map(|v| scalar(v, &b.state_handle_pointer))
                .transpose()?,
            None => None,
        };
        match path {
            Some(p) => argv.push(format!("{}={p}", b.state_handle_flag)),
            None if b.state_handle_engine_supplied => {
                return Err(AaError::internal(
                    "the binding needs an engine-supplied state handle, but none was provided",
                ));
            }
            None => {}
        }
    }

    if ctx.dry_run {
        match cap.dry_run.as_ref().filter(|d| d.supported) {
            Some(d) => argv.extend(d.argv.iter().cloned()),
            None => {
                return Err(AaError::invalid(format!(
                    "capability {} does not support dry_run",
                    cap.id
                )));
            }
        }
    }

    argv.extend(b.command_path.iter().cloned());
    argv.extend(b.fixed_args.iter().cloned());

    let mut positionals: Vec<(u32, usize, Vec<String>)> = Vec::new();
    for (i, m) in b.args.iter().enumerate() {
        let Some(v) = lookup(input, &m.pointer) else { continue };
        match style(m) {
            pb::ArgStyle::Option => push_option(&mut argv, m, scalar(v, &m.pointer)?)?,
            pb::ArgStyle::RepeatedOption => match v {
                Value::Array(items) => {
                    for item in items {
                        push_option(&mut argv, m, scalar(item, &m.pointer)?)?;
                    }
                }
                other => push_option(&mut argv, m, scalar(other, &m.pointer)?)?,
            },
            pb::ArgStyle::Flag => match v {
                Value::Bool(true) => {
                    check_flag(m)?;
                    argv.push(m.flag.clone());
                }
                Value::Bool(false) => {}
                _ => return Err(AaError::invalid(format!("{} must be a boolean", m.pointer))),
            },
            pb::ArgStyle::NegatableFlag => {
                check_flag(m)?;
                match v {
                    Value::Bool(true) => argv.push(m.flag.clone()),
                    Value::Bool(false) => {
                        let rest = m
                            .flag
                            .strip_prefix("--")
                            .ok_or_else(|| config(format!("negatable flag {:?} must be a long option", m.flag)))?;
                        argv.push(format!("--no-{rest}"));
                    }
                    _ => return Err(AaError::invalid(format!("{} must be a boolean", m.pointer))),
                }
            }
            pb::ArgStyle::Positional => {
                let values = match v {
                    Value::Array(items) => items
                        .iter()
                        .map(|x| scalar(x, &m.pointer))
                        .collect::<AaResult<Vec<_>>>()?,
                    other => vec![scalar(other, &m.pointer)?],
                };
                positionals.push((m.position, i, values));
            }
            pb::ArgStyle::Unspecified => {
                return Err(config(format!("argument {} has no style", m.pointer)));
            }
        }
    }
    positionals.sort_by_key(|(pos, i, _)| (*pos, *i));
    let values: Vec<String> = positionals.into_iter().flat_map(|(_, _, v)| v).collect();
    if !values.is_empty() {
        argv.push("--".to_string());
        argv.extend(values);
    }

    if argv.iter().any(|a| a.contains('\0')) {
        return Err(AaError::invalid("the input contains a NUL byte"));
    }
    Ok(argv)
}

fn text(s: impl Into<String>, source: &str) -> pb::TargetText {
    pb::TargetText {
        text: s.into(),
        origin: pb::ContentOrigin::Target as i32,
        source: source.to_string(),
    }
}

fn process_report(out: &RunOutput) -> pb::ProcessReport {
    pb::ProcessReport {
        exit_code: out.exit_code.unwrap_or(-1),
        signal: out.signal.unwrap_or(0),
        stdout_bytes: out.stdout_total,
        stderr_bytes: out.stderr_total,
        stderr_excerpt: (!out.stderr_excerpt.is_empty()).then(|| text(out.stderr_excerpt.clone(), "stderr")),
        output_truncated: out.stdout_truncated,
        wall_ms: u32::try_from(out.wall.as_millis()).unwrap_or(u32::MAX),
        cleanup: out.cleanup as i32,
        confinement_denials: out.confinement_denials.clone(),
        unowned_process_notes: Vec::new(),
    }
}

fn failed(process: pb::ProcessReport, outcome: pb::Outcome, error: AaError) -> BindingOutput {
    BindingOutput {
        outcome,
        output_json: None,
        error: Some(error),
        process,
        observed_files: Vec::new(),
    }
}

fn details(v: Value) -> String {
    serde_json::to_string(&v).unwrap_or_default()
}

/// Maps a finished run to an outcome, a normalized output and a typed error.
pub fn normalize_output(binding: &pb::CliArgvBinding, out: &aa_core::api::RunOutput) -> BindingOutput {
    let process = process_report(out);
    if out.timed_out {
        let e = AaError::new(
            pb::ErrorCode::Timeout,
            "the target did not finish before the deadline; its process group was terminated",
            "check what happened before retrying: aa audit --last 1",
        )
        .with_recovery(
            pb::RecoveryKind::CheckStatusBeforeRetry,
            "RuntimeService.GetActionStatus",
        );
        return failed(process, pb::Outcome::Unknown, e);
    }
    if out.stdout_truncated {
        let mut e = AaError::new(
            pb::ErrorCode::ResourceConstraint,
            format!(
                "the target wrote {} bytes to stdout, above the {}-byte limit; the overflow was discarded",
                out.stdout_total,
                out.stdout.len()
            ),
            "narrow the request, or raise max_output_bytes in the overlay: aa discover <target>",
        );
        e.details_json = details(serde_json::json!({"stdout_total": out.stdout_total, "limit": out.stdout.len()}));
        return failed(process, pb::Outcome::Failed, e);
    }
    if out.cleanup == pb::CleanupStatus::Incomplete {
        let e = AaError::new(
            pb::ErrorCode::CleanupIncomplete,
            "processes of the owned group survived termination",
            "check the engine status before dispatching to this target again: aa status",
        )
        .with_recovery(pb::RecoveryKind::RetryCleanup, "RuntimeService.GetActionStatus");
        return failed(process, pb::Outcome::Unknown, e);
    }
    let convention = pb::ErrorConvention::try_from(binding.errors).unwrap_or(pb::ErrorConvention::Unspecified);
    let exit = out.exit_code;
    if exit != Some(0) {
        let stderr = text(out.stderr_excerpt.clone(), "stderr");
        let mut e = AaError::new(
            pb::ErrorCode::UpstreamError,
            match (exit, out.signal) {
                (Some(c), _) => format!("the target exited with code {c}"),
                (None, Some(s)) => format!("the target was terminated by signal {s}"),
                (None, None) => "the target ended without an exit status".to_string(),
            },
            "inspect the upstream error and the input: aa describe <id>",
        );
        e.upstream = (!out.stderr_excerpt.is_empty()).then_some(stderr);
        e.details_json = details(serde_json::json!({"exit_code": exit, "signal": out.signal}));
        if convention == pb::ErrorConvention::CliAnythingJson
            && let Ok(Value::Object(m)) = serde_json::from_slice::<Value>(&out.stdout)
            && let Some(msg) = m.get("error").and_then(Value::as_str)
        {
            let ty = m.get("type").and_then(Value::as_str).unwrap_or("");
            e.upstream = Some(text(msg, if ty.is_empty() { "cli-anything" } else { ty }));
            e.details_json = details(serde_json::json!({"exit_code": exit, "type": ty}));
        }
        return failed(process, pb::Outcome::Failed, e);
    }
    let mode = pb::OutputMode::try_from(binding.output).unwrap_or(pb::OutputMode::Unspecified);
    let output_json = match mode {
        pb::OutputMode::JsonStdout => match serde_json::from_slice::<Value>(&out.stdout) {
            Ok(v) => Some(v),
            Err(err) => {
                let mut e = AaError::new(
                    pb::ErrorCode::UnreadableOutput,
                    format!("the target's stdout is not valid JSON ({err})"),
                    "check the binding's output mode in the overlay: aa describe <id>",
                );
                let head: String = String::from_utf8_lossy(&out.stdout).chars().take(512).collect();
                e.upstream = Some(text(head, "stdout"));
                return failed(process, pb::Outcome::Failed, e);
            }
        },
        pb::OutputMode::TextStdout | pb::OutputMode::Unspecified => {
            let s = String::from_utf8_lossy(&out.stdout).into_owned();
            let lines: Vec<Value> = s.lines().map(|l| Value::String(l.to_string())).collect();
            Some(serde_json::json!({"text": s, "lines": lines}))
        }
        pb::OutputMode::None => None,
    };
    BindingOutput {
        outcome: pb::Outcome::Completed,
        output_json,
        error: None,
        process,
        observed_files: Vec::new(),
    }
}

fn expand(v: &str, base: &Path, home: &Path, cwd: &Path, tmp: &Path) -> String {
    v.replace("{home}", &home.to_string_lossy())
        .replace("{tmp}", &tmp.to_string_lossy())
        .replace("{cwd}", &cwd.to_string_lossy())
        .replace("{root}", &base.to_string_lossy())
}

/// `plan()` against an explicit root layout `base/{home,cwd,tmp}`.
pub(crate) fn plan_at(cap: &pb::Capability, input: &Value, ctx: &InvokeCtx, base: &Path) -> AaResult<RunSpec> {
    let b = cli_binding(cap)?;
    let argv = build_argv(cap, input, ctx)?;
    let (home, default_cwd, tmp) = PrivateRoot::layout(base);
    let cwd = if b.cwd_pointer.is_empty() {
        default_cwd.clone()
    } else {
        let v = lookup(input, &b.cwd_pointer)
            .ok_or_else(|| AaError::invalid(format!("missing the working-directory input {}", b.cwd_pointer)))?;
        let p = PathBuf::from(scalar(v, &b.cwd_pointer)?);
        if !p.is_absolute() {
            return Err(AaError::invalid(format!("{} must be an absolute path", b.cwd_pointer)));
        }
        p
    };
    let exe_dir = Path::new(&argv[0]).parent().map(|p| p.to_string_lossy().into_owned());
    let path = match exe_dir {
        Some(d) if !d.is_empty() => format!("{d}:{DEFAULT_PATH}"),
        _ => DEFAULT_PATH.to_string(),
    };
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), path);
    env.insert("LANG".to_string(), "C.UTF-8".to_string());
    let mut extra: Vec<(&String, &String)> = b.env.iter().collect();
    extra.sort();
    for (k, v) in extra {
        if matches!(k.as_str(), "HOME" | "TMPDIR" | "AA_RUN_ID") {
            tracing::warn!(key = %k, "binding env may not override a supervisor-owned variable; ignored");
            continue;
        }
        env.insert(k.clone(), expand(v, base, &home, &default_cwd, &tmp));
    }
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    env.insert("TMPDIR".to_string(), tmp.to_string_lossy().into_owned());

    let binding_deadline = (b.deadline_ms > 0).then(|| Duration::from_millis(u64::from(b.deadline_ms)));
    let ctx_deadline = (!ctx.deadline.is_zero()).then_some(ctx.deadline);
    let deadline = match (binding_deadline, ctx_deadline) {
        (Some(a), Some(c)) => a.min(c),
        (Some(a), None) => a,
        (None, Some(c)) => c,
        (None, None) => DEFAULT_DEADLINE,
    };
    let confinement = ctx.confinement.as_ref().map(|c| {
        let mut c: ConfinementSpec = c.clone();
        if !c.writable.iter().any(|w| w == base) {
            c.writable.push(base.to_path_buf());
        }
        // Binding-intrinsic execution: the pinned files are exactly the exec allow-list (architecture §6.2).
        for f in &ctx.identity.files {
            let p = PathBuf::from(&f.realpath);
            if p.is_absolute() && !c.exec_allow.contains(&p) {
                c.exec_allow.push(p);
            }
        }
        c
    });
    Ok(RunSpec {
        argv,
        cwd,
        env,
        stdin: None,
        deadline,
        max_stdout: if b.max_output_bytes > 0 {
            b.max_output_bytes
        } else {
            DEFAULT_MAX_STDOUT
        },
        max_stderr_excerpt: STDERR_EXCERPT,
        confinement,
        run_id: if ctx.action_id.is_empty() {
            aa_core::ids::new_action_id()
        } else {
            ctx.action_id.clone()
        },
    })
}

pub struct CliArgvBinding {
    runner: std::sync::Arc<dyn aa_core::api::ProcessRunner>,
}

impl CliArgvBinding {
    pub fn new(runner: std::sync::Arc<dyn aa_core::api::ProcessRunner>) -> Self {
        Self { runner }
    }
}

/// plan() = build_argv + RunSpec; invoke() runs in a fresh PrivateRoot (cwd = cwd_pointer path if set, else
/// root.cwd), HOME/TMPDIR private, confinement from ctx.
#[async_trait::async_trait]
impl Binding for CliArgvBinding {
    fn kind(&self) -> BindingKind {
        BindingKind::CliArgv
    }

    fn plan(&self, cap: &pb::Capability, input: &serde_json::Value, ctx: &InvokeCtx) -> AaResult<RunSpec> {
        plan_at(cap, input, ctx, &ctx.scratch_parent)
    }

    async fn invoke(
        &self,
        cap: &pb::Capability,
        input: &serde_json::Value,
        ctx: &InvokeCtx,
    ) -> AaResult<BindingOutput> {
        let b = cli_binding(cap)?;
        let root = PrivateRoot::create(&ctx.scratch_parent)?;
        let mut spec = plan_at(cap, input, ctx, &root.base)?;
        if !b.cwd_pointer.is_empty() {
            spec.cwd = std::fs::canonicalize(&spec.cwd).map_err(|e| {
                AaError::invalid(format!(
                    "the working directory {} is not accessible: {e}",
                    spec.cwd.display()
                ))
            })?;
        }
        let watched: Vec<(PathBuf, &str)> = [
            Some((root.home.clone(), "private:home")),
            Some((root.cwd.clone(), "private:cwd")),
            ctx.state_handle.clone().map(|p| (p, "scope:state_handle")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let before: Vec<_> = watched.iter().map(|(p, _)| snapshot_or_empty(p, &[])).collect();
        let out = self.runner.run(spec).await?;
        let mut observed = Vec::new();
        for ((p, label), b) in watched.iter().zip(before.iter()) {
            observed.extend(diff(b, &snapshot_or_empty(p, &[]), label));
        }
        let mut result = normalize_output(cli_binding(cap)?, &out);
        result.observed_files = observed;
        drop(root);
        Ok(result)
    }
}
