//! `help-text@1`: git parse-options and GNU-style `-h`/`--help` parsing.
//!
//! `harvest()` runs `X <cmd> -h` (overlay `help_argv`, default `["{command}", "-h"]`) for each command in the
//! overlay include-list and records every call verbatim in one JSON artifact (`aa-help-text/1`). git prints
//! parse-options usage and exits 129, which is the expected exit for this harvester. `candidates()` is pure:
//! `--[no-]x` becomes a negatable flag, `--opt[=<v>]` and `--opt <v>` become string options (an enum when the
//! help lists the values), and `<pathspec>...` in the usage line becomes a positional array.

use crate::common::{self, CandidateParts, Scratch};
use aa_core::{
    AaError, AaResult,
    api::{Candidate, HarvestCtx, Harvester, RawHarvest},
    pb,
};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::{sync::LazyLock, time::Duration};

#[derive(Debug, Clone, PartialEq)]
pub struct OptionSpec {
    pub long: String,
    pub short: Option<char>,
    pub value: Option<String>,
    pub negatable: bool,
    pub help: String,
}

/// `"-s, --[no-]short    show status concisely"`, `"--untracked-files[=<mode>]"`.
pub fn parse_git_parse_options(text: &str) -> Vec<OptionSpec> {
    parse_option_lines(text)
}

/// GNU getopt-style (`-o, --output=FILE  desc`, `--limit int  desc` as printed by Cobra/pflag).
pub fn parse_gnu(text: &str) -> Vec<OptionSpec> {
    parse_option_lines(text)
}

/// Options start within this many columns; deeper-indented lines continue the previous option's help.
const MAX_OPTION_INDENT: usize = 10;

fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 8 } else { 1 })
        .sum()
}

fn looks_like_option(trimmed: &str) -> bool {
    let b = trimmed.as_bytes();
    match b {
        [b'-', b'-', c, ..] => c.is_ascii_alphanumeric() || *c == b'[',
        [b'-', c, ..] => c.is_ascii_alphanumeric() || *c == b'?',
        _ => false,
    }
}

/// Shared core of both parsers: git notation (`--[no-]x`, `[=<v>]`) and GNU/pflag notation (`=FILE`, ` int`).
fn parse_option_lines(text: &str) -> Vec<OptionSpec> {
    let mut out: Vec<OptionSpec> = Vec::new();
    let mut open = false; // the last pushed option may still receive continuation lines
    for line in text.lines() {
        let trimmed = line.trim();
        let indent = indent_of(line);
        if trimmed.is_empty() {
            open = false;
            continue;
        }
        if (1..=MAX_OPTION_INDENT).contains(&indent) && looks_like_option(trimmed) {
            match parse_option_line(trimmed) {
                Some(spec) => {
                    out.push(spec);
                    open = true;
                }
                None => open = false,
            }
        } else if open && indent > MAX_OPTION_INDENT {
            if let Some(last) = out.last_mut() {
                if !last.help.is_empty() {
                    last.help.push(' ');
                }
                last.help.push_str(trimmed);
            }
        } else {
            open = false;
        }
    }
    out
}

/// One option line (already trimmed). Returns None if the line does not start with a parsable option.
fn parse_option_line(s: &str) -> Option<OptionSpec> {
    let mut spec = OptionSpec {
        long: String::new(),
        short: None,
        value: None,
        negatable: false,
        help: String::new(),
    };
    let mut rest = s;
    loop {
        rest = parse_alternative(rest, &mut spec)?;
        match rest.strip_prefix(", ") {
            Some(r) if r.starts_with('-') => rest = r,
            _ => break,
        }
    }
    // A separate value token: ` <v>`, ` (a|b)x`, or a single word followed by a 2+ space gap / end of line.
    if spec.value.is_none()
        && let Some(r) = rest.strip_prefix(' ').filter(|r| !r.starts_with(' '))
    {
        let end = r.find(' ').unwrap_or(r.len());
        let (token, after) = r.split_at(end);
        let gap = after.is_empty() || after.starts_with("  ");
        if !token.is_empty() && (token.starts_with('<') || token.starts_with('(') || gap) {
            spec.value = Some(clean_value(token));
            rest = after;
        }
    }
    spec.help = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    (!spec.long.is_empty() || spec.short.is_some()).then_some(spec)
}

