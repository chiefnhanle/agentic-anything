//! `click-info@1`: Click `to_info_dict()` via the aa-click-info/1 Python shim (ADR 0009).
//!
//! `harvest()` writes the shim into a private root and runs `<python> -I <root>/aa_click_info.py <entry_point>`
//! confined (network denied, writes only in the root), 20 s deadline, 4 MiB stdout cap. `candidates()` is pure
//! over the recorded bytes: every leaf command becomes a candidate (`repl` is marked interactive; the bare root
//! group is never a candidate).

use crate::{
    CLICK_INFO_SHIM,
    common::{self, CandidateParts, Scratch},
    schema::{self, CliAnythingGlobals, PathUse},
};
use aa_core::{
    AaError, AaResult,
    api::{Candidate, HarvestCtx, Harvester, RawHarvest},
    pb,
};
use serde_json::Value;
use std::time::Duration;

pub(crate) const PROTOCOL: &str = "aa-click-info/1";
const HARVESTER: &str = "click-info@1";
const DEADLINE: Duration = Duration::from_secs(20);
const MAX_STDOUT: u64 = 4 * 1024 * 1024;
/// Click's usage-error exit code (L1 expects it).
const CLICK_USAGE_EXIT: i32 = 2;
/// Leaf names that are interactive sessions, never one-shot capabilities.
const INTERACTIVE_LEAVES: &[&str] = &["repl", "interactive"];

/// id "click-info", v1, INTROSPECT.
pub struct ClickInfoHarvester;

/// Console-script name: `options.entry_point`, else the CLI-Anything convention `cli-anything-<name>`.
fn entry_point(target: &pb::Target, options: &Value) -> Option<String> {
    if let Some(ep) = common::opt_str(options, "entry_point") {
        return Some(ep.to_owned());
    }
    if target.kind == pb::TargetKind::CliAnythingHarness as i32 {
        return target
            .id
            .strip_prefix("cli-anything.")
            .map(|n| format!("cli-anything-{n}"));
    }
    None
}

#[async_trait::async_trait]
impl Harvester for ClickInfoHarvester {
    fn id(&self) -> &'static str {
        "click-info"
    }

    fn version(&self) -> u32 {
        1
    }

    fn authority(&self) -> pb::DiscoveryAuthority {
        pb::DiscoveryAuthority::Introspect
    }

    fn applies(&self, target: &pb::Target, options: &serde_json::Value) -> bool {
        common::identity_file(target, "interpreter").is_some() && entry_point(target, options).is_some()
    }

    async fn harvest(&self, ctx: HarvestCtx<'_>) -> AaResult<Vec<RawHarvest>> {
        common::require_authorization(ctx.target, HARVESTER)?;
        let python = common::require_file(ctx.target, "interpreter", HARVESTER)?;
        let ep = entry_point(ctx.target, ctx.options).ok_or_else(|| {
            AaError::new(
                pb::ErrorCode::ConfigurationInvalid,
                format!("{HARVESTER}: no entry point for target '{}'", ctx.target.id),
                "set harvest[].options.entry_point in target.aa.yaml, then: aa discover <target>",
            )
        })?;
        if ep.starts_with('-') || ep.chars().any(char::is_whitespace) {
            return Err(AaError::invalid(format!(
                "{HARVESTER}: invalid entry point name {ep:?}"
            )));
        }
        let scratch = Scratch::create(ctx.scratch_parent)?;
        let shim = scratch.base.join("aa_click_info.py");
        std::fs::write(&shim, CLICK_INFO_SHIM)?;
        let argv = vec![
            python.realpath.clone(),
            "-I".to_owned(),
            shim.display().to_string(),
            ep.clone(),
        ];
        let env = scratch.env(&[("PYTHONDONTWRITEBYTECODE", "1")]);
        let spec = scratch.spec(argv, env, DEADLINE, MAX_STDOUT, ctx.target, ctx.allow_network);
        let out = ctx.runner.run(spec).await?;
        common::check_run(HARVESTER, "the click-info shim", &out)?;
        let payload: Option<Value> = serde_json::from_slice(&out.stdout).ok();
        let error = payload.as_ref().and_then(|p| p.get("error")).and_then(Value::as_str);
        if out.exit_code != Some(0) || error.is_some() || payload.is_none() {
            let what = error.unwrap_or("no aa-click-info/1 document on stdout");
            let remediation = match what {
                "ambiguous_root" => {
                    "point harvest[].options.entry_point at a console script whose module has exactly one root \
                     click.Group, then: aa discover <target>"
                }
                "entry_point_not_found" | "click_missing" => {
                    "install the harness into the pinned interpreter's venv (operator step), then: aa discover <target>"
                }
                _ => "inspect the harvest record: aa audit --last 5",
            };
            let mut err = AaError::new(
                pb::ErrorCode::UpstreamError,
                format!(
                    "{HARVESTER}: shim failed for entry point '{ep}' (exit {:?}): {what}",
                    out.exit_code
                ),
                remediation,
            );
            err.upstream = Some(common::target_text(
                out.stderr_excerpt.clone(),
                pb::ContentOrigin::Target,
                "click-info:stderr",
            ));
            if let Some(p) = payload {
                err.details_json = p.to_string();
            }
            return Err(err);
        }
        Ok(vec![common::raw(
            HARVESTER,
            "click-info.json",
            "application/json",
            format!("argv:{} -I aa_click_info.py {ep}", python.realpath),
            pb::DiscoveryAuthority::Introspect,
            out.stdout,
        )])
    }

    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> AaResult<Vec<Candidate>> {
        let mut out = Vec::new();
        for r in common::raws_for("click-info", raw) {
            out.extend(candidates_from(target, r)?);
        }
        Ok(out)
    }
}

