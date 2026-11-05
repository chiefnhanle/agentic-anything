//! `cobra-complete@1`: Cobra `__complete` crawl.
//!
//! `harvest()` crawls recursively with `X __complete <path…> ""` (subcommands, `name\tdescription` lines and a
//! trailing `:<directive>` line), `X __complete <path…> --` on leaves (flags), and `X <path…> --help` on leaves
//! (flag value types and the usage line). It respects `max_depth` 4 and `max_calls` 600 (options may only
//! lower them) and an optional `commands` include-list of top-level commands. Every call's argv and output is
//! recorded as one line of a JSONL artifact. `candidates()` is pure: leaves become capabilities, flags with
//! descriptions become options; the usage-error convention is exit 1 + `"unknown flag"` (gh, observed).

use crate::{
    common::{self, CandidateParts, Scratch},
    help_text::{self, OptionSpec},
};
use aa_core::{
    AaError, AaResult,
    api::{Candidate, HarvestCtx, Harvester, RawHarvest},
    pb,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

const PROTOCOL: &str = "aa-cobra-complete/1";
const HARVESTER: &str = "cobra-complete@1";
const MAX_DEPTH: u64 = 4;
const MAX_CALLS: u64 = 600;
const CALL_DEADLINE: Duration = Duration::from_secs(10);
const MAX_STDOUT: u64 = 1024 * 1024;
/// Cobra/gh usage errors: exit 1 plus this stderr marker (L1 requires both).
const USAGE_EXIT: i32 = 1;
const USAGE_MARKER: &str = "unknown flag";
/// Cobra built-ins that are not capabilities (`help` completes command names and would recurse).
const SKIP_COMMANDS: &[&str] = &["help", "__complete", "__completeNoDesc"];

/// id "cobra-complete", v1, INTROSPECT.
pub struct CobraCompleteHarvester;

/// One `__complete` listing: entries (name, description) and the directive, if the output had one.
#[derive(Debug, Default)]
struct Listing {
    entries: Vec<(String, String)>,
    directive: Option<u32>,
}

fn parse_listing(stdout: &str) -> Listing {
    let mut l = Listing::default();
    for line in stdout.lines() {
        if let Some(d) = line.strip_prefix(':').and_then(|d| d.trim().parse::<u32>().ok()) {
            l.directive = Some(d);
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let (name, desc) = line.split_once('\t').unwrap_or((line, ""));
        l.entries.push((name.trim().to_owned(), desc.trim().to_owned()));
    }
    l
}

struct Crawl<'a, 'c> {
    ctx: &'a HarvestCtx<'c>,
    scratch: Scratch,
    exe: String,
    env: BTreeMap<String, String>,
    lines: Vec<Value>,
    calls: u64,
    max_calls: u64,
    truncated: bool,
}

impl Crawl<'_, '_> {
    async fn call(&mut self, kind: &str, path: &[String], args: Vec<String>) -> AaResult<Option<String>> {
        if self.calls >= self.max_calls {
            self.truncated = true;
            return Ok(None);
        }
        self.calls += 1;
        let mut argv = vec![self.exe.clone()];
        argv.extend(args.iter().cloned());
        let spec = self.scratch.spec(
            argv,
            self.env.clone(),
            CALL_DEADLINE,
            MAX_STDOUT,
            self.ctx.target,
            self.ctx.allow_network,
        );
        let out = self.ctx.runner.run(spec).await?;
        common::check_run(HARVESTER, &format!("`{}`", args.join(" ")), &out)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        self.lines.push(json!({
            "kind": kind,
            "path": path,
            "args": args,
            "exit_code": out.exit_code,
            "stdout": stdout,
            "stderr": out.stderr_excerpt,
        }));
        Ok(Some(stdout))
    }
}

#[async_trait::async_trait]
impl Harvester for CobraCompleteHarvester {
    fn id(&self) -> &'static str {
        "cobra-complete"
    }

    fn version(&self) -> u32 {
        1
    }

    fn authority(&self) -> pb::DiscoveryAuthority {
        pb::DiscoveryAuthority::Introspect
    }

    fn applies(&self, target: &pb::Target, options: &serde_json::Value) -> bool {
        let _ = options;
        // A python_console_script (pinned interpreter) is not a Cobra binary; click-info covers it.
        common::identity_file(target, "executable").is_some() && common::identity_file(target, "interpreter").is_none()
    }

    async fn harvest(&self, ctx: HarvestCtx<'_>) -> AaResult<Vec<RawHarvest>> {
        common::require_authorization(ctx.target, HARVESTER)?;
        let exe = common::require_file(ctx.target, "executable", HARVESTER)?
            .realpath
            .clone();
        let max_depth = common::opt_bounded(ctx.options, "max_depth", MAX_DEPTH);
        let max_calls = common::opt_bounded(ctx.options, "max_calls", MAX_CALLS);
        let include = common::opt_strings(ctx.options, "commands");
        let with_help = ctx.options.get("help").and_then(Value::as_bool).unwrap_or(true);
        let scratch = Scratch::create(ctx.scratch_parent)?;
        let env = scratch.env(&[
            ("GH_NO_UPDATE_NOTIFIER", "1"),
            ("GH_PROMPT_DISABLED", "1"),
            ("GH_SPINNER_DISABLED", "1"),
            ("PAGER", "cat"),
        ]);
        let mut crawl = Crawl {
            ctx: &ctx,
            scratch,
            exe: exe.clone(),
            env,
            lines: vec![json!({
                "protocol": PROTOCOL,
                "executable": exe,
                "max_depth": max_depth,
                "max_calls": max_calls,
                "commands": include,
            })],
            calls: 0,
            max_calls,
            truncated: false,
        };
        // Depth-first, children in listing order.
        let mut stack: Vec<Vec<String>> = vec![Vec::new()];
        let mut root_seen = false;
        while let Some(path) = stack.pop() {
            let mut args = vec!["__complete".to_owned()];
            args.extend(path.iter().cloned());
            args.push(String::new());
            let Some(stdout) = crawl.call("commands", &path, args).await? else {
                break;
            };
            let listing = parse_listing(&stdout);
            if path.is_empty() {
                if listing.directive.is_none() {
                    return Err(AaError::new(
                        pb::ErrorCode::CapabilityUnavailable,
                        format!(
                            "{HARVESTER}: `{} __complete \"\"` printed no completion directive",
                            ctx.target.id
                        ),
                        "this is not a Cobra CLI; use help-text@1 in target.aa.yaml, then: aa discover <target>",
                    ));
                }
                root_seen = true;
            }
            let children: Vec<String> = listing
                .entries
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| !n.is_empty() && !n.starts_with('-') && !SKIP_COMMANDS.contains(&n.as_str()))
                .filter(|n| !path.is_empty() || include.is_empty() || include.iter().any(|i| i == n))
                .collect();
            if children.is_empty() && !path.is_empty() {
                let mut args = vec!["__complete".to_owned()];
                args.extend(path.iter().cloned());
                args.push("--".to_owned());
                if crawl.call("flags", &path, args).await?.is_none() {
                    break;
                }
                if with_help {
                    let mut args = path.clone();
                    args.push("--help".to_owned());
                    if crawl.call("help", &path, args).await?.is_none() {
                        break;
                    }
                }
                continue;
            }
            if path.len() as u64 >= max_depth {
                crawl.truncated = true;
                continue;
            }
            for name in children.into_iter().rev() {
                let mut p = path.clone();
                p.push(name);
                stack.push(p);
            }
        }
        if !root_seen {
            return Err(AaError::new(
                pb::ErrorCode::ResourceConstraint,
                format!("{HARVESTER}: max_calls={max_calls} allowed no call"),
                "raise harvest[].options.max_calls (<= 600) in target.aa.yaml, then: aa discover <target>",
            ));
        }
        let (calls, truncated) = (crawl.calls, crawl.truncated);
        crawl.lines.push(json!({"calls": calls, "truncated": truncated}));
        let mut bytes = Vec::new();
        for line in &crawl.lines {
            bytes.extend(serde_json::to_vec(line)?);
            bytes.push(b'\n');
        }
        Ok(vec![common::raw(
            HARVESTER,
            "cobra-complete.jsonl",
            "application/x-ndjson",
            format!("argv:{exe} __complete …"),
            pb::DiscoveryAuthority::Introspect,
            bytes,
        )])
    }

    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> AaResult<Vec<Candidate>> {
        let mut out = Vec::new();
        for r in common::raws_for("cobra-complete", raw) {
            out.extend(candidates_from(target, r)?);
        }
        Ok(out)
    }
}