/// Parses `--[no-]name[=<v>]`, `--name=V`, `-x` at the start of `s`; returns the remainder.
fn parse_alternative<'a>(s: &'a str, spec: &mut OptionSpec) -> Option<&'a str> {
    let (rest, attached) = if let Some(r) = s.strip_prefix("--") {
        let (r, negatable) = match r.strip_prefix("[no-]") {
            Some(r) => (r, true),
            None => (r, false),
        };
        let end = r
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .unwrap_or(r.len());
        if end == 0 {
            return None;
        }
        spec.long = r[..end].to_owned();
        spec.negatable |= negatable;
        (&r[end..], true)
    } else {
        let r = s.strip_prefix('-')?;
        let c = r.chars().next().filter(|c| c.is_ascii_alphanumeric() || *c == '?')?;
        spec.short = Some(c);
        (&r[c.len_utf8()..], false)
    };
    // Attached value: `[=<v>]`, `[=...]`, `=<v>`, `=V`, `[<v>]`.
    if rest.starts_with("[=") || rest.starts_with('=') || (attached && rest.starts_with("[<")) {
        let end = value_end(rest);
        spec.value = Some(clean_value(&rest[..end]));
        return Some(&rest[end..]);
    }
    Some(rest)
}

/// End of an attached value token: balanced brackets, stops at whitespace or `,` at depth 0.
fn value_end(s: &str) -> usize {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' | '<' | '{' => depth += 1,
            ']' | ')' | '>' | '}' => depth -= 1,
            ' ' | ',' if depth <= 0 => return i,
            _ => {}
        }
    }
    s.len()
}

/// `[=<mode>]` -> `mode`, `=<n>` -> `n`, `=PATTERN` -> `PATTERN`, `<range:file>` -> `range:file`,
/// `[=(direct|inherit)]` -> `(direct|inherit)`.
fn clean_value(token: &str) -> String {
    let mut v = token;
    if let Some(inner) = v.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        v = inner;
    }
    v = v.strip_prefix('=').unwrap_or(v);
    if let Some(inner) = v.strip_prefix('<').and_then(|x| x.strip_suffix('>'))
        && !inner.contains(['<', '>'])
    {
        v = inner;
    }
    v.to_owned()
}

static HELP_ENUM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:modes?|when|values?|one of|styles?)\s*:\s*([A-Za-z0-9_+-]+(?:\s*,\s*[A-Za-z0-9_+-]+)+)")
        .expect("static regex")
});
static BRACE_ENUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{([A-Za-z0-9_+-]+(?:\|[A-Za-z0-9_+-]+)+)\}").expect("static regex"));

/// Enumerated values, if the value token (`(a|b)`) or the help (`optional modes: a, b, c`, `{a|b}`) lists them.
pub(crate) fn enum_values(spec: &OptionSpec) -> Option<Vec<String>> {
    if let Some(v) = &spec.value
        && let Some(inner) = v.strip_prefix('(').and_then(|x| x.strip_suffix(')'))
        && inner.contains('|')
        && inner.chars().all(|c| c.is_ascii_alphanumeric() || "|_-".contains(c))
    {
        return Some(inner.split('|').map(str::to_owned).collect());
    }
    if let Some(c) = HELP_ENUM.captures(&spec.help) {
        return Some(c[1].split(',').map(|s| s.trim().to_owned()).collect());
    }
    BRACE_ENUM
        .captures(&spec.help)
        .map(|c| c[1].split('|').map(str::to_owned).collect())
}

/// A `<placeholder>` from a usage line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Positional {
    pub name: String,
    pub required: bool,
    pub variadic: bool,
}

/// The first usage line: `usage: git status [<options>] [--] [<pathspec>...]` or the line after `USAGE`/`Usage:`.
pub(crate) fn usage_line(text: &str) -> Option<String> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("usage:") {
            if rest.trim().is_empty() {
                return lines.find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned());
            }
            return Some(t[t.len() - rest.len()..].trim().to_owned());
        }
        if t == "USAGE" {
            return lines.find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned());
        }
    }
    None
}

