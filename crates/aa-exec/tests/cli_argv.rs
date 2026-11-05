//! `cli_argv` acceptance tests (build-plan §3.2 acceptance 3 and 4): `build_argv` table tests,
//! `normalize_output`, and `CliArgvBinding` plan/invoke.

use aa_core::{
    api::{Binding, ConfinementSpec, InvokeCtx, RunOutput, testing},
    pb,
};
use aa_exec::{CliArgvBinding, NoConfiner, Supervisor, SupervisorConfig, build_argv, normalize_output};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

const EXE: &str = "/opt/tool/bin/tool";

fn arg(pointer: &str, style: pb::ArgStyle, flag: &str) -> pb::ArgMapping {
    pb::ArgMapping {
        pointer: pointer.into(),
        style: style as i32,
        flag: flag.into(),
        ..Default::default()
    }
}

fn positional(pointer: &str, position: u32) -> pb::ArgMapping {
    pb::ArgMapping {
        pointer: pointer.into(),
        style: pb::ArgStyle::Positional as i32,
        position,
        ..Default::default()
    }
}

fn cap(b: pb::CliArgvBinding) -> pb::Capability {
    pb::Capability {
        id: "tool.x".into(),
        target_id: "tool".into(),
        binding: Some(pb::Binding {
            kind: Some(pb::binding::Kind::Cli(b)),
        }),
        ..Default::default()
    }
}

fn identity(path: &str) -> pb::TargetIdentity {
    pb::TargetIdentity {
        files: vec![pb::FileIdentity {
            role: "executable".into(),
            realpath: path.into(),
            sha256: "00".into(),
            version_banner: String::new(),
        }],
        ..Default::default()
    }
}

fn ctx() -> InvokeCtx {
    InvokeCtx {
        action_id: "act_test".into(),
        identity: identity(EXE),
        state_handle: None,
        scratch_parent: PathBuf::from("/scratch/run"),
        deadline: Duration::from_secs(60),
        dry_run: false,
        confinement: None,
    }
}