/// Pure: one recorded aa-click-info/1 document -> leaf candidates (sorted by native path).
pub(crate) fn candidates_from(target: &pb::Target, raw: &RawHarvest) -> AaResult<Vec<Candidate>> {
    let doc: Value = serde_json::from_slice(&raw.bytes)
        .map_err(|e| AaError::invalid(format!("{HARVESTER}: raw artifact is not JSON: {e}")))?;
    if doc.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        return Err(AaError::invalid(format!("{HARVESTER}: raw artifact is not {PROTOCOL}")));
    }
    if let Some(e) = doc.get("error") {
        return Err(AaError::invalid(format!(
            "{HARVESTER}: raw artifact records a shim error: {e}"
        )));
    }
    let root = doc
        .get("root")
        .filter(|r| r.is_object())
        .ok_or_else(|| AaError::invalid(format!("{HARVESTER}: raw artifact has no root command")))?;
    let globals = schema::detect_globals(root.get("params").unwrap_or(&Value::Null));
    let ctx = Walk {
        target,
        globals,
        interpreter_args: interpreter_args(&doc),
        source_sha: raw.sha256(),
    };
    let mut out = Vec::new();
    if root.get("commands").is_some() {
        ctx.walk(root, &mut Vec::new(), &mut out)?;
    } else {
        // A plain click.Command root: the target itself is the one capability, named after the entry point.
        let name = doc.get("entry_point").and_then(Value::as_str).unwrap_or("main");
        out.push(ctx.leaf(root, vec![name.to_owned()], false)?);
    }
    Ok(out)
}

/// `-I -m <main_module>` when the package has `__main__.py` (CLI-Anything), else a `-c` trampoline that calls
/// the console-script object (never a shell).
fn interpreter_args(doc: &Value) -> Vec<String> {
    if let Some(m) = doc.get("main_module").and_then(Value::as_str).filter(|m| !m.is_empty()) {
        return vec!["-I".into(), "-m".into(), m.to_owned()];
    }
    let value = doc.get("entry_value").and_then(Value::as_str).unwrap_or("");
    let (module, attr) = value.split_once(':').unwrap_or((value, "main"));
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if valid(module) && valid(attr.trim()) {
        let code = format!(
            "import importlib, sys; o = importlib.import_module({module:?})\nfor a in {:?}.split('.'): o = getattr(o, a)\nsys.exit(o())",
            attr.trim()
        );
        return vec!["-I".into(), "-c".into(), code];
    }
    vec!["-I".into()]
}

struct Walk<'a> {
    target: &'a pb::Target,
    globals: CliAnythingGlobals,
    interpreter_args: Vec<String>,
    source_sha: String,
}