/// The whole usage block: the first usage line plus every following git-style `or: …` line. aa-core's help-text
/// multi-mode rule reads the `or:` forms (e.g. `git rev-parse --parseopt …` / `or: git rev-parse --sq-quote …`) to
/// withhold an unsplit native candidate as MIXED_AUTHORITY.
pub(crate) fn usage_block(text: &str) -> Option<String> {
    let first = usage_line(text)?;
    let mut out = vec![format!("usage: {first}")];
    let mut lines = text.lines().map(str::trim);
    // Skip to the first usage line, then collect the contiguous `or:` lines after it.
    if lines.by_ref().any(|l| l.ends_with(first.as_str())) {
        for l in lines {
            match l.strip_prefix("or:") {
                Some(rest) => out.push(format!("   or: {}", rest.trim())),
                None => break,
            }
        }
    }
    Some(out.join("\n"))
}

/// Positionals of a usage line. If the line has a `--` separator, only placeholders after it count (the binding
/// always emits positionals after `--`). Option values (`--format <fmt>`) and `<options>`/`<flags>` are skipped.
pub(crate) fn parse_usage_positionals(usage: &str) -> Vec<Positional> {
    let mut found: Vec<(usize, Positional)> = Vec::new();
    let mut separator_at: Option<usize> = None;
    let mut depth = 0i32;
    let mut prev_word = String::new();
    let chars: Vec<char> = usage.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            '<' => {
                let Some(len) = chars[i + 1..].iter().position(|&c| c == '>') else {
                    break;
                };
                let name: String = chars[i + 1..i + 1 + len].iter().collect();
                let after = i + 2 + len;
                let variadic = chars[after..].starts_with(&['.', '.', '.']);
                let is_option_value = prev_word.starts_with('-') && prev_word != "--";
                let lower = name.to_ascii_lowercase();
                if !is_option_value && lower != "options" && lower != "flags" {
                    found.push((
                        i,
                        Positional {
                            name,
                            required: depth <= 0,
                            variadic,
                        },
                    ));
                }
                prev_word.clear();
                i = after;
                continue;
            }
            _ => {}
        }
        if chars[i] == '-' && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let end = chars[i..]
                .iter()
                .position(|c| c.is_whitespace() || "[](){}|".contains(*c))
                .map_or(chars.len(), |p| i + p);
            let word: String = chars[i..end].iter().collect();
            if word == "--" {
                separator_at = Some(i);
            }
            prev_word = word;
            i = end;
            continue;
        }
        if chars[i].is_alphanumeric() {
            prev_word.clear();
        }
        i += 1;
    }
    found
        .into_iter()
        .filter(|(at, _)| separator_at.is_none_or(|s| *at > s))
        .map(|(_, p)| p)
        .collect()
}

/// Integer-looking value placeholders (`<n>`, `int`, ...).
fn is_integer_value(v: &str) -> bool {
    matches!(
        v.to_ascii_lowercase().as_str(),
        "n" | "num" | "number" | "count" | "depth" | "int" | "uint" | "int32" | "int64" | "uint32" | "uint64"
    )
}