fn argv(b: pb::CliArgvBinding, input: Value) -> Vec<String> {
    build_argv(&cap(b), &input, &ctx()).unwrap()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn text_binding() -> pb::CliArgvBinding {
    pb::CliArgvBinding {
        executable_role: "executable".into(),
        command_path: s(&["notes", "add"]),
        args: vec![arg("/text", pb::ArgStyle::Option, "--text")],
        output: pb::OutputMode::TextStdout as i32,
        ..Default::default()
    }
}

#[test]
fn option_value_starting_with_dash_uses_equals_form() {
    assert_eq!(
        argv(text_binding(), json!({"text": "-rf"})),
        s(&[EXE, "notes", "add", "--text=-rf"])
    );
    assert_eq!(
        argv(text_binding(), json!({"text": 5})),
        s(&[EXE, "notes", "add", "--text=5"])
    );
    assert_eq!(argv(text_binding(), json!({})), s(&[EXE, "notes", "add"]));
    assert_eq!(argv(text_binding(), json!({"text": null})), s(&[EXE, "notes", "add"]));
}

#[test]
fn positionals_come_after_the_separator() {
    let b = pb::CliArgvBinding {
        command_path: s(&["add"]),
        args: vec![
            positional("/pathspec", 1),
            arg("/force", pb::ArgStyle::Flag, "--force"),
            positional("/first", 0),
        ],
        ..Default::default()
    };
    assert_eq!(
        argv(
            b.clone(),
            json!({"pathspec": ["-rf", "b.txt"], "force": true, "first": "--x"})
        ),
        s(&[EXE, "add", "--force", "--", "--x", "-rf", "b.txt"])
    );
    // No positional values: no separator.
    assert_eq!(argv(b.clone(), json!({"force": false})), s(&[EXE, "add"]));
    assert_eq!(argv(b, json!({"pathspec": []})), s(&[EXE, "add"]));
}

#[test]
fn repeated_option() {
    let b = pb::CliArgvBinding {
        args: vec![arg("/tag", pb::ArgStyle::RepeatedOption, "--tag")],
        ..Default::default()
    };
    assert_eq!(
        argv(b.clone(), json!({"tag": ["a", "-b"]})),
        s(&[EXE, "--tag=a", "--tag=-b"])
    );
    assert_eq!(argv(b, json!({"tag": "one"})), s(&[EXE, "--tag=one"]));
}

#[test]
fn negatable_and_plain_flags() {
    let b = pb::CliArgvBinding {
        args: vec![
            arg("/short", pb::ArgStyle::NegatableFlag, "--short"),
            arg("/verbose", pb::ArgStyle::Flag, "--verbose"),
        ],
        ..Default::default()
    };
    assert_eq!(argv(b.clone(), json!({"short": true})), s(&[EXE, "--short"]));
    assert_eq!(
        argv(b.clone(), json!({"short": false, "verbose": true})),
        s(&[EXE, "--no-short", "--verbose"])
    );
    let e = build_argv(&cap(b), &json!({"verbose": "yes"}), &ctx()).unwrap_err();
    assert_eq!(e.code, pb::ErrorCode::InvalidRequest);
}

fn cli_anything_binding() -> pb::CliArgvBinding {
    pb::CliArgvBinding {
        executable_role: "interpreter".into(),
        interpreter_args: s(&["-I", "-m", "cli_anything.libreoffice"]),
        global_args: s(&["--json"]),
        state_handle_flag: "--project".into(),
        state_handle_pointer: "/project".into(),
        state_handle_engine_supplied: true,
        command_path: s(&["writer", "add-paragraph"]),
        fixed_args: s(&["--fixed"]),
        args: vec![arg("/text", pb::ArgStyle::Option, "--text")],
        output: pb::OutputMode::JsonStdout as i32,
        errors: pb::ErrorConvention::CliAnythingJson as i32,
        usage_exit_code: 2,
        ..Default::default()
    }
}

fn cli_anything_ctx() -> InvokeCtx {
    let mut c = ctx();
    c.identity.files[0].role = "interpreter".into();
    c.identity.files[0].realpath = "/venv/bin/python".into();
    c.state_handle = Some(PathBuf::from("/ws/ses_1/project.json"));
    c
}

#[test]
fn state_handle_and_dry_run_come_before_the_command_path() {
    let mut c = cap(cli_anything_binding());
    c.dry_run = Some(pb::DryRun {
        supported: true,
        argv: s(&["--dry-run"]),
        kind: pb::DryRunKind::NoPersist as i32,
        verified: false,
    });
    let mut x = cli_anything_ctx();
    let got = build_argv(&c, &json!({"text": "hi"}), &x).unwrap();
    assert_eq!(
        got,
        s(&[
            "/venv/bin/python",
            "-I",
            "-m",
            "cli_anything.libreoffice",
            "--json",
            "--project=/ws/ses_1/project.json",
            "writer",
            "add-paragraph",
            "--fixed",
            "--text=hi"
        ])
    );
    x.dry_run = true;
    let got = build_argv(&c, &json!({"text": "hi"}), &x).unwrap();
    let dry = got.iter().position(|a| a == "--dry-run").unwrap();
    let cmd = got.iter().position(|a| a == "writer").unwrap();
    let project = got.iter().position(|a| a.starts_with("--project=")).unwrap();
    assert!(project < dry && dry < cmd, "{got:?}");

    // dry_run requested but unsupported.
    let plain = cap(cli_anything_binding());
    assert_eq!(
        build_argv(&plain, &json!({}), &x).unwrap_err().code,
        pb::ErrorCode::InvalidRequest
    );
    // Engine-supplied state handle missing.
    let mut no_state = cli_anything_ctx();
    no_state.state_handle = None;
    assert!(build_argv(&plain, &json!({}), &no_state).is_err());
}

#[test]
fn agent_supplied_state_handle_from_the_input() {
    let mut b = cli_anything_binding();
    b.state_handle_engine_supplied = false;
    let mut x = cli_anything_ctx();
    x.state_handle = None;
    let got = build_argv(&cap(b), &json!({"project": "/p/x.json"}), &x).unwrap();
    assert!(got.contains(&"--project=/p/x.json".to_string()));
}

#[test]
fn refusals() {
    // NUL bytes.
    let e = build_argv(&cap(text_binding()), &json!({"text": "a\u{0}b"}), &ctx()).unwrap_err();
    assert_eq!(e.code, pb::ErrorCode::InvalidRequest);
    // Unknown input keys.
    let e = build_argv(&cap(text_binding()), &json!({"text": "a", "extra": 1}), &ctx()).unwrap_err();
    assert_eq!(e.code, pb::ErrorCode::InvalidRequest);
    // Non-object input.
    assert!(build_argv(&cap(text_binding()), &json!([1]), &ctx()).is_err());
    // Objects as option values.
    assert!(build_argv(&cap(text_binding()), &json!({"text": {"a": 1}}), &ctx()).is_err());
    // no_equals_form: a value starting with '-' is refused; others are separate arguments.
    let mut b = text_binding();
    b.args[0].no_equals_form = true;
    let e = build_argv(&cap(b.clone()), &json!({"text": "-rf"}), &ctx()).unwrap_err();
    assert_eq!(e.code, pb::ErrorCode::InvalidRequest);
    assert_eq!(argv(b, json!({"text": "x"})), s(&[EXE, "notes", "add", "--text", "x"]));
    // Missing pinned executable role.
    let mut b = text_binding();
    b.executable_role = "interpreter".into();
    assert_eq!(
        build_argv(&cap(b), &json!({}), &ctx()).unwrap_err().code,
        pb::ErrorCode::ConfigurationInvalid
    );
    // Not a cli binding.
    let mut c = cap(text_binding());
    c.binding = None;
    assert!(build_argv(&c, &json!({}), &ctx()).is_err());
}

#[test]
fn full_order() {
    let b = pb::CliArgvBinding {
        global_args: s(&["--no-optional-locks", "-c", "core.pager=cat"]),
        command_path: s(&["status"]),
        fixed_args: s(&["--porcelain=v2", "--branch"]),
        args: vec![
            arg("/untracked_files", pb::ArgStyle::Option, "--untracked-files"),
            positional("/paths", 0),
        ],
        cwd_pointer: "/repo".into(),
        ..Default::default()
    };
    assert_eq!(
        argv(b, json!({"repo": "/r", "untracked_files": "no", "paths": ["a"]})),
        s(&[
            EXE,
            "--no-optional-locks",
            "-c",
            "core.pager=cat",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
            "--",
            "a"
        ])
    );
}

fn out(exit: i32, stdout: &str) -> RunOutput {
    let mut o = testing::ok_output(stdout);
    o.exit_code = Some(exit);
    o
}

#[test]
fn normalize_cli_anything_error() {
    let o = out(1, r#"{"error":"Index 99 out of range (0-0)","type":"IndexError"}"#);
    let r = normalize_output(&cli_anything_binding(), &o);
    assert_eq!(r.outcome, pb::Outcome::Failed);
    let e = r.error.unwrap();
    assert_eq!(e.code, pb::ErrorCode::UpstreamError);
    let up = e.upstream.unwrap();
    assert_eq!(up.text, "Index 99 out of range (0-0)");
    assert_eq!(up.origin, pb::ContentOrigin::Target as i32);
    assert_eq!(up.source, "IndexError");
    assert!(e.details_json.contains("IndexError"));
    assert!(
        !e.message.contains("Index 99"),
        "engine message must not echo target text"
    );
    assert_eq!(r.process.exit_code, 1);
}

#[test]
fn normalize_unreadable_json_and_text() {
    let r = normalize_output(&cli_anything_binding(), &out(0, "not json"));
    assert_eq!(r.error.unwrap().code, pb::ErrorCode::UnreadableOutput);
    assert_eq!(r.outcome, pb::Outcome::Failed);

    let r = normalize_output(&cli_anything_binding(), &out(0, r#"{"paragraphs":[]}"#));
    assert_eq!(r.outcome, pb::Outcome::Completed);
    assert_eq!(r.output_json.unwrap(), json!({"paragraphs": []}));

    let r = normalize_output(&text_binding(), &out(0, "# branch.head main\nline 2\n"));
    assert_eq!(r.outcome, pb::Outcome::Completed);
    assert!(r.error.is_none());
    assert_eq!(
        r.output_json.unwrap(),
        json!({"text": "# branch.head main\nline 2\n", "lines": ["# branch.head main", "line 2"]})
    );
}

#[test]
fn normalize_exit_code_timeout_and_truncation() {
    let mut o = out(129, "");
    o.stderr_excerpt = "error: unknown option".into();
    let r = normalize_output(&text_binding(), &o);
    let e = r.error.unwrap();
    assert_eq!(e.code, pb::ErrorCode::UpstreamError);
    assert_eq!(e.upstream.unwrap().text, "error: unknown option");
    assert_eq!(
        r.process.stderr_excerpt.unwrap().origin,
        pb::ContentOrigin::Target as i32
    );

    let mut o = out(0, "");
    o.timed_out = true;
    o.exit_code = None;
    o.signal = Some(9);
    let r = normalize_output(&text_binding(), &o);
    assert_eq!(r.outcome, pb::Outcome::Unknown);
    assert_eq!(r.error.unwrap().code, pb::ErrorCode::Timeout);
    assert_eq!(r.process.signal, 9);

    let mut o = out(0, "abc");
    o.stdout_truncated = true;
    o.stdout_total = 10_000;
    let r = normalize_output(&text_binding(), &o);
    assert_eq!(r.error.unwrap().code, pb::ErrorCode::ResourceConstraint);
    assert!(r.output_json.is_none());
    assert!(r.process.output_truncated);
}

#[test]
fn plan_lays_out_the_root_and_merges_confinement() {
    let mut b = text_binding();
    b.cwd_pointer = "/repo".into();
    b.deadline_ms = 5000;
    b.max_output_bytes = 1234;
    b.env.insert("GH_CONFIG_DIR".into(), "{home}/.config/gh".into());
    b.env.insert("HOME".into(), "/evil".into());
    let binding = CliArgvBinding::new(Arc::new(testing::FakeRunner::new()));
    let mut x = ctx();
    x.confinement = Some(ConfinementSpec {
        deny_network: true,
        writable: vec![],
        exec_allow: vec![],
    });
    let spec = binding.plan(&cap(b), &json!({"repo": "/r", "text": "t"}), &x).unwrap();
    assert_eq!(spec.cwd, PathBuf::from("/r"));
    assert_eq!(spec.env["HOME"], "/scratch/run/home");
    assert_eq!(spec.env["TMPDIR"], "/scratch/run/tmp");
    assert_eq!(spec.env["GH_CONFIG_DIR"], "/scratch/run/home/.config/gh");
    assert_eq!(spec.env["LANG"], "C.UTF-8");
    assert!(spec.env["PATH"].starts_with("/opt/tool/bin:"));
    assert!(!spec.env.contains_key("AA_RUN_ID"));
    assert_eq!(spec.deadline, Duration::from_secs(5));
    assert_eq!(spec.max_stdout, 1234);
    assert_eq!(spec.run_id, "act_test");
    let c = spec.confinement.unwrap();
    assert!(c.deny_network);
    assert_eq!(c.writable, vec![PathBuf::from("/scratch/run")]);
    assert_eq!(c.exec_allow, vec![PathBuf::from(EXE)]);

    // Missing cwd input.
    let mut b = text_binding();
    b.cwd_pointer = "/repo".into();
    assert!(binding.plan(&cap(b), &json!({}), &x).is_err());
}

#[tokio::test]
async fn invoke_uses_a_fresh_private_root() {
    let scratch = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let fake = Arc::new(testing::FakeRunner::new().on(&[EXE], testing::ok_output("ok\n")));
    let binding = CliArgvBinding::new(fake.clone());
    let mut b = text_binding();
    b.cwd_pointer = "/repo".into();
    let mut x = ctx();
    x.scratch_parent = scratch.path().to_path_buf();
    let r = binding
        .invoke(&cap(b), &json!({"repo": repo.path(), "text": "hi"}), &x)
        .await
        .unwrap();
    assert_eq!(r.outcome, pb::Outcome::Completed);
    assert_eq!(r.output_json.unwrap()["lines"], json!(["ok"]));
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    let spec = &calls[0];
    assert_eq!(spec.cwd, std::fs::canonicalize(repo.path()).unwrap());
    let home = PathBuf::from(&spec.env["HOME"]);
    let canon_scratch = std::fs::canonicalize(scratch.path()).unwrap();
    assert!(home.starts_with(&canon_scratch), "{home:?}");
    assert!(!home.exists(), "the private root must be removed after the run");
    assert_eq!(spec.argv, s(&[EXE, "notes", "add", "--text=hi"]));
}

#[tokio::test]
async fn invoke_for_real_with_the_supervisor() {
    let scratch = tempfile::tempdir().unwrap();
    let echo = aa_exec::file_identity("executable", std::path::Path::new("/bin/echo"), None).unwrap();
    let sup = Arc::new(Supervisor::new(SupervisorConfig {
        confiner: Arc::new(NoConfiner),
        term_grace: Duration::from_millis(250),
    }));
    let binding = CliArgvBinding::new(sup);
    let b = pb::CliArgvBinding {
        args: vec![positional("/words", 0)],
        output: pb::OutputMode::TextStdout as i32,
        ..Default::default()
    };
    let mut x = ctx();
    x.identity = pb::TargetIdentity {
        files: vec![echo],
        ..Default::default()
    };
    x.scratch_parent = scratch.path().to_path_buf();
    let r = binding
        .invoke(&cap(b), &json!({"words": ["hello", "-n"]}), &x)
        .await
        .unwrap();
    assert_eq!(r.outcome, pb::Outcome::Completed, "{:?}", r.error);
    // echo prints "--" literally (it has no option parser): the separator and both words arrive verbatim.
    assert_eq!(r.output_json.unwrap()["lines"], json!(["-- hello -n"]));
    assert_eq!(r.process.cleanup, pb::CleanupStatus::Complete as i32);
    assert!(r.observed_files.is_empty());
}