#[derive(Default)]
struct Node {
    description: String,
    is_group: bool,
    flags: Option<Vec<(String, String)>>,
    help: Option<String>,
}

fn candidates_from(target: &pb::Target, raw: &RawHarvest) -> AaResult<Vec<Candidate>> {
    let text = std::str::from_utf8(&raw.bytes)
        .map_err(|e| AaError::invalid(format!("{HARVESTER}: raw artifact is not UTF-8: {e}")))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Value = serde_json::from_str(lines.next().unwrap_or(""))
        .map_err(|e| AaError::invalid(format!("{HARVESTER}: bad JSONL header: {e}")))?;
    if header.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        return Err(AaError::invalid(format!("{HARVESTER}: raw artifact is not {PROTOCOL}")));
    }
    let mut nodes: BTreeMap<Vec<String>, Node> = BTreeMap::new();
    for line in lines {
        let rec: Value =
            serde_json::from_str(line).map_err(|e| AaError::invalid(format!("{HARVESTER}: bad JSONL line: {e}")))?;
        let Some(kind) = rec.get("kind").and_then(Value::as_str) else {
            continue; // trailer
        };
        let path: Vec<String> = rec
            .get("path")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let stdout = rec.get("stdout").and_then(Value::as_str).unwrap_or("");
        match kind {
            "commands" => {
                let listing = parse_listing(stdout);
                let children: Vec<_> = listing
                    .entries
                    .into_iter()
                    .filter(|(n, _)| !n.is_empty() && !n.starts_with('-') && !SKIP_COMMANDS.contains(&n.as_str()))
                    .collect();
                if !children.is_empty() {
                    nodes.entry(path.clone()).or_default().is_group = true;
                }
                for (name, desc) in children {
                    let mut p = path.clone();
                    p.push(name);
                    nodes.entry(p).or_default().description = desc;
                }
            }
            "flags" => {
                let flags = parse_listing(stdout)
                    .entries
                    .into_iter()
                    .filter(|(n, d)| n.starts_with("--") && n != "--help" && !d.is_empty())
                    .collect();
                nodes.entry(path).or_default().flags = Some(flags);
            }
            "help" => nodes.entry(path).or_default().help = Some(stdout.to_owned()),
            _ => {}
        }
    }
    let exe = common::file_name(header.get("executable").and_then(Value::as_str).unwrap_or(""));
    let sha = raw.sha256();
    let mut out = Vec::new();
    for (path, node) in &nodes {
        // A leaf is a crawled command whose flags were listed and that has no subcommands.
        let Some(flags) = &node.flags else { continue };
        if node.is_group || path.is_empty() {
            continue;
        }
        out.push(leaf_candidate(target, &exe, path, node, flags, &sha)?);
    }
    Ok(out)
}

