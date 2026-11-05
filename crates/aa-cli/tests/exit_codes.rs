//! Exit-code classifier, argument parsing and endpoint handling. Client-only: no engine runs here.

mod common;

use aa_cli::exit::{CliError, exit_code, exit_code_for_error};
use aa_proto::v1 as pb;
use assert_cmd::Command;
use prost::Message as _;
use std::collections::BTreeMap;

/// The expected exit code for every `ErrorCode`, by proto name. The test below fails if the proto gains a value
/// that this table (and therefore the classifier review) does not cover.
const EXPECTED: &[(&str, i32)] = &[
    ("ERROR_CODE_UNSPECIFIED", 1),
    ("ERROR_CODE_INVALID_REQUEST", 2),
    ("ERROR_CODE_UNRESOLVED_SELECTOR", 2),
    ("ERROR_CODE_ACCESS_DENIED", 3),
    ("ERROR_CODE_POLICY_DENIED", 3),
    ("ERROR_CODE_EXCLUDED", 3),
    ("ERROR_CODE_CONFIRMATION_REQUIRED", 3),
    ("ERROR_CODE_CONFIRMATION_DECLINED", 3),
    ("ERROR_CODE_CONFIRMATION_EXPIRED", 3),
    ("ERROR_CODE_ENGINE_HALTED", 4),
    ("ERROR_CODE_CIRCUIT_OPEN", 1),
    ("ERROR_CODE_CAPABILITY_WITHHELD", 3),
    ("ERROR_CODE_CAPABILITY_UNAVAILABLE", 3),
    ("ERROR_CODE_TARGET_UNAVAILABLE", 1),
    ("ERROR_CODE_TARGET_IDENTITY_CHANGED", 3),
    ("ERROR_CODE_TARGET_STATE_CHANGED", 3),
    ("ERROR_CODE_CATALOG_STALE", 3),
    ("ERROR_CODE_ACTION_UNCONFIRMED", 1),
    ("ERROR_CODE_EXECUTION_STATE_UNKNOWN", 1),
    ("ERROR_CODE_UPSTREAM_ERROR", 1),
    ("ERROR_CODE_UNREADABLE_OUTPUT", 1),
    ("ERROR_CODE_TIMEOUT", 1),
    ("ERROR_CODE_CANCELLED", 130),
    ("ERROR_CODE_RESOURCE_CONSTRAINT", 1),
    ("ERROR_CODE_CLEANUP_INCOMPLETE", 1),
    ("ERROR_CODE_ISOLATION_NOT_PROVEN", 3),
    ("ERROR_CODE_ADAPTER_PROTOCOL", 1),
    ("ERROR_CODE_CONFIGURATION_INVALID", 2),
    ("ERROR_CODE_AUDIT_UNAVAILABLE", 4),
    ("ERROR_CODE_INTERNAL", 1),
];

