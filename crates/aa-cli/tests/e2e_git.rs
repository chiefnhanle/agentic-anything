//! The slice's headline e2e test: real `git`, a real `aad` process over UDS, and the real `aa` binary.
//!
//! `cargo test -p aa-cli --test e2e_git -- --nocapture`

mod common;

use common::{Engine, admission, as_u64, make_repo, repo_root, withheld_reasons};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn overlay_path() -> String {
    repo_root()
        .join("targets/git/target.aa.yaml")
        .to_string_lossy()
        .into_owned()
}

fn add_git(e: &Engine, overlay: &str) {
    e.ok(&[
        "target",
        "add",
        "git",
        "--exec",
        "git",
        "--overlay",
        overlay,
        "--authorization",
        "test",
    ]);
}

fn input(v: Value) -> String {
    serde_json::to_string(&v).unwrap()
}

fn has_reason(cap: &Value, reason: &str) -> bool {
    withheld_reasons(cap).iter().any(|r| r == reason)
}

fn cap<'a>(caps: &'a BTreeMap<String, Value>, id: &str) -> &'a Value {
    caps.get(id)
        .unwrap_or_else(|| panic!("{id} missing from {:?}", caps.keys().collect::<Vec<_>>()))
}

#[test]
fn git_headline() {
    let e = Engine::start();
    let repo = e.root.join("repo");
    make_repo(&repo);
    let repo_s = repo.to_string_lossy().into_owned();

    // 1. Register.
    add_git(&e, &overlay_path());

    // 2. Discover: native help-text candidates plus the overlay's mode splits.
    let d = e.ok(&["discover", "git"]);
    let caps: BTreeMap<String, Value> = d["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .map(|c| (c["id"].as_str().unwrap().to_string(), c.clone()))
        .collect();
    for id in [
        "git.status",
        "git.log",
        "git.add",
        "git.branch.list",
        "git.branch.delete",
        "git.branch",
    ] {
        cap(&caps, id);
    }
    let branch = cap(&caps, "git.branch");
    assert_ne!(admission(branch), "ADMISSION_STATE_SERVABLE");
    assert!(has_reason(branch, "WITHHELD_REASON_MIXED_AUTHORITY"), "{branch}");
    assert!(
        d["counts"]["withheldByReason"]["MIXED_AUTHORITY"].as_u64().unwrap_or(0) >= 1,
        "{d}"
    );

    // 3. Verify: READs become servable; WRITE stays withheld until L4 (slice 2).
    e.ok(&["verify", "git"]);
    let caps = e.capabilities("git");
    for id in ["git.status", "git.log", "git.branch.list"] {
        let c = cap(&caps, id);
        assert_eq!(admission(c), "ADMISSION_STATE_SERVABLE", "{id}: {c}");
        assert_eq!(c["level"], "EFFECT_LEVEL_READ", "{id}: {c}");
    }
    let status = e.ok(&["status"]);
    if status["confinementAvailable"] == true {
        let desc = e.ok(&["describe", "git.status"]);
        assert_eq!(
            desc["capability"]["admission"]["requiredRuntimeIsolation"], "ISOLATION_CONFINED",
            "{desc}"
        );
    }
    let add = cap(&caps, "git.add");
    assert_ne!(admission(add), "ADMISSION_STATE_SERVABLE");
    assert!(has_reason(add, "WITHHELD_REASON_NOT_VERIFIED"), "{add}");
    assert_ne!(admission(cap(&caps, "git.branch.delete")), "ADMISSION_STATE_SERVABLE");
    // The overlay's binding.args are git.log's whole input schema, including the parameters it adds (§7.3).
    let desc = e.ok(&["describe", "git.log"]);
    let schema: Value = serde_json::from_str(desc["capability"]["inputSchemaJson"].as_str().unwrap_or("{}")).unwrap();
    let mut props: Vec<&str> = schema["properties"]
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    props.sort_unstable();
    assert_eq!(props, ["author", "max_count", "repo"], "{schema}");
    assert_eq!(schema["properties"]["max_count"]["type"], "integer", "{schema}");
    // `git rev-parse -h` shows three usage forms of the same command: unsplit, it switches authority by mode.
    let rev_parse = cap(&caps, "git.rev-parse");
    assert_ne!(admission(rev_parse), "ADMISSION_STATE_SERVABLE");
    assert!(has_reason(rev_parse, "WITHHELD_REASON_MIXED_AUTHORITY"), "{rev_parse}");

    // 4. Invoke a servable READ.
    let r = e.aa_env(&[
        "invoke",
        "git.status",
        "--input-json",
        &input(json!({ "repo": repo_s })),
    ]);
    assert_eq!(r.code, 0, "{r}\n{}", e.log_text());
    let lines = r.data()["lines"].as_array().cloned().unwrap_or_default();
    assert!(
        lines
            .iter()
            .any(|l| l.as_str().is_some_and(|s| s.starts_with("# branch.head"))),
        "{r}"
    );
    let mut invoke_actions = vec![r.json()["meta"]["action_id"].as_str().unwrap_or_default().to_string()];

    // 5. A withheld WRITE is refused and nothing is dispatched.
    let index_before = std::fs::read(repo.join(".git/index")).unwrap();
    let r = e.aa_env(&[
        "x",
        "git.add",
        "--input-json",
        &input(json!({ "repo": repo_s, "pathspec": ["README"] })),
    ]);
    assert_eq!(r.code, 3, "{r}");
    assert_eq!(r.error()["code"], "ERROR_CODE_CAPABILITY_WITHHELD", "{r}");
    assert_eq!(
        std::fs::read(repo.join(".git/index")).unwrap(),
        index_before,
        "git.add touched the index"
    );
    invoke_actions.push(r.json()["meta"]["action_id"].as_str().unwrap_or_default().to_string());

    // 6. A path outside the policy's read roots.
    let r = e.aa_env(&["x", "git.status", "--input-json", &input(json!({ "repo": "/etc" }))]);
    assert_eq!(r.code, 3, "{r}");
    assert_eq!(r.error()["denialReason"], "DENIAL_REASON_OUTSIDE_PATH_SCOPE", "{r}");
    invoke_actions.push(r.json()["meta"]["action_id"].as_str().unwrap_or_default().to_string());

    // 7. Schema-invalid input.
    let r = e.aa_env(&["x", "git.status", "--input-json", r#"{"repo":1}"#]);
    assert_eq!(r.code, 2, "{r}");
    invoke_actions.push(r.json()["meta"]["action_id"].as_str().unwrap_or_default().to_string());

    // 8. Audit: chain intact; one ActionRecord per invoke attempt; pipeline actions carry intents.
    let a = e.ok(&["audit", "--last", "10", "--verify"]);
    assert_eq!(a["chain"]["intact"], true, "{a}");
    assert!(
        a["entries"].as_array().is_some_and(|v| !v.is_empty() && v.len() <= 10),
        "{a}"
    );
    let entries = e.audit_all();
    let actions: Vec<&Value> = entries.iter().filter_map(|x| x.get("action")).collect();
    let intents: Vec<&Value> = entries.iter().filter_map(|x| x.get("intent")).collect();
    let user_actions: Vec<&&Value> = actions
        .iter()
        .filter(|r| {
            !r["capabilityId"]
                .as_str()
                .unwrap_or_default()
                .starts_with("aa.pipeline.")
        })
        .collect();
    assert_eq!(
        user_actions.len(),
        4,
        "one record per invoke attempt: {user_actions:#?}"
    );
    for id in invoke_actions.iter().filter(|s| !s.is_empty()) {
        assert!(
            user_actions.iter().any(|r| r["actionId"] == id.as_str()),
            "no record for {id}"
        );
    }
    let has_intent = |action_id: &Value| intents.iter().any(|i| &i["actionId"] == action_id);
    let harvests: Vec<&&Value> = actions
        .iter()
        .filter(|r| {
            r["capabilityId"]
                .as_str()
                .unwrap_or_default()
                .starts_with("aa.pipeline.harvest.")
        })
        .collect();
    assert!(!harvests.is_empty(), "no harvest pipeline action in the audit log");
    for h in &harvests {
        assert!(has_intent(&h["actionId"]), "harvest action without intent: {h}");
    }
    let verifies: Vec<&&Value> = actions
        .iter()
        .filter(|r| {
            r["capabilityId"]
                .as_str()
                .unwrap_or_default()
                .starts_with("aa.pipeline.verify.")
        })
        .collect();
    assert!(!verifies.is_empty(), "no verify pipeline action in the audit log");
    assert!(
        verifies.iter().any(|v| has_intent(&v["actionId"])),
        "no verify pipeline action has an intent"
    );
}

/// The overlay without `--no-optional-locks` (and without `GIT_OPTIONAL_LOCKS=0`), with a fixture that leaves the
/// index's stat data stale (`git read-tree HEAD`, the deterministic form of an mtime bump: Homebrew git compares
/// whole-second mtimes, so a same-second rewrite is not seen): L3 must observe `git status` rewriting `.git/index`.
fn unlocked_overlay() -> String {
    let text = std::fs::read_to_string(overlay_path()).unwrap();
    let mut out = text.clone();
    for (from, to) in [
        (r#"global_args: ["--no-optional-locks", "-c""#, r#"global_args: ["-c""#),
        (r#", GIT_OPTIONAL_LOCKS: "0" }"#, " }"),
        (
            "      - { input: { repo: { $root: /repo } }, fixture: one-commit-repo }",
            "      - { input: { repo: { $root: /repo } }, fixture: touched-repo }",
        ),
        (
            r#"      - { input: { repo: { $root: /repo }, untracked_files: "no" }, fixture: one-commit-repo }"#,
            r#"      - { input: { repo: { $root: /repo }, untracked_files: "no" }, fixture: touched-repo }"#,
        ),
        (
            "\ncapabilities:\n",
            concat!(
                "  - id: touched-repo\n",
                "    steps:\n",
                "      - include_fixture: one-commit-repo\n",
                "      - argv: [git, -C, repo, read-tree, HEAD]\n",
                "    ready: git.status\n",
                "\ncapabilities:\n"
            ),
        ),
    ] {
        assert!(out.contains(from), "overlay no longer contains {from:?}");
        out = out.replacen(from, to, 1);
    }
    let code: String = out
        .lines()
        .map(|l| l.split(" #").next().unwrap_or_default())
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("--no-optional-locks") && !code.contains("GIT_OPTIONAL_LOCKS"),
        "{code}"
    );
    out
}

/// Engine-free guard: the variant transform still applies to the committed overlay.
#[test]
fn unlocked_overlay_transform_applies() {
    let o = unlocked_overlay();
    assert!(o.contains("- id: touched-repo"));
    assert_eq!(o.matches("fixture: touched-repo").count(), 2);
    if let Some(dump) = std::env::var_os("AA_DUMP_UNLOCKED_OVERLAY") {
        std::fs::write(dump, &o).unwrap();
    }
}

#[test]
fn git_status_side_effect_caught() {
    let e = Engine::start();
    let overlay = e.root.join("git-unlocked.aa.yaml");
    std::fs::write(&overlay, unlocked_overlay()).unwrap();
    add_git(&e, overlay.to_str().unwrap());
    e.ok(&["discover", "git"]);
    e.ok(&["verify", "git"]);
    let caps = e.capabilities("git");
    let status = cap(&caps, "git.status");
    assert_ne!(admission(status), "ADMISSION_STATE_SERVABLE", "{status}");
    let desc = e.ok(&["describe", "git.status"]);
    let records = desc["records"].as_array().cloned().unwrap_or_default();
    let side_effect: Vec<&Value> = records
        .iter()
        .filter(|r| {
            r["findings"]
                .as_array()
                .is_some_and(|f| f.iter().any(|w| w["reason"] == "WITHHELD_REASON_SIDE_EFFECT"))
        })
        .collect();
    assert!(!side_effect.is_empty(), "no SIDE_EFFECT finding: {desc}");
    assert!(
        side_effect.iter().any(|r| r.to_string().contains(".git/index")),
        "SIDE_EFFECT finding does not name .git/index: {side_effect:#?}"
    );
    assert!(
        has_reason(status, "WITHHELD_REASON_SIDE_EFFECT") || desc.to_string().contains("WITHHELD_REASON_SIDE_EFFECT"),
        "{status}"
    );
}

#[test]
fn operator_split_from_cli() {
    let e = Engine::start();
    let halted = e.ok(&["halt", "--reason", "e2e"]);
    let epoch = as_u64(&halted["halt"]["haltEpoch"]);
    assert!(epoch > 0, "{halted}");
    let s = e.ok(&["status"]);
    assert_eq!(s["halt"]["halted"], true, "{s}");
    assert_eq!(s["callerRole"], "ROLE_AGENT", "{s}");

    // Halted: dispatch is refused at the listener (exit 4), before any capability resolves.
    let r = e.aa_env(&["x", "git.status", "--input-json", "{}"]);
    assert_eq!(r.code, 4, "{r}");

    // operator.sock unreadable => unreachable => 4.
    let epoch_s = epoch.to_string();
    Engine::chmod(&e.operator_socket(), 0o000);
    let r = e.aa_env(&["unhalt", "--epoch", &epoch_s]);
    Engine::chmod(&e.operator_socket(), 0o600);
    assert_eq!(r.code, 4, "{r}");

    // The same request over agent.sock is refused by the role split => 3 ACCESS_DENIED.
    let r = e.aa_env(&["unhalt", "--epoch", &epoch_s, "--via-agent-socket"]);
    assert_eq!(r.code, 3, "{r}");
    assert_eq!(r.error()["code"], "ERROR_CODE_ACCESS_DENIED", "{r}");

    // The operator socket releases it (quoted epoch, as `jq` prints proto3-JSON uint64).
    let quoted = format!("\"{epoch}\"");
    e.ok(&["unhalt", "--epoch", &quoted, "--reason", "e2e done"]);
    let s = e.ok(&["status"]);
    assert_ne!(s["halt"]["halted"], true, "{s}");
}
