//! Acceptance 1: pure replay of recorded raw harvests (no Python, git or gh), plus harvest() round-trips
//! through `aa-core::api::testing::FakeRunner` that must reproduce the committed bytes.

mod support;

use aa_core::{
    api::{
        HarvestCtx, Harvester,
        testing::{FakeRunner, ok_output},
    },
    pb,
};
use aa_harvest::{ClickInfoHarvester, CobraCompleteHarvester, HelpTextHarvester, builtin_harvesters};
use serde_json::{Value, json};
use support::*;

fn notes_target() -> pb::Target {
    target(
        "notes",
        pb::TargetKind::CliTool,
        vec![file("interpreter", "/venv/bin/python")],
    )
}

fn gh_target() -> pb::Target {
    target(
        "gh",
        pb::TargetKind::CliTool,
        vec![file("executable", "/opt/homebrew/Cellar/gh/2.102.0/bin/gh")],
    )
}

fn git_target() -> pb::Target {
    target(
        "git",
        pb::TargetKind::CliTool,
        vec![file("executable", "/opt/homebrew/Cellar/git/2.48.1/bin/git")],
    )
}

#[test]
fn builtin_harvesters_ids_versions_authorities() {
    let hs = builtin_harvesters();
    let got: Vec<(String, u32, pb::DiscoveryAuthority)> = hs
        .iter()
        .map(|h| (h.id().to_owned(), h.version(), h.authority()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("cli-anything-registry".into(), 1, pb::DiscoveryAuthority::Static),
            ("click-info".into(), 1, pb::DiscoveryAuthority::Introspect),
            ("cobra-complete".into(), 1, pb::DiscoveryAuthority::Introspect),
            ("help-text".into(), 1, pb::DiscoveryAuthority::Introspect),
        ]
    );
}

#[test]
fn click_info_notes_seven_leaves_snapshot() {
    let raw = raw_fixture(
        "click-info@1",
        "click-info.notes.json",
        pb::DiscoveryAuthority::Introspect,
    );
    let t = notes_target();
    let cands = ClickInfoHarvester.candidates(&t, &[raw]).unwrap();
    let paths: Vec<String> = cands.iter().map(|c| c.native_path.join(" ")).collect();
    assert_eq!(
        paths,
        [
            "export",
            "notes add",
            "notes lenient",
            "notes list",
            "notes remove",
            "notes stats",
            "repl"
        ]
    );
    for c in &cands {
        assert_eq!(c.interactive, c.native_path == ["repl"], "{:?}", c.native_path);
        assert!(
            c.capability.effects.is_none() && c.capability.admission.is_none(),
            "no effects/admission"
        );
        assert!(c.native_signals.is_empty(), "click-info carries types only");
        let s = schema(c);
        assert_eq!(
            s["additionalProperties"],
            json!(false),
            "closed schema for {}",
            c.capability.id
        );
        let b = cli(c);
        assert_eq!(b.executable_role, "interpreter");
        assert_eq!(b.interpreter_args, ["-I", "-m", "notes_cli"]);
        assert_eq!(b.global_args, ["--json"]);
        assert_eq!(b.state_handle_flag, "--project");
        assert!(b.state_handle_engine_supplied);
        assert_eq!(b.errors, pb::ErrorConvention::CliAnythingJson as i32);
        assert_eq!(b.output, pb::OutputMode::JsonStdout as i32);
        assert_eq!(b.usage_exit_code, 2);
        let dry = c.capability.dry_run.as_ref().unwrap();
        assert_eq!(dry.argv, ["--dry-run"]);
        assert_eq!(dry.kind, pb::DryRunKind::NoPersist as i32);
    }
    let ids: Vec<&str> = cands.iter().map(|c| c.capability.id.as_str()).collect();
    assert!(ids.contains(&"notes.notes.add") && ids.contains(&"notes.repl"));
    let snap: Vec<Value> = cands.iter().map(candidate_json).collect();
    insta::assert_json_snapshot!("click_info_notes_candidates", snap);
}