/// pflag value types that take repeated values (`strings`, `stringArray`, `ints`, ...).
fn is_repeated_type(v: &str) -> bool {
    matches!(
        v,
        "strings" | "stringArray" | "stringSlice" | "ints" | "intSlice" | "uints" | "floats" | "bools" | "durations"
    )
}

fn leaf_candidate(
    target: &pb::Target,
    exe: &str,
    path: &[String],
    node: &Node,
    flags: &[(String, String)],
    sha: &str,
) -> AaResult<Candidate> {
    let help = node.help.as_deref().unwrap_or("");
    let parsed: BTreeMap<String, OptionSpec> = help_text::parse_gnu(help)
        .into_iter()
        .filter(|o| !o.long.is_empty())
        .map(|o| (o.long.clone(), o))
        .collect();
    // Flags listed by __complete (with descriptions); types/shorts from --help when it agrees. A flag that
    // --help does not show keeps the always-valid `--flag=value` string form.
    let options: Vec<OptionSpec> = flags
        .iter()
        .map(|(name, desc)| {
            let long = name.trim_start_matches('-').to_owned();
            match parsed.get(&long) {
                Some(o) => OptionSpec {
                    help: if o.help.is_empty() {
                        desc.clone()
                    } else {
                        o.help.clone()
                    },
                    ..o.clone()
                },
                None => OptionSpec {
                    long,
                    short: None,
                    value: Some("string".into()),
                    negatable: false,
                    help: desc.clone(),
                },
            }
        })
        .collect();
    let positionals = help_text::usage_line(help)
        .map(|u| {
            // Only placeholders after the command path count (`gh alias set <alias> <expansion> [flags]`).
            let tail = path
                .last()
                .and_then(|leaf| u.find(&format!(" {leaf}")).map(|i| i + leaf.len() + 1));
            help_text::parse_usage_positionals(tail.map_or(u.as_str(), |i| &u[i..]))
        })
        .unwrap_or_default();
    let (schema, args) = help_text::options_to_schema(&options, &positionals, is_repeated_type);
    let native_id = path.join(" ");
    let binding = pb::CliArgvBinding {
        executable_role: "executable".into(),
        command_path: path.to_vec(),
        args,
        output: pb::OutputMode::TextStdout as i32,
        errors: pb::ErrorConvention::ExitCode as i32,
        deadline_ms: common::DEFAULT_DEADLINE_MS,
        max_output_bytes: common::DEFAULT_MAX_OUTPUT_BYTES,
        kill_scope: pb::KillScope::OwnedGroup as i32,
        usage_exit_code: USAGE_EXIT,
        usage_stderr_marker: USAGE_MARKER.into(),
        ..Default::default()
    };
    let title = if node.description.is_empty() {
        common::target_text(
            format!("{exe} {native_id}"),
            pb::ContentOrigin::Engine,
            "cobra-complete:path",
        )
    } else {
        common::target_text(node.description.clone(), pb::ContentOrigin::Target, "cobra-complete")
    };
    let description = (!node.description.is_empty()).then(|| {
        common::target_text(
            node.description.clone(),
            pb::ContentOrigin::Target,
            format!("cobra-complete:{exe} {native_id}"),
        )
    });
    common::candidate(
        target,
        CandidateParts {
            native_path: path.to_vec(),
            title,
            description,
            schema,
            binding,
            scopes: Vec::new(),
            dry_run: None,
            source_sha: sha.to_owned(),
            input_confidence: pb::Confidence::Inferred,
            interactive: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_parse() {
        let l = parse_listing("delete\tDelete set aliases\nlist\tList your aliases\n:4\n");
        assert_eq!(l.directive, Some(4));
        assert_eq!(l.entries[1], ("list".to_owned(), "List your aliases".to_owned()));
        let l = parse_listing(":0\n");
        assert!(l.entries.is_empty());
        assert_eq!(l.directive, Some(0));
        assert_eq!(parse_listing("not cobra").directive, None);
    }
}
