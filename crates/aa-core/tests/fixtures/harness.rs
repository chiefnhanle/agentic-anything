//! Test-only replay of recorded harvests into `Candidate`s, plus target/record builders.
//!
//! aa-core cannot depend on aa-harvest (dependency direction), so these are small, deterministic stand-ins for
//! the harvesters' pure `candidates()` step over the recorded fixtures in this directory. Included by the
//! integration tests with `#[path = "fixtures/harness.rs"] mod harness;`.
#![allow(dead_code, clippy::too_many_arguments)]

use aa_core::api::Candidate;
use aa_core::{digest, ids, pb};
use serde_json::{Map, Value, json};

pub const GIT_HELP: &str = include_str!("help-text.git.json");
pub const NOTES_CLICK: &str = include_str!("click-info.notes.json");
pub const GH_COBRA: &str = include_str!("cobra.gh-alias.json");
pub const GIT_OVERLAY: &str = include_str!("git.target.aa.yaml");
pub const NOTES_OVERLAY: &str = include_str!("notes.target.aa.yaml");
pub const GH_OVERLAY: &str = include_str!("gh.target.aa.yaml");

pub fn tt(text: &str, source: &str) -> Option<pb::TargetText> {
    Some(pb::TargetText {
        text: text.into(),
        origin: pb::ContentOrigin::Target as i32,
        source: source.into(),
    })
}

fn canonical(v: &Value) -> String {
    String::from_utf8(digest::jcs(v).unwrap()).unwrap()
}

fn schema(props: Map<String, Value>, required: Vec<String>) -> String {
    let mut s = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        s["required"] = Value::from(required);
    }
    canonical(&s)
}

fn prop_name(native: &str) -> String {
    native
        .trim_start_matches('-')
        .chars()
        .map(|c| if c == '-' { '_' } else { c.to_ascii_lowercase() })
        .collect()
}

fn candidate(
    target: &pb::Target,
    path: Vec<String>,
    title: &str,
    description: Option<pb::TargetText>,
    schema_json: String,
    binding: pb::CliArgvBinding,
    sha: &str,
    interactive: bool,
) -> Candidate {
    let refs: Vec<&str> = path.iter().map(String::as_str).collect();
    Candidate {
        capability: pb::Capability {
            id: ids::capability_id(&target.id, &refs),
            target_id: target.id.clone(),
            title: tt(title, "harvest"),
            description,
            input_schema_json: schema_json,
            binding: Some(pb::Binding {
                kind: Some(pb::binding::Kind::Cli(binding)),
            }),
            provenance: Some(pb::Provenance {
                source_shas: vec![sha.into()],
                native_ids: vec![path.join(" ")],
                ..Default::default()
            }),
            ..Default::default()
        },
        native_path: path,
        native_signals: Vec::new(),
        interactive,
    }
}

// ---------------------------------------------------------------------------
// help-text@1 (git parse-options)
// ---------------------------------------------------------------------------

struct Opt {
    flag: String,
    name: String,
    valued: bool,
    negatable: bool,
    help: String,
}

fn parse_opt_spec(spec: &str) -> Option<Opt> {
    let mut rest = spec.trim();
    let mut short = None;
    if rest.len() >= 4 && rest.starts_with('-') && !rest.starts_with("--") && rest[2..].starts_with(", ") {
        short = Some(rest[1..2].to_string());
        rest = &rest[4..];
    }
    if let Some(long) = rest.strip_prefix("--") {
        let (negatable, long) = match long.strip_prefix("[no-]") {
            Some(l) => (true, l),
            None => (false, long),
        };
        let end = long.find(['[', '=', ' ']).unwrap_or(long.len());
        let name = &long[..end];
        if name.is_empty() {
            return None;
        }
        return Some(Opt {
            flag: format!("--{name}"),
            name: prop_name(name),
            valued: end < long.len(),
            negatable,
            help: String::new(),
        });
    }
    let s = short.or_else(|| rest.strip_prefix('-').map(|r| r.chars().take(1).collect()))?;
    if s.is_empty() {
        return None;
    }
    Some(Opt {
        flag: format!("-{s}"),
        name: prop_name(&s),
        valued: rest.contains(' '),
        negatable: false,
        help: String::new(),
    })
}