#[test]
fn click_params_on_notes_leaves() {
    // Acceptance 2 over real to_info_dict output: Choice->enum, IntRange->bounds, multiple->array,
    // flag->boolean (none on leaves here), nargs=-1->positional array, --json/--project never exposed.
    let raw = raw_fixture(
        "click-info@1",
        "click-info.notes.json",
        pb::DiscoveryAuthority::Introspect,
    );
    let cands = ClickInfoHarvester.candidates(&notes_target(), &[raw]).unwrap();
    let by = |p: &str| cands.iter().find(|c| c.native_path.join(" ") == p).unwrap();
    let export = schema(by("export"));
    assert_eq!(export["properties"]["fmt"]["enum"], json!(["txt", "json"]));
    assert_eq!(export["properties"]["output_path"]["format"], json!("path"));
    assert_eq!(export["required"], json!(["output_path"]));
    let scopes: Vec<(String, i32)> = by("export")
        .capability
        .scopes
        .iter()
        .map(|s| (s.pointer.clone(), s.kind))
        .collect();
    assert!(scopes.contains(&("/output_path".into(), pb::ScopeKind::WritePath as i32)));
    // The CLI-Anything --project handle is engine-supplied: no agent input pointer, so no pointer scope.
    assert!(!scopes.iter().any(|(_, k)| *k == pb::ScopeKind::StateHandle as i32));
    let binding = match by("export").capability.binding.as_ref().and_then(|b| b.kind.as_ref()) {
        Some(pb::binding::Kind::Cli(c)) => c.clone(),
        other => panic!("not a cli binding: {other:?}"),
    };
    assert_eq!(binding.state_handle_flag, "--project");
    assert!(binding.state_handle_engine_supplied);
    assert!(binding.state_handle_pointer.is_empty());
    let list = schema(by("notes list"));
    assert_eq!(
        list["properties"]["limit"],
        json!({"type": "integer", "minimum": 1, "maximum": 1000})
    );
    let add = schema(by("notes add"));
    assert_eq!(
        add["properties"]["tags"],
        json!({"type": "array", "items": {"type": "string"}})
    );
    assert_eq!(add["required"], json!(["text"]));
    let lenient = schema(by("notes lenient"));
    assert_eq!(
        lenient["properties"]["words"],
        json!({"type": "array", "items": {"type": "string"}})
    );
    let lenient_args = &cli(by("notes lenient")).args;
    assert_eq!(lenient_args[0].style, pb::ArgStyle::Positional as i32);
    let remove = schema(by("notes remove"));
    assert_eq!(remove["properties"]["index"], json!({"type": "integer"}));
    for c in &cands {
        let s = schema(c);
        for hidden in ["json", "as_json", "project", "project_path", "dry_run", "help"] {
            assert!(
                s["properties"].get(hidden).is_none(),
                "{hidden} exposed on {}",
                c.capability.id
            );
        }
        assert!(cli(c).args.iter().all(|a| a.flag != "--json" && a.flag != "--project"));
    }
}

