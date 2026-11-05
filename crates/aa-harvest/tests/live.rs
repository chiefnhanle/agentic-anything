//! Acceptance 4 and 5: live harvests, gated on environment variables. A missing variable prints a skip
//! reason; a skip never counts as a pass.
//!
//! - `AA_TEST_PYTHON`: an interpreter with click and the notes_cli fixture installed
//!   (`uv pip install --python $AA_TEST_PYTHON crates/aa-harvest/tests/fixtures/notes_cli`).
//! - `AA_TEST_CLI_ANYTHING=/Users/nhanclie/CLI-Anything` and `AA_TEST_LO_PYTHON=<venv>/bin/python` (the
//!   LibreOffice harness installed by the operator) for the CLI-Anything LibreOffice test.

mod support;

use aa_core::{
    api::{
        HarvestCtx, Harvester, RawHarvest,
        testing::{FakeRunner, ok_output},
    },
    pb,
};
use aa_harvest::{ClickInfoHarvester, cli_anything};
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;
use support::*;

fn harvest_click(runner: &dyn aa_core::api::ProcessRunner, t: &pb::Target, entry_point: &str) -> Vec<RawHarvest> {
    let scratch = tempfile::tempdir().unwrap();
    let options = json!({"entry_point": entry_point});
    block_on(ClickInfoHarvester.harvest(HarvestCtx {
        target: t,
        options: &options,
        runner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap_or_else(|e| panic!("click-info harvest of {entry_point} failed: {e:?}"))
}

fn without_click_version(bytes: &[u8]) -> String {
    let re = Regex::new(r#""click_version":\s*"[^"]*""#).unwrap();
    re.replace_all(std::str::from_utf8(bytes).unwrap(), r#""click_version": "<any>""#)
        .into_owned()
}

#[test]
fn live_click_info_notes_matches_committed_fixture() {
    let Some(python) = live_env(
        "AA_TEST_PYTHON",
        "needs an interpreter with click + notes_cli installed",
    ) else {
        return;
    };
    let t = target("notes", pb::TargetKind::CliTool, vec![file("interpreter", &python)]);
    let raws = harvest_click(&StdRunner, &t, "notes");
    assert_eq!(raws.len(), 1);
    // Re-record to a temp file, then compare with the committed fixture modulo click_version.
    let tmp = tempfile::tempdir().unwrap();
    let rerecorded = tmp.path().join("click-info.notes.json");
    std::fs::write(&rerecorded, &raws[0].bytes).unwrap();
    let live = std::fs::read(&rerecorded).unwrap();
    let committed = std::fs::read(fixture_path("raw/click-info.notes.json")).unwrap();
    assert_eq!(
        without_click_version(&live),
        without_click_version(&committed),
        "re-record with: AA_RECORD=1 AA_TEST_PYTHON=… cargo test -p aa-harvest --test record"
    );
    // The same bytes replayed through FakeRunner give the same raw artifact (harvest is deterministic).
    let fake = FakeRunner::new().on(&[python.as_str()], ok_output(std::str::from_utf8(&live).unwrap()));
    let replayed = harvest_click(&fake, &t, "notes");
    assert_eq!(replayed[0].bytes, live);
    let doc: Value = serde_json::from_slice(&live).unwrap();
    assert_eq!(
        doc["root_selection"], "unique_root_group",
        "main() wrapper: the root group must be found"
    );
}

#[test]
fn live_cli_anything_libreoffice_import_and_harvest() {
    let Some(repo) = live_env("AA_TEST_CLI_ANYTHING", "needs a local CLI-Anything checkout") else {
        return;
    };
    let Some(python) = live_env(
        "AA_TEST_LO_PYTHON",
        "needs a venv with the LibreOffice harness installed",
    ) else {
        return;
    };
    let repo = Path::new(&repo);
    let entries = cli_anything::read_registry(repo).unwrap();
    assert!(entries.len() >= 79, "registry.json has {} entries", entries.len());
    let imported =
        cli_anything::import_harness(repo, "libreoffice", Path::new(&python), "Owner workstation; live test.").unwrap();
    assert_eq!(imported.target.id, "cli-anything.libreoffice");
    assert!(imported.target.preconditions.iter().any(|p| p.id == "binary:soffice"));
    assert!(imported.install_cmd_flags.contains(&"unpinned-git-url".to_owned()));
    let raws = harvest_click(&StdRunner, &imported.target, "cli-anything-libreoffice");
    let doc: Value = serde_json::from_slice(&raws[0].bytes).unwrap();
    assert_eq!(
        doc["root_selection"], "unique_root_group",
        "root selection must see through main()"
    );
    let cands = ClickInfoHarvester.candidates(&imported.target, &raws).unwrap();
    assert_eq!(
        cands.len(),
        40,
        "leaves: {:?}",
        cands.iter().map(|c| c.native_path.join(" ")).collect::<Vec<_>>()
    );
    assert!(cands.iter().any(|c| c.native_path == ["repl"] && c.interactive));
    let list = cands
        .iter()
        .find(|c| c.native_path == ["writer", "list"])
        .expect("writer list");
    let b = cli(list);
    assert_eq!(b.interpreter_args, ["-I", "-m", "cli_anything.libreoffice"]);
    assert_eq!(b.global_args, ["--json"]);
    assert_eq!(b.state_handle_flag, "--project");
}

#[test]
fn live_shim_errors_and_root_selection() {
    let Some(python) = live_env("AA_TEST_PYTHON", "needs an interpreter with click") else {
        return;
    };
    // An entry point that does not exist: typed error, the shim's JSON payload kept in details.
    let t = target("notes", pb::TargetKind::CliTool, vec![file("interpreter", &python)]);
    let scratch = tempfile::tempdir().unwrap();
    let options = json!({"entry_point": "aa-no-such-entry-point"});
    let err = block_on(ClickInfoHarvester.harvest(HarvestCtx {
        target: &t,
        options: &options,
        runner: &StdRunner,
        scratch_parent: scratch.path(),
        allow_network: false,
    }))
    .unwrap_err();
    assert_eq!(err.code, pb::ErrorCode::UpstreamError);
    assert!(err.message.contains("entry_point_not_found"), "{err:?}");
    // Root selection on synthetic modules (the shim's own function, imported from a private copy).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("aa_click_info.py"), aa_harvest::CLICK_INFO_SHIM).unwrap();
    let script = r#"
import sys, types, click
sys.path.insert(0, sys.argv[1])
import aa_click_info as s
def mod(**kw):
    m = types.ModuleType("m"); m.__dict__.update(kw); return m
root = click.Group("root"); child = click.Group("child"); root.add_command(child)
leaf = click.Command("leaf")
def main(): root()
r, how = s.select_root(click, main, mod(root=root, child=child, main=main))
assert r is root and how == "unique_root_group", (r, how)
r, how = s.select_root(click, leaf, mod())
assert r is leaf and how == "entry_point"
other = click.Group("other")
r, how = s.select_root(click, main, mod(root=root, child=child, other=other))
assert r is None and "roots=2" in how, how
assert s.sanitize({"a": (1, 2), "b": print, "c": float("nan")}) == {"a": [1, 2], "b": {"$callable": True}, "c": None}
print("ok")
"#;
    let out = std::process::Command::new(&python)
        .arg("-I")
        .arg("-c")
        .arg(script)
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success() && out.stdout == b"ok\n",
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
