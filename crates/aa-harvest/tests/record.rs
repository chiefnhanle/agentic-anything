//! Re-records the committed raw-harvest fixtures from real tools on this machine (opt-in: `AA_RECORD=1`).
//!
//! - `help-text.git.json`: `git {status,log,branch,add,rev-parse} -h` via help-text@1;
//! - `cobra.gh.jsonl`: `gh __complete` crawl of `gh alias` via cobra-complete@1;
//! - `click-info.notes.json`: the notes_cli fixture via click-info@1 (needs `AA_TEST_PYTHON` with click and
//!   `uv pip install --python $AA_TEST_PYTHON crates/aa-harvest/tests/fixtures/notes_cli`).
//! - `click-info.libreoffice.json`: the CLI-Anything LibreOffice harness (needs `AA_TEST_LO_PYTHON`).
//!
//! Every harvest runs through the real harvester code with a private HOME and a scrubbed env.

mod support;

use aa_core::{
    api::{HarvestCtx, Harvester},
    pb,
};
use aa_harvest::{ClickInfoHarvester, CobraCompleteHarvester, HelpTextHarvester};
use serde_json::json;
use support::*;

fn record(name: &str, bytes: &[u8]) {
    let path = fixture_path(&format!("raw/{name}"));
    std::fs::write(&path, bytes).unwrap();
    eprintln!("recorded {}", path.display());
}

fn harvest(h: &dyn Harvester, target: &pb::Target, options: serde_json::Value) -> Vec<u8> {
    let scratch = tempfile::tempdir().unwrap();
    let ctx = HarvestCtx {
        target,
        options: &options,
        runner: &StdRunner,
        scratch_parent: scratch.path(),
        allow_network: false,
    };
    let mut raws = block_on(h.harvest(ctx)).unwrap();
    assert_eq!(raws.len(), 1);
    raws.remove(0).bytes
}

fn resolve(bin: &str) -> Option<String> {
    let p = which::which(bin).ok()?.canonicalize().ok()?;
    Some(p.display().to_string())
}

#[test]
fn record_fixtures() {
    if live_env(
        "AA_RECORD",
        "set AA_RECORD=1 to re-record tests/fixtures/raw from real tools",
    )
    .is_none()
    {
        return;
    }
    if let Some(git) = resolve("git") {
        let t = target("git", pb::TargetKind::CliTool, vec![file("executable", &git)]);
        let bytes = harvest(
            &HelpTextHarvester,
            &t,
            json!({"commands": ["status", "log", "branch", "add", "rev-parse"], "help_argv": ["{command}", "-h"]}),
        );
        record("help-text.git.json", &bytes);
    }
    if let Some(gh) = resolve("gh") {
        let t = target("gh", pb::TargetKind::CliTool, vec![file("executable", &gh)]);
        let bytes = harvest(&CobraCompleteHarvester, &t, json!({"commands": ["alias"]}));
        record("cobra.gh.jsonl", &bytes);
    }
    if let Some(py) = std::env::var("AA_TEST_PYTHON").ok().filter(|s| !s.is_empty()) {
        let t = target("notes", pb::TargetKind::CliTool, vec![file("interpreter", &py)]);
        let bytes = harvest(&ClickInfoHarvester, &t, json!({"entry_point": "notes"}));
        record("click-info.notes.json", &bytes);
    }
    if let Some(py) = std::env::var("AA_TEST_LO_PYTHON").ok().filter(|s| !s.is_empty()) {
        let t = target(
            "cli-anything.libreoffice",
            pb::TargetKind::CliAnythingHarness,
            vec![file("interpreter", &py)],
        );
        let bytes = harvest(
            &ClickInfoHarvester,
            &t,
            json!({"entry_point": "cli-anything-libreoffice"}),
        );
        record("click-info.libreoffice.json", &bytes);
    }
}