#[test]
fn click_info_harvest_round_trip_through_fake_runner() {
    let fixture = std::fs::read(fixture_path("raw/click-info.notes.json")).unwrap();
    let fixture_text = String::from_utf8(fixture.clone()).unwrap();
    let runner = FakeRunner::new().on(&["/venv/bin/python", "-I"], ok_output(&fixture_text));
    let scratch = tempfile::tempdir().unwrap();
    let t = notes_target();
    let options = json!({"entry_point": "notes"});
    let raws = block_on(ClickInfoHarvester.harvest(HarvestCtx {
        target: &t,
        options: &options,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap();
    assert_eq!(raws.len(), 1);
    assert_eq!(raws[0].bytes, fixture);
    assert_eq!(raws[0].harvester, "click-info@1");
    assert_eq!(raws[0].authority, pb::DiscoveryAuthority::Introspect as i32);
    let calls = runner.calls();
    assert_eq!(calls.len(), 1);
    let spec = &calls[0];
    assert_eq!(spec.argv[0], "/venv/bin/python");
    assert_eq!(spec.argv[1], "-I");
    assert!(spec.argv[2].ends_with("/aa_click_info.py"));
    assert_eq!(spec.argv[3], "notes");
    assert_eq!(spec.deadline, std::time::Duration::from_secs(20));
    assert_eq!(spec.max_stdout, 4 * 1024 * 1024);
    assert!(spec.stdin.is_none());
    let conf = spec.confinement.as_ref().expect("confined");
    assert!(conf.deny_network);
    assert_eq!(conf.exec_allow, [std::path::PathBuf::from("/venv/bin/python")]);
    assert!(
        spec.argv[2].starts_with(&conf.writable[0].display().to_string()),
        "shim lives in the private root"
    );
    assert!(spec.env["HOME"].starts_with(&conf.writable[0].display().to_string()));
    assert!(!spec.env.contains_key("PYTHONPATH"));
    // The private root is gone after the harvest.
    assert!(!std::path::Path::new(&spec.argv[2]).exists());
}

#[test]
fn click_info_shim_errors_are_typed() {
    let runner = FakeRunner::new().on(&["/venv/bin/python"], {
        let mut o = ok_output("{\"error\":\"ambiguous_root\",\"protocol\":\"aa-click-info/1\"}\n");
        o.exit_code = Some(65);
        o
    });
    let scratch = tempfile::tempdir().unwrap();
    let t = notes_target();
    let options = json!({"entry_point": "notes"});
    let err = block_on(ClickInfoHarvester.harvest(HarvestCtx {
        target: &t,
        options: &options,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap_err();
    assert_eq!(err.code, pb::ErrorCode::UpstreamError);
    assert!(err.message.contains("ambiguous_root"), "{err:?}");
    // No authorization statement: refused before anything runs.
    let mut unauthorized = notes_target();
    unauthorized.authorization_statement.clear();
    let runner = FakeRunner::new();
    let err = block_on(ClickInfoHarvester.harvest(HarvestCtx {
        target: &unauthorized,
        options: &options,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap_err();
    assert_eq!(err.code, pb::ErrorCode::PolicyDenied);
    assert!(runner.calls().is_empty());
}

#[test]
fn cobra_gh_alias_candidates() {
    let raw = raw_fixture("cobra-complete@1", "cobra.gh.jsonl", pb::DiscoveryAuthority::Introspect);
    let cands = CobraCompleteHarvester.candidates(&gh_target(), &[raw]).unwrap();
    let ids: Vec<&str> = cands.iter().map(|c| c.capability.id.as_str()).collect();
    assert!(ids.contains(&"gh.alias.list"), "{ids:?}");
    assert!(ids.contains(&"gh.alias.set"), "{ids:?}");
    assert!(
        !ids.iter().any(|i| *i == "gh.alias" || i.starts_with("gh.help")),
        "{ids:?}"
    );
    let set = cands.iter().find(|c| c.capability.id == "gh.alias.set").unwrap();
    let s = schema(set);
    assert_eq!(s["properties"]["clobber"], json!({"type": "boolean"}));
    assert_eq!(s["properties"]["shell"], json!({"type": "boolean"}));
    assert_eq!(s["required"], json!(["alias", "expansion"]));
    assert!(s["properties"].get("help").is_none());
    let b = cli(set);
    assert_eq!(b.command_path, ["alias", "set"]);
    assert_eq!(b.usage_exit_code, 1);
    assert_eq!(b.usage_stderr_marker, "unknown flag");
    assert_eq!(b.executable_role, "executable");
    let clobber = b.args.iter().find(|a| a.pointer == "/clobber").unwrap();
    assert_eq!(
        (clobber.style, clobber.flag.as_str()),
        (pb::ArgStyle::Flag as i32, "--clobber")
    );
    let positions: Vec<(String, u32)> = b
        .args
        .iter()
        .filter(|a| a.style == pb::ArgStyle::Positional as i32)
        .map(|a| (a.pointer.clone(), a.position))
        .collect();
    assert_eq!(positions, [("/alias".to_owned(), 0), ("/expansion".to_owned(), 1)]);
    let title = set.capability.title.as_ref().unwrap();
    assert_eq!(
        title.origin,
        pb::ContentOrigin::Target as i32,
        "target text stays labelled"
    );
    let snap: Vec<Value> = cands.iter().map(candidate_json).collect();
    insta::assert_json_snapshot!("cobra_gh_alias_candidates", snap);
}

/// Scripts a FakeRunner with every recorded call of a JSONL/JSON fixture, keyed by the exact argv.
fn scripted(exe: &str, calls: impl Iterator<Item = (Vec<String>, i32, String, String)>) -> FakeRunner {
    let mut r = FakeRunner::new();
    for (args, exit, stdout, stderr) in calls {
        let mut argv = vec![exe.to_owned()];
        argv.extend(args);
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let mut out = ok_output(&stdout);
        out.exit_code = Some(exit);
        out.stderr_excerpt = stderr;
        r = r.on(&refs, out);
    }
    r
}

#[test]
fn cobra_harvest_round_trip_through_fake_runner() {
    let fixture = std::fs::read_to_string(fixture_path("raw/cobra.gh.jsonl")).unwrap();
    let exe = "/opt/homebrew/Cellar/gh/2.102.0/bin/gh";
    let calls = fixture.lines().filter_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        let args = v
            .get("args")?
            .as_array()?
            .iter()
            .map(|a| a.as_str().unwrap().to_owned())
            .collect();
        Some((
            args,
            v["exit_code"].as_i64()? as i32,
            v["stdout"].as_str()?.to_owned(),
            v["stderr"].as_str()?.to_owned(),
        ))
    });
    let runner = scripted(exe, calls);
    let scratch = tempfile::tempdir().unwrap();
    let t = gh_target();
    let options = json!({"commands": ["alias"]});
    let raws = block_on(CobraCompleteHarvester.harvest(HarvestCtx {
        target: &t,
        options: &options,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap();
    assert_eq!(String::from_utf8(raws[0].bytes.clone()).unwrap(), fixture);
    assert_eq!(runner.calls().len(), 14);
    // max_calls only narrows: 3 calls stop the crawl and mark it truncated.
    let options = json!({"commands": ["alias"], "max_calls": 3});
    let raws = block_on(CobraCompleteHarvester.harvest(HarvestCtx {
        target: &t,
        options: &options,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap();
    let text = String::from_utf8(raws[0].bytes.clone()).unwrap();
    assert!(text.trim_end().ends_with("{\"calls\":3,\"truncated\":true}"), "{text}");
}

#[test]
fn help_text_git_status_schema() {
    let raw = raw_fixture("help-text@1", "help-text.git.json", pb::DiscoveryAuthority::Introspect);
    let cands = HelpTextHarvester.candidates(&git_target(), &[raw]).unwrap();
    let ids: Vec<&str> = cands.iter().map(|c| c.capability.id.as_str()).collect();
    assert_eq!(ids, ["git.add", "git.branch", "git.log", "git.rev-parse", "git.status"]);
    let status = cands.iter().find(|c| c.capability.id == "git.status").unwrap();
    let s = schema(status);
    let mut modes: Vec<String> = s["properties"]["untracked_files"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    modes.sort();
    assert_eq!(modes, ["all", "no", "normal"]);
    assert_eq!(s["properties"]["short"], json!({"type": "boolean"}));
    assert_eq!(
        s["properties"]["pathspec"],
        json!({"type": "array", "items": {"type": "string"}})
    );
    assert_eq!(s["additionalProperties"], json!(false));
    let b = cli(status);
    let short = b.args.iter().find(|a| a.pointer == "/short").unwrap();
    assert_eq!(short.style, pb::ArgStyle::NegatableFlag as i32);
    assert_eq!(short.flag, "--short");
    let uf = b.args.iter().find(|a| a.pointer == "/untracked_files").unwrap();
    assert_eq!(
        (uf.style, uf.flag.as_str()),
        (pb::ArgStyle::Option as i32, "--untracked-files")
    );
    assert_eq!(b.usage_exit_code, 129);
    assert_eq!(b.command_path, ["status"]);
    let add = schema(cands.iter().find(|c| c.capability.id == "git.add").unwrap());
    assert_eq!(add["properties"]["pathspec"]["minItems"], json!(1));
    assert_eq!(add["required"], json!(["pathspec"]));
    let snap: Vec<Value> = cands.iter().map(candidate_json).collect();
    insta::assert_json_snapshot!("help_text_git_candidates", snap);
}

#[test]
fn help_text_harvest_round_trip_through_fake_runner() {
    let fixture = std::fs::read_to_string(fixture_path("raw/help-text.git.json")).unwrap();
    let doc: Value = serde_json::from_str(&fixture).unwrap();
    let exe = doc["executable"].as_str().unwrap();
    let calls = doc["calls"].as_array().unwrap().iter().map(|c| {
        (
            c["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_owned())
                .collect(),
            c["exit_code"].as_i64().unwrap() as i32,
            c["stdout"].as_str().unwrap().to_owned(),
            c["stderr"].as_str().unwrap().to_owned(),
        )
    });
    let runner = scripted(exe, calls);
    let scratch = tempfile::tempdir().unwrap();
    let t = git_target();
    let options =
        json!({"commands": ["status", "log", "branch", "add", "rev-parse"], "help_argv": ["{command}", "-h"]});
    let raws = block_on(HelpTextHarvester.harvest(HarvestCtx {
        target: &t,
        options: &options,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap();
    assert_eq!(String::from_utf8(raws[0].bytes.clone()).unwrap(), fixture);
    let calls = runner.calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[0].argv[1..], ["status", "-h"]);
    assert_eq!(calls[0].env["GIT_CONFIG_NOSYSTEM"], "1");
    // Options cannot smuggle flags in as commands.
    let bad = json!({"commands": ["--exec-path=/tmp"]});
    let err = block_on(HelpTextHarvester.harvest(HarvestCtx {
        target: &t,
        options: &bad,
        runner: &runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap_err();
    assert_eq!(err.code, pb::ErrorCode::InvalidRequest);
}

#[test]
fn applies_rules() {
    let none = Value::Null;
    assert!(ClickInfoHarvester.applies(&notes_target(), &json!({"entry_point": "notes"})));
    assert!(!ClickInfoHarvester.applies(&notes_target(), &none));
    assert!(!ClickInfoHarvester.applies(&git_target(), &json!({"entry_point": "notes"})));
    assert!(CobraCompleteHarvester.applies(&gh_target(), &none));
    // A python_console_script target (script + shebang interpreter): click-info by default, never cobra.
    let script = target(
        "notes",
        pb::TargetKind::CliTool,
        vec![
            file("executable", "/venv/bin/notes"),
            file("interpreter", "/venv/bin/python"),
        ],
    );
    assert!(ClickInfoHarvester.applies(&script, &none));
    assert!(!CobraCompleteHarvester.applies(&script, &none));
    assert!(HelpTextHarvester.applies(&git_target(), &json!({"commands": ["status"]})));
    assert!(!HelpTextHarvester.applies(&git_target(), &none));
    let reg = aa_harvest::CliAnythingRegistryHarvester;
    let lo = target("cli-anything.libreoffice", pb::TargetKind::CliAnythingHarness, vec![]);
    assert!(reg.applies(&lo, &json!({"repo": "/x"})));
    assert!(!reg.applies(&git_target(), &json!({"repo": "/x"})));
    assert!(reg.candidates(&lo, &[]).unwrap().is_empty());
}