fn parse_options(text: &str) -> Vec<Opt> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<Opt> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !line.starts_with(' ') || !trimmed.starts_with('-') {
            continue;
        }
        let (spec, help) = match trimmed.find("  ") {
            Some(p) => (&trimmed[..p], trimmed[p..].trim().to_string()),
            None => {
                let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or_default();
                (
                    trimmed,
                    if next.starts_with('-') {
                        String::new()
                    } else {
                        next.to_string()
                    },
                )
            }
        };
        if let Some(mut o) = parse_opt_spec(spec) {
            if out.iter().any(|x| x.name == o.name) {
                continue;
            }
            o.help = help;
            out.push(o);
        }
    }
    out
}

fn enum_values(help: &str) -> Option<Vec<String>> {
    let at = help
        .find("modes: ")
        .map(|p| p + 7)
        .or_else(|| help.find("when: ").map(|p| p + 6))?;
    let list = &help[at..];
    let list = &list[..list.find('.').unwrap_or(list.len())];
    Some(
        list.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// `(name, array, required)` positionals from the first usage line.
fn usage_positionals(text: &str) -> Vec<(String, bool, bool)> {
    let Some(line) = text.lines().find(|l| l.trim_start().starts_with("usage:")) else {
        return Vec::new();
    };
    let re = regex::Regex::new(r"<([a-z][a-z-]*)>(\.\.\.)?").unwrap();
    re.captures_iter(line)
        .filter(|c| &c[1] != "options")
        .map(|c| {
            let start = c.get(0).unwrap().start();
            let depth = line[..start].matches('[').count() as i64 - line[..start].matches(']').count() as i64;
            (prop_name(&c[1]), c.get(2).is_some(), depth <= 0)
        })
        .collect()
}

pub fn help_text_candidates(target: &pb::Target, raw: &str) -> Vec<Candidate> {
    let sha = digest::sha256_hex(raw.as_bytes());
    let doc: Value = serde_json::from_str(raw).unwrap();
    let program = doc["program"].as_str().unwrap();
    let mut out = Vec::new();
    for c in doc["commands"].as_array().unwrap() {
        let command = c["command"].as_str().unwrap().to_string();
        let text = format!("{}{}", c["stdout"].as_str().unwrap(), c["stderr"].as_str().unwrap());
        let mut props = Map::new();
        let mut required = Vec::new();
        let mut args = Vec::new();
        for o in parse_options(&text) {
            let (style, prop) = if o.valued {
                let mut p = json!({"type": "string"});
                if let Some(vals) = enum_values(&o.help) {
                    p = json!({"enum": vals});
                }
                (pb::ArgStyle::Option, p)
            } else if o.negatable {
                (pb::ArgStyle::NegatableFlag, json!({"type": "boolean"}))
            } else {
                (pb::ArgStyle::Flag, json!({"type": "boolean"}))
            };
            props.insert(o.name.clone(), prop);
            args.push(pb::ArgMapping {
                pointer: format!("/{}", o.name),
                style: style as i32,
                flag: o.flag,
                ..Default::default()
            });
        }
        for (i, (name, array, req)) in usage_positionals(&text).into_iter().enumerate() {
            let mut p = if array {
                json!({"type": "array", "items": {"type": "string"}})
            } else {
                json!({"type": "string"})
            };
            if array && req {
                p["minItems"] = json!(1);
            }
            if req {
                required.push(name.clone());
            }
            props.insert(name.clone(), p);
            args.push(pb::ArgMapping {
                pointer: format!("/{name}"),
                style: pb::ArgStyle::Positional as i32,
                position: i as u32,
                ..Default::default()
            });
        }
        let binding = pb::CliArgvBinding {
            executable_role: "executable".into(),
            command_path: vec![command.clone()],
            args,
            output: pb::OutputMode::TextStdout as i32,
            errors: pb::ErrorConvention::ExitCode as i32,
            kill_scope: pb::KillScope::OwnedGroup as i32,
            ..Default::default()
        };
        out.push(candidate(
            target,
            vec![command.clone()],
            &format!("{program} {command}"),
            tt(&text, &format!("help-text:{program} {command} -h")),
            schema(props, required),
            binding,
            &sha,
            false,
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// click-info@1
// ---------------------------------------------------------------------------

fn click_leaves<'a>(node: &'a Value, path: Vec<String>, out: &mut Vec<(Vec<String>, &'a Value)>) {
    match node.get("commands").and_then(Value::as_object) {
        Some(cmds) if !cmds.is_empty() => {
            for (name, child) in cmds {
                let mut p = path.clone();
                p.push(name.clone());
                click_leaves(child, p, out);
            }
        }
        _ if !path.is_empty() => out.push((path, node)),
        _ => {}
    }
}

pub fn click_candidates(target: &pb::Target, raw: &str) -> Vec<Candidate> {
    let sha = digest::sha256_hex(raw.as_bytes());
    let doc: Value = serde_json::from_str(raw).unwrap();
    let root = &doc["root"];
    let root_opts: Vec<String> = root["params"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| {
            p["opts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o.as_str().unwrap().to_string())
        })
        .collect();
    let mut leaves = Vec::new();
    click_leaves(root, Vec::new(), &mut leaves);
    let mut out = Vec::new();
    for (path, node) in leaves {
        let mut props = Map::new();
        let mut required = Vec::new();
        let mut args = Vec::new();
        let mut position = 0;
        for p in node["params"].as_array().unwrap() {
            let name = p["name"].as_str().unwrap().to_string();
            if name == "help" {
                continue;
            }
            let ty = p["type"]["param_type"].as_str().unwrap_or("String");
            let mut prop = match ty {
                "Int" => json!({"type": "integer"}),
                "Choice" => json!({"enum": p["type"]["choices"].clone()}),
                "Path" => json!({"type": "string", "format": "path"}),
                "Bool" => json!({"type": "boolean"}),
                _ => json!({"type": "string"}),
            };
            let is_arg = p["param_type_name"] == "argument";
            let is_flag = p["is_flag"].as_bool().unwrap_or(false);
            if is_arg && p["nargs"].as_i64() == Some(-1) {
                prop = json!({"type": "array", "items": prop});
            }
            if p["required"].as_bool().unwrap_or(false) {
                required.push(name.clone());
            }
            props.insert(name.clone(), prop);
            let flag = p["opts"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .find(|o| o.starts_with("--"))
                .unwrap_or_default()
                .to_string();
            args.push(pb::ArgMapping {
                pointer: format!("/{name}"),
                style: if is_arg {
                    pb::ArgStyle::Positional
                } else if is_flag {
                    pb::ArgStyle::Flag
                } else {
                    pb::ArgStyle::Option
                } as i32,
                flag: if is_arg { String::new() } else { flag },
                position: if is_arg {
                    position += 1;
                    position - 1
                } else {
                    0
                },
                ..Default::default()
            });
        }
        let binding = pb::CliArgvBinding {
            executable_role: "executable".into(),
            global_args: if root_opts.contains(&"--json".to_string()) {
                vec!["--json".into()]
            } else {
                vec![]
            },
            state_handle_flag: if root_opts.contains(&"--project".to_string()) {
                "--project".into()
            } else {
                String::new()
            },
            state_handle_engine_supplied: true,
            command_path: path.clone(),
            args,
            output: pb::OutputMode::JsonStdout as i32,
            errors: pb::ErrorConvention::CliAnythingJson as i32,
            kill_scope: pb::KillScope::OwnedGroup as i32,
            usage_exit_code: 2,
            ..Default::default()
        };
        let help = node["help"].as_str().unwrap_or_default();
        let mut c = candidate(
            target,
            path.clone(),
            help,
            tt(help, "click-help"),
            schema(props, required),
            binding,
            &sha,
            path.last().is_some_and(|l| l == "repl"),
        );
        if root_opts.contains(&"--dry-run".to_string()) {
            c.capability.dry_run = Some(pb::DryRun {
                supported: true,
                argv: vec!["--dry-run".into()],
                kind: pb::DryRunKind::NoPersist as i32,
                verified: false,
            });
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// cobra-complete@1
// ---------------------------------------------------------------------------

fn completions(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .filter(|l| !l.starts_with(':') && !l.is_empty())
        .map(|l| match l.split_once('\t') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect()
}

pub fn cobra_candidates(target: &pb::Target, raw: &str) -> Vec<Candidate> {
    let sha = digest::sha256_hex(raw.as_bytes());
    let doc: Value = serde_json::from_str(raw).unwrap();
    let calls: Vec<(Vec<String>, String)> = doc["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let argv = c["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_string())
                .collect();
            (argv, c["stdout"].as_str().unwrap().to_string())
        })
        .collect();
    let mut out = Vec::new();
    for (argv, stdout) in calls.iter().filter(|(a, _)| a.last().is_some_and(String::is_empty)) {
        let parent: Vec<String> = argv[2..argv.len() - 1].to_vec();
        for (name, desc) in completions(stdout) {
            let mut path = parent.clone();
            path.push(name);
            let mut flag_argv = vec![argv[0].clone(), "__complete".into()];
            flag_argv.extend(path.iter().cloned());
            flag_argv.push("--".into());
            let flags = calls
                .iter()
                .find(|(a, _)| *a == flag_argv)
                .map(|(_, s)| completions(s))
                .unwrap_or_default();
            let mut props = Map::new();
            let mut args = Vec::new();
            for (flag, _) in flags.iter().filter(|(f, _)| f != "--help") {
                let n = prop_name(flag);
                props.insert(n.clone(), json!({"type": "boolean"}));
                args.push(pb::ArgMapping {
                    pointer: format!("/{n}"),
                    style: pb::ArgStyle::Flag as i32,
                    flag: flag.clone(),
                    ..Default::default()
                });
            }
            let binding = pb::CliArgvBinding {
                executable_role: "executable".into(),
                command_path: path.clone(),
                args,
                output: pb::OutputMode::TextStdout as i32,
                errors: pb::ErrorConvention::ExitCode as i32,
                kill_scope: pb::KillScope::OwnedGroup as i32,
                ..Default::default()
            };
            out.push(candidate(
                target,
                path,
                &desc,
                tt(&desc, "cobra-complete"),
                schema(props, vec![]),
                binding,
                &sha,
                false,
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Targets, sources, records
// ---------------------------------------------------------------------------

pub fn target(id: &str, files: &[(&str, &str)], authorization: &str) -> pb::Target {
    pb::Target {
        id: id.into(),
        kind: pb::TargetKind::CliTool as i32,
        identity: Some(pb::TargetIdentity {
            files: files
                .iter()
                .map(|(role, path)| pb::FileIdentity {
                    role: (*role).into(),
                    realpath: (*path).into(),
                    sha256: digest::sha256_hex(path.as_bytes()),
                    version_banner: String::new(),
                })
                .collect(),
            ..Default::default()
        }),
        authorization_statement: authorization.into(),
        ..Default::default()
    }
}

pub fn git_target() -> pb::Target {
    target(
        "git",
        &[("executable", "/opt/homebrew/Cellar/git/2.48.1/bin/git")],
        "test",
    )
}

pub fn notes_target() -> pb::Target {
    target(
        "fixture-notes",
        &[
            ("executable", "/venv/bin/notes-cli"),
            ("interpreter", "/venv/bin/python3"),
        ],
        "test",
    )
}

pub fn gh_target() -> pb::Target {
    target("gh", &[("executable", "/opt/homebrew/bin/gh")], "test")
}

pub fn source(harvester: &str, uri: &str, raw: &str) -> pb::SourceRef {
    pb::SourceRef {
        harvester: harvester.into(),
        uri: uri.into(),
        sha256: digest::sha256_hex(raw.as_bytes()),
        authority: pb::DiscoveryAuthority::Introspect as i32,
        harvested_at: Some(pbjson_types::Timestamp {
            seconds: 1_793_916_604,
            nanos: 0,
        }),
        action_id: "act_01TESTHARVEST".into(),
    }
}

pub fn cap<'a>(lock: &'a pb::CapabilityLock, id: &str) -> &'a pb::Capability {
    lock.capabilities
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("{id} not in lock"))
}

pub fn state(c: &pb::Capability) -> pb::AdmissionState {
    pb::AdmissionState::try_from(c.admission.as_ref().unwrap().state).unwrap()
}

pub fn reasons(c: &pb::Capability) -> Vec<pb::WithheldReason> {
    c.admission
        .as_ref()
        .unwrap()
        .withheld
        .iter()
        .map(|w| pb::WithheldReason::try_from(w.reason).unwrap())
        .collect()
}

/// A lane record bound to `lock`'s current digests for `cap_id`.
pub fn record(
    lock: &pb::CapabilityLock,
    cap_id: &str,
    lane: pb::Lane,
    status: pb::LaneStatus,
    isolation: pb::Isolation,
    at: i64,
) -> pb::VerificationRecord {
    let c = cap(lock, cap_id);
    pb::VerificationRecord {
        record_id: format!("vr_{}_{}_{at}", lane.as_str_name().to_ascii_lowercase(), cap_id),
        capability_id: cap_id.into(),
        capability_digest: c.capability_digest.clone(),
        lane: lane as i32,
        status: status as i32,
        lock_digest: lock.lock_digest.clone(),
        identity_digest: lock
            .target
            .as_ref()
            .and_then(|t| t.identity.as_ref())
            .map(|i| i.identity_digest.clone())
            .unwrap_or_default(),
        isolation: isolation as i32,
        host: "macos/aarch64".into(),
        engine_version: "test".into(),
        non_claims: vec!["cloud/remote state not observed".into()],
        ran_at: Some(pbjson_types::Timestamp { seconds: at, nanos: 0 }),
        ..Default::default()
    }
}

/// L0 (real) + L1 + L3 PASSED at CONFINED for each id.
pub fn passing_records(lock: &pb::CapabilityLock, ids: &[&str], at: i64) -> Vec<pb::VerificationRecord> {
    let mut out = Vec::new();
    for id in ids {
        let mut l0 = aa_core::lanes::run_l0(lock, cap(lock, id), "test");
        l0.ran_at = Some(pbjson_types::Timestamp { seconds: at, nanos: 0 });
        // run_l0 ids hash the wall-clock ran_at; pin them so snapshots are stable.
        l0.record_id = format!("vr_lane_static_{id}_{at}");
        out.push(l0);
        out.push(record(
            lock,
            id,
            pb::Lane::Parser,
            pb::LaneStatus::Passed,
            pb::Isolation::Confined,
            at,
        ));
        out.push(record(
            lock,
            id,
            pb::Lane::SandboxDiff,
            pb::LaneStatus::Passed,
            pb::Isolation::Confined,
            at,
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// A minimal in-memory AuditSink (api::testing is behind the `testing` feature, which integration tests of
// aa-core itself do not get).
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct TestAudit {
    entries: std::sync::Mutex<Vec<pb::AuditEntry>>,
}

impl TestAudit {
    pub fn new() -> Self {
        Self::default()
    }

    fn append(
        &self,
        principal: &pb::Principal,
        halt_epoch: u64,
        body: pb::audit_entry::Body,
    ) -> aa_core::AaResult<u64> {
        let mut e = self.entries.lock().unwrap();
        let (seq, prev) = e
            .last()
            .map_or((1, String::new()), |l| (l.seq + 1, l.entry_sha256.clone()));
        let mut entry = pb::AuditEntry {
            seq,
            prev_sha256: prev,
            principal: Some(principal.clone()),
            halt_epoch,
            body: Some(body),
            ..Default::default()
        };
        entry.entry_sha256 = digest::self_digest(&entry, |x| x.entry_sha256.clear())?;
        e.push(entry);
        Ok(seq)
    }
}

impl aa_core::api::AuditSink for TestAudit {
    fn append_intent(&self, p: &pb::Principal, h: u64, i: pb::ActionIntent) -> aa_core::AaResult<u64> {
        self.append(p, h, pb::audit_entry::Body::Intent(i))
    }
    fn append_action(&self, p: &pb::Principal, h: u64, r: pb::ActionRecord) -> aa_core::AaResult<u64> {
        self.append(p, h, pb::audit_entry::Body::Action(r))
    }
    fn append_admin(&self, p: &pb::Principal, h: u64, e: pb::AdminEvent) -> aa_core::AaResult<u64> {
        self.append(p, h, pb::audit_entry::Body::Admin(e))
    }
    fn head(&self) -> (u64, String) {
        self.entries
            .lock()
            .unwrap()
            .last()
            .map_or((0, String::new()), |e| (e.seq, e.entry_sha256.clone()))
    }
    fn entry(&self, seq: u64) -> aa_core::AaResult<Option<pb::AuditEntry>> {
        Ok(self.entries.lock().unwrap().iter().find(|e| e.seq == seq).cloned())
    }
}