/// Options + usage positionals -> closed schema and ArgMappings. Shared with `cobra-complete@1`.
pub(crate) fn options_to_schema(
    options: &[OptionSpec],
    positionals: &[Positional],
    repeated_value: impl Fn(&str) -> bool,
) -> (Value, Vec<pb::ArgMapping>) {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut args = Vec::new();
    for o in options {
        if o.long == "help" || (o.long.is_empty() && o.short == Some('h')) {
            continue;
        }
        let (name, flag, short_only) = if o.long.is_empty() {
            // Short-only options keep their case (`-D` is not `-d`).
            let c = o.short.filter(char::is_ascii_alphanumeric).unwrap_or('?');
            let name = if c == '?' { String::new() } else { c.to_string() };
            (name, format!("-{c}"), true)
        } else {
            (common::property_name(&o.long), format!("--{}", o.long), false)
        };
        if name.is_empty() || properties.contains_key(&name) {
            continue;
        }
        let pointer = common::json_pointer(&name);
        let (schema, style) = match &o.value {
            None if o.negatable => (json!({"type": "boolean"}), pb::ArgStyle::NegatableFlag),
            None => (json!({"type": "boolean"}), pb::ArgStyle::Flag),
            Some(v) => {
                let item = if let Some(values) = enum_values(o) {
                    json!({"type": "string", "enum": values})
                } else if is_integer_value(v) {
                    json!({"type": "integer"})
                } else if matches!(v.to_ascii_lowercase().as_str(), "float" | "float32" | "float64") {
                    json!({"type": "number"})
                } else {
                    json!({"type": "string"})
                };
                if repeated_value(v) {
                    (json!({"type": "array", "items": item}), pb::ArgStyle::RepeatedOption)
                } else {
                    (item, pb::ArgStyle::Option)
                }
            }
        };
        properties.insert(name, schema);
        args.push(pb::ArgMapping {
            pointer,
            style: style as i32,
            flag,
            position: 0,
            no_equals_form: short_only && o.value.is_some(),
        });
    }
    let mut position = 0u32;
    for p in positionals {
        let name = common::property_name(&p.name);
        if name.is_empty() || properties.contains_key(&name) {
            continue;
        }
        let schema = if p.variadic {
            let mut s = json!({"type": "array", "items": {"type": "string"}});
            if p.required {
                s["minItems"] = json!(1);
            }
            s
        } else {
            json!({"type": "string"})
        };
        if p.required {
            required.push(name.clone());
        }
        args.push(pb::ArgMapping {
            pointer: common::json_pointer(&name),
            style: pb::ArgStyle::Positional as i32,
            flag: String::new(),
            position,
            no_equals_form: false,
        });
        position += 1;
        properties.insert(name, schema);
    }
    (common::object_schema(properties, required), args)
}

const PROTOCOL: &str = "aa-help-text/1";
const HARVESTER: &str = "help-text@1";
const CALL_DEADLINE: Duration = Duration::from_secs(10);
const MAX_STDOUT: u64 = 1024 * 1024;
const MAX_COMMANDS: usize = 200;

/// id "help-text", v1, INTROSPECT.
pub struct HelpTextHarvester;

fn help_argv(template: &[String], command: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for t in template {
        if t == "{command}" {
            out.extend(command.iter().map(|s| (*s).to_owned()));
        } else {
            out.push(t.replace("{command}", &command.join(" ")));
        }
    }
    out
}

