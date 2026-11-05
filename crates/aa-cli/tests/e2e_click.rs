//! The hermetic Click fixture (`notes_cli`, shaped like a CLI-Anything harness) end to end.
//!
//! Needs `AA_TEST_PYTHON` pointing at a venv interpreter that has the fixture installed:
//!   uv venv .venv && uv pip install --python .venv/bin/python crates/aa-harvest/tests/fixtures/notes_cli
//!   AA_TEST_PYTHON=$PWD/.venv/bin/python cargo test -p aa-cli --test e2e_click
//! Without it the test prints a skip reason (a skip never counts as coverage).

mod common;

use common::{Engine, admission, repo_root, skip, withheld_reasons};
use serde_json::Value;
use std::path::PathBuf;

/// The fixture's console script next to `$AA_TEST_PYTHON` (`notes-cli`, or `notes` as the fixture names it).
fn notes_script() -> Result<PathBuf, String> {
    let py = std::env::var_os("AA_TEST_PYTHON")
        .filter(|v| !v.is_empty())
        .ok_or("AA_TEST_PYTHON is not set (a venv python with crates/aa-harvest/tests/fixtures/notes_cli installed)")?;
    let bin = PathBuf::from(py)
        .parent()
        .map(PathBuf::from)
        .ok_or("AA_TEST_PYTHON has no parent dir")?;
    ["notes-cli", "notes"]
        .iter()
        .map(|n| bin.join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "no notes-cli/notes console script in {}; install it: uv pip install --python {}/python \
             crates/aa-harvest/tests/fixtures/notes_cli",
                bin.display(),
                bin.display()
            )
        })
}

fn reasons_include(cap: &Value, reason: &str) -> bool {
    withheld_reasons(cap).iter().any(|r| r == reason)
}

#[test]
#[ignore = "needs integration"]
fn click_fixture_end_to_end() {
    let script = match notes_script() {
        Ok(s) => s,
        Err(reason) => return skip("click_fixture_end_to_end", &reason),
    };
    let e = Engine::start();
    let overlay = repo_root().join("targets/fixture-notes/target.aa.yaml");
    e.ok(&[
        "target",
        "add",
        "fixture-notes",
        "--python-script",
        script.to_str().unwrap(),
        "--overlay",
        overlay.to_str().unwrap(),
        "--authorization",
        "hermetic test fixture",
    ]);
    let d = e.ok(&["discover", "fixture-notes"]);
    assert!(d["counts"]["candidates"].as_u64().unwrap_or(0) >= 7, "{d}");
    e.ok(&["verify", "fixture-notes"]);

    let caps = e.capabilities("fixture-notes");
    let get = |id: &str| {
        caps.get(id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} missing: {:?}", caps.keys()))
    };

    let list = get("fixture-notes.notes.list");
    assert_eq!(admission(&list), "ADMISSION_STATE_SERVABLE", "{list}");
    let stats = get("fixture-notes.notes.stats");
    assert_ne!(admission(&stats), "ADMISSION_STATE_SERVABLE");
    assert!(reasons_include(&stats, "WITHHELD_REASON_SIDE_EFFECT"), "{stats}");
    let lenient = get("fixture-notes.notes.lenient");
    assert_ne!(admission(&lenient), "ADMISSION_STATE_SERVABLE");
    assert!(
        reasons_include(&lenient, "WITHHELD_REASON_PARSER_MISMATCH"),
        "{lenient}"
    );
    let repl = get("fixture-notes.repl");
    assert_ne!(admission(&repl), "ADMISSION_STATE_SERVABLE");
    assert!(reasons_include(&repl, "WITHHELD_REASON_INTERACTIVE"), "{repl}");
    let add = get("fixture-notes.notes.add");
    assert_ne!(admission(&add), "ADMISSION_STATE_SERVABLE");
    assert!(reasons_include(&add, "WITHHELD_REASON_NOT_VERIFIED"), "{add}");

    // The state handle is engine-supplied: --project is never an agent input.
    let desc = e.ok(&["describe", "fixture-notes.notes.list"]);
    let cli = &desc["capability"]["binding"]["cli"];
    assert_eq!(cli["stateHandleFlag"], "--project", "{desc}");
    assert_eq!(cli["stateHandleEngineSupplied"], true, "{desc}");
    let schema: Value = serde_json::from_str(desc["capability"]["inputSchemaJson"].as_str().unwrap_or("{}")).unwrap();
    assert!(
        schema["properties"].get("project").is_none(),
        "project exposed: {schema}"
    );

    // Invoking it runs with the engine-supplied project in the (ephemeral) session workspace.
    let r = e.aa_env(&["x", "fixture-notes.notes.list", "--input-json", "{}"]);
    assert_eq!(r.code, 0, "{r}\n{}", e.log_text());
    assert_eq!(r.data()["count"], 0, "{r}");
    let action_id = r.json()["meta"]["action_id"].as_str().unwrap_or_default().to_string();
    let a = e.ok(&["audit", "--action", &action_id]);
    let record = a["entries"]
        .as_array()
        .and_then(|v| v.iter().find_map(|x| x.get("action").cloned()))
        .unwrap_or_else(|| panic!("no ActionRecord for {action_id}: {a}"));
    assert!(record["sessionId"].as_str().is_some_and(|s| !s.is_empty()), "{record}");
    assert_eq!(record["outcome"], "OUTCOME_COMPLETED", "{record}");
}