impl Walk<'_> {
    fn walk(&self, group: &Value, path: &mut Vec<String>, out: &mut Vec<Candidate>) -> AaResult<()> {
        let Some(commands) = group.get("commands").and_then(Value::as_object) else {
            return Ok(());
        };
        let mut names: Vec<&String> = commands.keys().collect();
        names.sort();
        for name in names {
            let cmd = &commands[name.as_str()];
            if cmd.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            path.push(name.clone());
            if cmd.get("commands").is_some() {
                self.walk(cmd, path, out)?;
            } else {
                let interactive = INTERACTIVE_LEAVES.contains(&name.as_str());
                out.push(self.leaf(cmd, path.clone(), interactive)?);
            }
            path.pop();
        }
        Ok(())
    }

    fn leaf(&self, cmd: &Value, native_path: Vec<String>, interactive: bool) -> AaResult<Candidate> {
        let analysis = schema::analyze(cmd.get("params").unwrap_or(&Value::Array(Vec::new())), &self.globals)?;
        let native_id = native_path.join(" ");
        let help = cmd.get("help").and_then(Value::as_str).unwrap_or("").trim();
        let short = cmd
            .get("short_help")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or_else(|| short_help(help), str::to_owned);
        let title = if short.is_empty() {
            common::target_text(native_id.clone(), pb::ContentOrigin::Engine, "click-info:path")
        } else {
            common::target_text(short, pb::ContentOrigin::Target, "click-help")
        };
        let description = (!help.is_empty())
            .then(|| common::target_text(help, pb::ContentOrigin::Target, format!("click-info:{native_id}")));
        let mut scopes: Vec<pb::ScopeParam> = analysis
            .paths
            .iter()
            .map(|(pointer, u)| pb::ScopeParam {
                pointer: pointer.clone(),
                kind: match u {
                    PathUse::Read => pb::ScopeKind::ReadPath,
                    PathUse::Write => pb::ScopeKind::WritePath,
                } as i32,
            })
            .collect();
        let json = self.globals.json_flag.is_some();
        let mut binding = pb::CliArgvBinding {
            executable_role: "interpreter".into(),
            interpreter_args: self.interpreter_args.clone(),
            global_args: self.globals.json_flag.iter().cloned().collect(),
            command_path: native_path.clone(),
            args: analysis.args,
            output: if json {
                pb::OutputMode::JsonStdout
            } else {
                pb::OutputMode::TextStdout
            } as i32,
            errors: if json {
                pb::ErrorConvention::CliAnythingJson
            } else {
                pb::ErrorConvention::ExitCode
            } as i32,
            deadline_ms: common::DEFAULT_DEADLINE_MS,
            max_output_bytes: common::DEFAULT_MAX_OUTPUT_BYTES,
            kill_scope: pb::KillScope::OwnedGroup as i32,
            usage_exit_code: CLICK_USAGE_EXIT,
            ..Default::default()
        };
        if let Some(flag) = &self.globals.project_flag {
            binding.state_handle_flag = flag.clone();
            binding.state_handle_pointer = "/project".into();
            binding.state_handle_engine_supplied = true;
            scopes.push(pb::ScopeParam {
                pointer: "/project".into(),
                kind: pb::ScopeKind::StateHandle as i32,
            });
        }
        let dry_run = self.globals.dry_run_flag.as_ref().map(|flag| pb::DryRun {
            supported: true,
            argv: vec![flag.clone()],
            kind: pb::DryRunKind::NoPersist as i32,
            verified: false,
        });
        common::candidate(
            self.target,
            CandidateParts {
                native_path,
                title,
                description,
                schema: analysis.schema,
                binding,
                scopes,
                dry_run,
                source_sha: self.source_sha.clone(),
                input_confidence: pb::Confidence::Declared,
                interactive,
            },
        )
    }
}

/// Click's `make_default_short_help` in spirit: the first sentence of the first paragraph, at most 120 chars.
fn short_help(help: &str) -> String {
    let para = help
        .split("\n\n")
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let sentence = para.split_inclusive(". ").next().unwrap_or("").trim_end();
    let s = sentence.strip_suffix('.').unwrap_or(sentence);
    if s.chars().count() > 120 {
        let cut: String = s.chars().take(117).collect();
        format!("{cut}...")
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_help_takes_first_sentence() {
        assert_eq!(short_help("List notes."), "List notes");
        assert_eq!(short_help("Show stats. Writes a cache.\n\nMore."), "Show stats");
        assert_eq!(short_help(""), "");
    }

    #[test]
    fn trampoline_when_no_main_module() {
        let args = interpreter_args(&serde_json::json!({"entry_value": "pkg.cli:main", "main_module": null}));
        assert_eq!(args[..2], ["-I".to_owned(), "-c".to_owned()]);
        assert!(args[2].contains("\"pkg.cli\""));
        let args = interpreter_args(&serde_json::json!({"main_module": "pkg"}));
        assert_eq!(args, ["-I", "-m", "pkg"]);
        let args = interpreter_args(&serde_json::json!({"entry_value": "bad;rm:x", "main_module": null}));
        assert_eq!(args, ["-I"]);
    }

    #[test]
    fn entry_point_resolution() {
        let mut t = pb::Target {
            id: "cli-anything.libreoffice".into(),
            kind: pb::TargetKind::CliAnythingHarness as i32,
            ..Default::default()
        };
        assert_eq!(
            entry_point(&t, &Value::Null).as_deref(),
            Some("cli-anything-libreoffice")
        );
        assert_eq!(
            entry_point(&t, &serde_json::json!({"entry_point": "x"})).as_deref(),
            Some("x")
        );
        t.kind = pb::TargetKind::CliTool as i32;
        assert_eq!(entry_point(&t, &Value::Null), None);
    }
}