#[async_trait::async_trait]
impl Harvester for HelpTextHarvester {
    fn id(&self) -> &'static str {
        "help-text"
    }

    fn version(&self) -> u32 {
        1
    }

    fn authority(&self) -> pb::DiscoveryAuthority {
        pb::DiscoveryAuthority::Introspect
    }

    fn applies(&self, target: &pb::Target, options: &serde_json::Value) -> bool {
        common::identity_file(target, "executable").is_some() && !common::opt_strings(options, "commands").is_empty()
    }

    async fn harvest(&self, ctx: HarvestCtx<'_>) -> AaResult<Vec<RawHarvest>> {
        common::require_authorization(ctx.target, HARVESTER)?;
        let exe = common::require_file(ctx.target, "executable", HARVESTER)?;
        let commands = common::opt_strings(ctx.options, "commands");
        if commands.is_empty() || commands.len() > MAX_COMMANDS {
            return Err(AaError::new(
                pb::ErrorCode::ConfigurationInvalid,
                format!("{HARVESTER} needs 1..={MAX_COMMANDS} commands (got {})", commands.len()),
                "list the commands to harvest in target.aa.yaml harvest[].commands, then: aa discover <target>",
            ));
        }
        let mut template = common::opt_strings(ctx.options, "help_argv");
        if template.is_empty() {
            template = vec!["{command}".into(), "-h".into()];
        }
        let scratch = Scratch::create(ctx.scratch_parent)?;
        let env = scratch.env(&[
            ("PAGER", "cat"),
            ("GIT_PAGER", "cat"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_OPTIONAL_LOCKS", "0"),
        ]);
        let mut calls = Vec::new();
        for command in &commands {
            let tokens: Vec<&str> = command.split_whitespace().collect();
            if tokens.is_empty() || tokens.iter().any(|t| t.starts_with('-') || t.contains('\0')) {
                return Err(AaError::invalid(format!("{HARVESTER}: invalid command {command:?}")));
            }
            let args = help_argv(&template, &tokens);
            let mut argv = vec![exe.realpath.clone()];
            argv.extend(args.iter().cloned());
            let spec = scratch.spec(
                argv,
                env.clone(),
                CALL_DEADLINE,
                MAX_STDOUT,
                ctx.target,
                ctx.allow_network,
            );
            let out = ctx.runner.run(spec).await?;
            common::check_run(HARVESTER, &format!("`{command} -h`"), &out)?;
            calls.push(json!({
                "command": tokens.join(" "),
                "args": args,
                "exit_code": out.exit_code,
                "stdout": String::from_utf8_lossy(&out.stdout),
                "stderr": out.stderr_excerpt,
            }));
        }
        let doc = json!({
            "protocol": PROTOCOL,
            "executable": exe.realpath,
            "help_argv": template,
            "calls": calls,
        });
        let mut bytes = serde_json::to_vec_pretty(&doc)?;
        bytes.push(b'\n');
        Ok(vec![common::raw(
            HARVESTER,
            "help-text.json",
            "application/json",
            format!("argv:{} {{{}}} -h", exe.realpath, commands.join(",")),
            pb::DiscoveryAuthority::Introspect,
            bytes,
        )])
    }

    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> AaResult<Vec<Candidate>> {
        let mut out = Vec::new();
        for r in common::raws_for("help-text", raw) {
            let doc: Value = serde_json::from_slice(&r.bytes)
                .map_err(|e| AaError::invalid(format!("{HARVESTER}: raw artifact is not JSON: {e}")))?;
            if doc.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
                return Err(AaError::invalid(format!("{HARVESTER}: raw artifact is not {PROTOCOL}")));
            }
            let exe = common::file_name(doc.get("executable").and_then(Value::as_str).unwrap_or(""));
            let sha = r.sha256();
            for call in doc.get("calls").and_then(Value::as_array).into_iter().flatten() {
                out.push(call_candidate(target, &exe, call, &sha)?);
            }
        }
        out.sort_by(|a, b| a.native_path.cmp(&b.native_path));
        out.dedup_by(|a, b| a.native_path == b.native_path);
        Ok(out)
    }
}

