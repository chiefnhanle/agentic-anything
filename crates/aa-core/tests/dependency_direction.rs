//! ADR 0013 / build-plan §2 dependency rule:
//!
//! - `aa-proto <- aa-core <- {aa-exec, aa-harvest, aa-store} <- aa-engine`;
//! - `aa-cli -> aa-proto[grpc]`, plus `aa-engine` as a dev-dependency only;
//! - `aa-core` (normal deps, transitively) must not contain tokio, tonic, hyper, rusqlite or process-wrap.
//!
//! Workspace edges come from `cargo metadata --format-version 1`. The transitive ban uses `cargo tree -p aa-core
//! -e normal`, because metadata's `resolve` unifies features across the whole workspace (it would report
//! `aa-proto`'s optional tonic as enabled because aa-engine enables `grpc`), whereas `cargo tree -p` resolves
//! features for aa-core alone, exactly as `cargo build -p aa-core` does.

use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf, process::Command};

const BANNED: &[&str] = &["tokio", "tonic", "hyper", "rusqlite", "process-wrap"];

fn cargo() -> Command {
    let mut c = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    c.current_dir(workspace_root());
    c
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Layer of each workspace crate; a crate may only depend on crates of a strictly lower layer.
fn layer(name: &str) -> Option<u8> {
    Some(match name {
        "aa-proto" => 0,
        "aa-core" => 1,
        "aa-exec" | "aa-harvest" | "aa-store" => 2,
        "aa-engine" => 3,
        "aa-cli" => 4,
        _ => return None,
    })
}

fn metadata() -> Value {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline"])
        .output()
        .expect("run cargo metadata");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("metadata JSON")
}

/// (from, to, kind, features) for every dependency edge between workspace members.
fn workspace_edges(meta: &Value) -> Vec<(String, String, String, Vec<String>)> {
    let mut edges = Vec::new();
    for pkg in meta["packages"].as_array().expect("packages") {
        let from = pkg["name"].as_str().expect("name").to_string();
        for dep in pkg["dependencies"].as_array().expect("dependencies") {
            let to = dep["name"].as_str().expect("dep name").to_string();
            if layer(&to).is_none() {
                continue;
            }
            let kind = dep["kind"].as_str().unwrap_or("normal").to_string();
            let features = dep["features"]
                .as_array()
                .map(|f| f.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            edges.push((from.clone(), to, kind, features));
        }
    }
    edges
}

#[test]
fn workspace_members_are_the_seven_crates() {
    let meta = metadata();
    let members: BTreeSet<String> = meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|p| p["name"].as_str().expect("name").to_string())
        .collect();
    let expected: BTreeSet<String> = [
        "aa-proto",
        "aa-core",
        "aa-exec",
        "aa-harvest",
        "aa-store",
        "aa-engine",
        "aa-cli",
    ]
    .map(String::from)
    .into();
    assert_eq!(members, expected);
}

#[test]
fn dependency_direction_holds() {
    let meta = metadata();
    let edges = workspace_edges(&meta);
    assert!(!edges.is_empty(), "no workspace edges found");
    let mut violations = Vec::new();
    for (from, to, kind, features) in &edges {
        let (lf, lt) = (layer(from).expect("member"), layer(to).expect("member"));
        let ok = match from.as_str() {
            // The CLI never links engine internals: aa-proto only (with grpc), aa-engine as a dev-dependency.
            "aa-cli" => matches!((kind.as_str(), to.as_str()), (_, "aa-proto") | ("dev", "aa-engine")),
            _ => lt < lf && to != "aa-cli",
        };
        if !ok {
            violations.push(format!("{from} -[{kind}]-> {to}"));
        }
        if from == "aa-core" && to == "aa-proto" && features.iter().any(|f| f == "grpc") {
            violations.push("aa-core enables aa-proto/grpc".into());
        }
    }
    assert!(violations.is_empty(), "dependency rule violated: {violations:#?}");

    let core_on_proto = edges
        .iter()
        .any(|(f, t, k, _)| f == "aa-core" && t == "aa-proto" && k == "normal");
    assert!(core_on_proto, "aa-core must depend on aa-proto");
    let cli_proto_grpc = edges
        .iter()
        .any(|(f, t, k, feats)| f == "aa-cli" && t == "aa-proto" && k == "normal" && feats.iter().any(|x| x == "grpc"));
    assert!(cli_proto_grpc, "aa-cli must depend on aa-proto with the grpc feature");
}

#[test]
fn aa_core_normal_closure_has_no_banned_crates() {
    let out = cargo()
        .args([
            "tree",
            "-p",
            "aa-core",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--offline",
        ])
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf8");
    let names: BTreeSet<&str> = text.lines().filter_map(|l| l.split_whitespace().next()).collect();
    assert!(
        names.contains("aa-core") && names.contains("aa-proto"),
        "unexpected tree output: {text}"
    );
    let banned: Vec<&&str> = BANNED.iter().filter(|b| names.contains(**b)).collect();
    assert!(
        banned.is_empty(),
        "aa-core's normal dependency closure contains {banned:?}"
    );
    // Also catch the hyper/tonic family by prefix (hyper-util, tonic-prost, tokio-*, ...).
    let family: Vec<&&str> = names
        .iter()
        .filter(|n| {
            ["tokio", "tonic", "hyper", "rusqlite", "process-wrap"]
                .iter()
                .any(|b| n.starts_with(b))
        })
        .collect();
    assert!(
        family.is_empty(),
        "aa-core's normal dependency closure contains {family:?}"
    );
}