/// Every `ErrorCode` value, read from the compiled descriptor set (not from a hand-written list).
fn error_code_values() -> Vec<(String, i32)> {
    let fds = pbjson_types::FileDescriptorSet::decode(aa_proto::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let file = fds
        .file
        .iter()
        .find(|f| {
            f.package.as_deref() == Some("agentic_anything.v1") && f.enum_type.iter().any(|e| e.name() == "ErrorCode")
        })
        .expect("ErrorCode enum in descriptor set");
    let e = file.enum_type.iter().find(|e| e.name() == "ErrorCode").unwrap();
    e.value.iter().map(|v| (v.name().to_string(), v.number())).collect()
}

#[test]
fn exit_code_for_error_covers_every_error_code() {
    let values = error_code_values();
    let expected: BTreeMap<&str, i32> = EXPECTED.iter().copied().collect();
    assert_eq!(
        values.len(),
        expected.len(),
        "ErrorCode gained or lost values; update EXPECTED and the classifier"
    );
    for (name, number) in values {
        let code = pb::ErrorCode::try_from(number).unwrap_or_else(|_| panic!("{name}={number} not in the Rust enum"));
        assert_eq!(code.as_str_name(), name);
        let err = pb::Error {
            code: number,
            message: "m".into(),
            ..Default::default()
        };
        let got = exit_code_for_error(&err);
        let want = *expected
            .get(name.as_str())
            .unwrap_or_else(|| panic!("{name} missing from EXPECTED"));
        assert_eq!(got, want, "{name}");
        assert!([1, 2, 3, 4, 130].contains(&got), "{name} -> {got}");
        // The same error arriving as a tonic status detail classifies identically.
        let st = tonic::Status::with_details(tonic::Code::Unknown, "x", err.encode_to_vec().into());
        assert_eq!(exit_code(&CliError::Status(st)), want, "{name} via Status details");
        assert_eq!(exit_code(&CliError::Domain(err)), want, "{name} via Domain");
    }
}

#[test]
fn unknown_numeric_code_is_a_failure() {
    let err = pb::Error {
        code: 9999,
        ..Default::default()
    };
    assert_eq!(exit_code_for_error(&err), 1);
}

#[test]
fn status_without_engine_is_4() {
    let rt = tempfile::tempdir().unwrap();
    let r = common::aa_with_runtime(rt.path(), &["status"]);
    assert_eq!(r.code, 4, "{r}");
    let v: serde_json::Value = serde_json::from_str(r.stderr.trim()).expect("stderr JSON");
    assert_eq!(v["error"]["recovery"]["kind"], "RECOVERY_KIND_RESTART_ENGINE", "{r}");
    assert!(r.stdout.is_empty(), "{r}");
}

#[test]
fn every_rpc_command_without_engine_is_4() {
    let rt = tempfile::tempdir().unwrap();
    for args in [
        vec!["status"],
        vec!["halt", "--reason", "t"],
        vec!["unhalt", "--epoch", "1"],
        vec!["target", "add", "git", "--exec", "git", "--authorization", "t"],
        vec!["discover", "git"],
        vec!["verify", "git"],
        vec!["list"],
        vec!["describe", "git.status"],
        vec!["x", "git.status", "--input-json", "{}"],
        vec!["audit", "--last", "3", "--verify"],
    ] {
        let r = common::aa_with_runtime(rt.path(), &args);
        assert_eq!(r.code, 4, "aa {args:?}\n{r}");
    }
}

#[test]
fn full_output_envelope_when_unreachable() {
    let rt = tempfile::tempdir().unwrap();
    let r = common::aa_with_runtime(rt.path(), &["--full-output", "status"]);
    assert_eq!(r.code, 4, "{r}");
    let v = r.json();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["category"], "ERROR_CATEGORY_UNAVAILABLE");
    assert!(v["meta"].is_object());
    // Global flags also work after the subcommand.
    let r = common::aa_with_runtime(rt.path(), &["status", "--json", "--full-output"]);
    assert_eq!(r.code, 4, "{r}");
    assert_eq!(r.json()["ok"], false);
}