fn call_candidate(target: &pb::Target, exe: &str, call: &Value, sha: &str) -> AaResult<Candidate> {
    let command = call.get("command").and_then(Value::as_str).unwrap_or("").trim();
    if command.is_empty() {
        return Err(AaError::invalid(format!("{HARVESTER}: a recorded call has no command")));
    }
    let stdout = call.get("stdout").and_then(Value::as_str).unwrap_or("");
    let stderr = call.get("stderr").and_then(Value::as_str).unwrap_or("");
    let text = if stdout.trim().is_empty() { stderr } else { stdout };
    let options = parse_option_lines(text);
    let usage = usage_line(text);
    let positionals = usage.as_deref().map(parse_usage_positionals).unwrap_or_default();
    let (schema, args) = options_to_schema(&options, &positionals, |_| false);
    let exit = call.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
    let native_path: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
    let binding = pb::CliArgvBinding {
        executable_role: "executable".into(),
        command_path: native_path.clone(),
        args,
        output: pb::OutputMode::TextStdout as i32,
        errors: pb::ErrorConvention::ExitCode as i32,
        deadline_ms: common::DEFAULT_DEADLINE_MS,
        max_output_bytes: common::DEFAULT_MAX_OUTPUT_BYTES,
        kill_scope: pb::KillScope::OwnedGroup as i32,
        // `-h` exits with the parser's usage code on parse-options CLIs (git: 129); 0 = unknown (overlay sets it).
        usage_exit_code: i32::try_from(exit).ok().filter(|c| *c != 0 && *c != 1).unwrap_or(0),
        ..Default::default()
    };
    let description = usage_block(text)
        .map(|u| common::target_text(u, pb::ContentOrigin::Target, format!("help-text:{exe} {command} -h")));
    common::candidate(
        target,
        CandidateParts {
            native_path,
            title: common::target_text(format!("{exe} {command}"), pb::ContentOrigin::Engine, "help-text:argv"),
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

    fn opt(long: &str, short: Option<char>, value: Option<&str>, negatable: bool, help: &str) -> OptionSpec {
        OptionSpec {
            long: long.into(),
            short,
            value: value.map(str::to_owned),
            negatable,
            help: help.into(),
        }
    }

    #[test]
    fn git_parse_options_lines() {
        let text = "usage: git status [<options>] [--] [<pathspec>...]\n\n\
            \x20   -s, --[no-]short      show status concisely\n\
            \x20   --[no-]porcelain[=<version>]\n\
            \x20                         machine-readable output\n\
            \x20   -u, --[no-]untracked-files[=<mode>]\n\
            \x20                         show untracked files, optional modes: all, normal, no. (Default: all)\n\
            \x20   --no-renames          do not detect renames\n\
            \x20   -M, --find-renames[=<n>]\n\
            \x20                         detect renames, optionally set similarity index\n\
            \x20   --[no-]decorate[=...] decorate options\n\
            \x20   --[no-]unset-upstream unset the upstream info\n\
            \x20   -L <range:file>       trace the evolution of line range\n\
            \x20   -t, --[no-]track[=(direct|inherit)]\n\
            \x20                         set branch tracking configuration\n\
            \x20   --[no-]chmod (+|-)x   override the executable bit\n\
            \x20   -u, --[no-]set-upstream-to <upstream>\n\
            \x20                         change the upstream info\n\
            \x20   -D                    delete branch (even if not merged)\n";
        let o = parse_git_parse_options(text);
        assert_eq!(o[0], opt("short", Some('s'), None, true, "show status concisely"));
        assert_eq!(
            o[1],
            opt("porcelain", None, Some("version"), true, "machine-readable output")
        );
        assert_eq!(o[2].long, "untracked-files");
        assert_eq!(o[2].value.as_deref(), Some("mode"));
        assert!(o[2].negatable);
        assert_eq!(enum_values(&o[2]).unwrap(), ["all", "normal", "no"]);
        assert_eq!(o[3], opt("no-renames", None, None, false, "do not detect renames"));
        assert_eq!(o[4].value.as_deref(), Some("n"));
        assert_eq!(o[5], opt("decorate", None, Some("..."), true, "decorate options"));
        assert_eq!(o[6], opt("unset-upstream", None, None, true, "unset the upstream info"));
        assert_eq!(
            o[7],
            opt(
                "",
                Some('L'),
                Some("range:file"),
                false,
                "trace the evolution of line range"
            )
        );
        assert_eq!(enum_values(&o[8]).unwrap(), ["direct", "inherit"]);
        assert_eq!(o[9].value.as_deref(), Some("(+|-)x"));
        assert_eq!(o[9].help, "override the executable bit");
        assert_eq!(o[10].value.as_deref(), Some("upstream"));
        assert_eq!(
            o[11],
            opt("", Some('D'), None, false, "delete branch (even if not merged)")
        );
        assert_eq!(o.len(), 12);
    }

    #[test]
    fn spec_examples_from_build_plan() {
        let o = parse_git_parse_options("    -s, --[no-]short    show status concisely\n");
        assert_eq!(o, vec![opt("short", Some('s'), None, true, "show status concisely")]);
        let o = parse_git_parse_options("    --untracked-files[=<mode>]\n");
        assert_eq!(o, vec![opt("untracked-files", None, Some("mode"), false, "")]);
    }

    #[test]
    fn gnu_and_pflag_lines() {
        let text = "Usage: ls [OPTION]... [FILE]...\n\
            \x20 -a, --all                  do not ignore entries starting with .\n\
            \x20     --block-size=SIZE      with -l, scale sizes by SIZE\n\
            \x20 -I, --ignore=PATTERN       do not list implied entries matching shell PATTERN\n\
            \x20 -w, --width=COLS           set output width to COLS.  0 means no limit\n\
            FLAGS\n\
            \x20     --app string        Filter by GitHub App author\n\
            \x20 -L, --limit int         Maximum number of items to fetch (default 30)\n\
            \x20 -l, --label strings     Filter by label\n\
            \x20 -s, --state string      Filter by state: {open|closed|merged|all} (default \"open\")\n\
            \x20 -R, --repo [HOST/]OWNER/REPO   Select another repository\n\
            \x20 -d, --draft             Filter by draft state\n";
        let o = parse_gnu(text);
        assert_eq!(
            o[0],
            opt("all", Some('a'), None, false, "do not ignore entries starting with .")
        );
        assert_eq!(o[1].value.as_deref(), Some("SIZE"));
        assert_eq!(o[2].value.as_deref(), Some("PATTERN"));
        assert_eq!(o[3].help, "set output width to COLS. 0 means no limit");
        assert_eq!(
            o[4],
            opt("app", None, Some("string"), false, "Filter by GitHub App author")
        );
        assert_eq!(o[5].value.as_deref(), Some("int"));
        assert_eq!(enum_values(&o[7]).unwrap(), ["open", "closed", "merged", "all"]);
        assert_eq!(o[8].value.as_deref(), Some("[HOST/]OWNER/REPO"));
        assert_eq!(o[9], opt("draft", Some('d'), None, false, "Filter by draft state"));
    }

    #[test]
    fn usage_block_keeps_or_lines() {
        let text = "usage: git rev-parse --parseopt [<options>] -- [<args>...]\n   or: git rev-parse --sq-quote [<arg>...]\n   or: git rev-parse [<options>] [<arg>...]\n\nRun \"git rev-parse --parseopt -h\" for more\n";
        assert_eq!(
            usage_block(text).as_deref(),
            Some(
                "usage: git rev-parse --parseopt [<options>] -- [<args>...]\n   or: git rev-parse --sq-quote [<arg>...]\n   or: git rev-parse [<options>] [<arg>...]"
            )
        );
        assert_eq!(
            usage_block("usage: git status [<options>] [--] [<pathspec>...]\n\n    -v\n").as_deref(),
            Some("usage: git status [<options>] [--] [<pathspec>...]")
        );
        assert_eq!(
            usage_block("USAGE\n  gh alias set <alias> <expansion> [flags]\n").as_deref(),
            Some("usage: gh alias set <alias> <expansion> [flags]")
        );
    }

    #[test]
    fn usage_positionals() {
        let p = parse_usage_positionals("git status [<options>] [--] [<pathspec>...]");
        assert_eq!(
            p,
            vec![Positional {
                name: "pathspec".into(),
                required: false,
                variadic: true
            }]
        );
        let p = parse_usage_positionals("git add [<options>] [--] <pathspec>...");
        assert!(p[0].required && p[0].variadic);
        let p = parse_usage_positionals("git log [<options>] [<revision-range>] [[--] <path>...]");
        assert_eq!(
            p.len(),
            1,
            "placeholders before `--` would be misread after the binding's `--`: {p:?}"
        );
        assert_eq!(p[0].name, "path");
        let p = parse_usage_positionals("gh alias set <alias> <expansion> [flags]");
        assert_eq!(
            p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["alias", "expansion"]
        );
        assert!(p.iter().all(|p| p.required && !p.variadic));
        let p = parse_usage_positionals("gh alias delete {<alias> | --all} [flags]");
        assert!(!p[0].required, "an alternative group is not unconditionally required");
        let p = parse_usage_positionals("x [--format <fmt>] <file>");
        assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["file"]);
        assert_eq!(
            usage_line("USAGE\n  gh alias set <alias> <expansion> [flags]\n").as_deref(),
            Some("gh alias set <alias> <expansion> [flags]")
        );
        assert_eq!(
            usage_line("usage: git add [<options>] [--] <pathspec>...\n").as_deref(),
            Some("git add [<options>] [--] <pathspec>...")
        );
    }

    #[test]
    fn help_argv_template() {
        let t = vec!["{command}".to_owned(), "-h".to_owned()];
        assert_eq!(help_argv(&t, &["remote", "add"]), ["remote", "add", "-h"]);
        let t = vec!["help".to_owned(), "{command}".to_owned()];
        assert_eq!(help_argv(&t, &["status"]), ["help", "status"]);
    }
}