#[test]
fn unreadable_socket_is_4() {
    use std::os::unix::fs::PermissionsExt as _;
    let rt = tempfile::tempdir().unwrap();
    let sock = rt.path().join("operator.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o000)).unwrap();
    let r = common::aa_with_runtime(rt.path(), &["unhalt", "--epoch", "1"]);
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(r.code, 4, "{r}");
}

#[test]
fn usage_errors_are_2() {
    let rt = tempfile::tempdir().unwrap();
    for args in [
        vec!["no-such-command"],
        vec!["invoke"],
        vec!["x", "git.status", "--input-json", "{not json"],
        vec!["x", "git.status", "--input-json", "@/definitely/not/here.json"],
        vec!["unhalt"],
        vec!["unhalt", "--epoch", "seven"],
        vec!["target", "add", "git", "--authorization", "t"],
        vec!["target", "add", "git", "--exec", "git"],
        vec![
            "target",
            "add",
            "git",
            "--exec",
            "git",
            "--python-script",
            "/x",
            "--authorization",
            "t",
        ],
        vec!["target", "add", "Bad_Id", "--exec", "git", "--authorization", "t"],
        vec!["target", "add", "git", "--exec", "git", "--authorization", "   "],
        vec![
            "target",
            "add",
            "git",
            "--exec",
            "git",
            "--overlay",
            "/definitely/missing.yaml",
            "--authorization",
            "t",
        ],
        vec!["verify", "git", "--lane", "bogus"],
        vec![
            "import",
            "cli-anything",
            "/repo",
            "--harness",
            "lo",
            "--authorization",
            "t",
        ],
    ] {
        let r = common::aa_with_runtime(rt.path(), &args);
        assert_eq!(r.code, 2, "aa {args:?}\n{r}");
    }
}

#[test]
fn overlay_for_another_target_is_a_usage_error() {
    let rt = tempfile::tempdir().unwrap();
    let overlay = common::repo_root().join("targets/git/target.aa.yaml");
    let r = common::aa_with_runtime(
        rt.path(),
        &[
            "target",
            "add",
            "gitx",
            "--exec",
            "git",
            "--overlay",
            overlay.to_str().unwrap(),
            "--authorization",
            "t",
        ],
    );
    assert_eq!(r.code, 2, "{r}");
    assert!(r.stderr.contains("gitx"), "{r}");
}

#[test]
fn stdin_input_is_capped_at_8_mib() {
    let rt = tempfile::tempdir().unwrap();
    let big = vec![b' '; (aa_cli::MAX_INPUT_BYTES + 1) as usize];
    let out = Command::new(common::aa_bin())
        .args(["x", "git.status", "--input-json", "-"])
        .env("AA_RUNTIME_DIR", rt.path())
        .write_stdin(big)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    // Valid JSON on stdin passes parsing and then fails to connect (4), proving the order of checks.
    let out = Command::new(common::aa_bin())
        .args(["x", "git.status", "--input-json", "-"])
        .env("AA_RUNTIME_DIR", rt.path())
        .write_stdin(r#"{"repo":"/tmp"}"#)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
}

#[test]
fn help_and_version_are_0() {
    Command::new(common::aa_bin()).arg("--help").assert().code(0);
    Command::new(common::aa_bin()).arg("--version").assert().code(0);
    Command::new(common::aa_bin())
        .args(["invoke", "--help"])
        .assert()
        .code(0);
}

#[test]
fn closed_stdout_is_141() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let status = std::process::Command::new(common::aa_bin())
        .arg("--help")
        .stdout(writer)
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(141));
}

#[test]
fn x_is_an_alias_of_invoke() {
    let list = aa_cli::command_list();
    for cmd in [
        "daemon",
        "status",
        "halt",
        "unhalt",
        "target add",
        "import cli-anything",
        "discover",
        "verify",
        "list",
        "describe",
        "invoke",
        "x",
        "audit",
    ] {
        assert!(list.iter().any(|c| c == cmd), "missing command {cmd:?} in {list:?}");
    }
}

#[tokio::test]
async fn run_in_process_classifies_usage() {
    assert_eq!(aa_cli::run(vec!["aa".into(), "bogus".into()]).await, 2);
    assert_eq!(aa_cli::run(vec!["aa".into(), "--version".into()]).await, 0);
}

#[test]
fn runtime_dir_from_env() {
    // AA_RUNTIME_DIR decides which socket the client dials (checked in a child process, so this process's
    // environment is never mutated).
    let rt = tempfile::tempdir().unwrap();
    let r = common::aa_with_runtime(rt.path(), &["status"]);
    assert!(
        r.stderr.contains(&rt.path().join("agent.sock").display().to_string()),
        "{r}"
    );
}
