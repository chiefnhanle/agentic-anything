//! `aa invoke <cap> --input-json <inline|@file|->` (alias `aa x`) (RuntimeService.Invoke).

use super::{agent, client_error, target::read_capped};
use crate::MAX_INPUT_BYTES;
use crate::connect::ClientConfig;
use crate::exit::CliError;
use crate::output::{CmdResult, Failure, Output, Reply, to_json};
use aa_proto::v1 as pb;
use pb::runtime_service_client::RuntimeServiceClient;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, clap::Args)]
pub struct InvokeArgs {
    /// Capability id, e.g. `git.status`.
    pub capability: String,
    /// The input object: inline JSON, `@file`, or `-` for stdin (at most 8 MiB).
    #[arg(long, default_value = "{}")]
    pub input_json: String,
    /// Session id (from OpenSession); default: an ephemeral session.
    #[arg(long)]
    pub session: Option<String>,
    /// Ask for the capability's native dry run.
    #[arg(long)]
    pub dry_run: bool,
    /// Idempotency key; default: auto-generated per call.
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Why the call is made (shown to a human, labelled as agent-authored).
    #[arg(long)]
    pub reason: Option<String>,
    /// Deadline in milliseconds; default: the capability's binding deadline.
    #[arg(long)]
    pub deadline_ms: Option<u32>,
    /// Refuse to run if the capability digest changed (CATALOG_STALE).
    #[arg(long)]
    pub expect_capability_digest: Option<String>,
}

/// Reads `--input-json`: inline text, `@file`, or `-` (stdin), each capped at 8 MiB, and checks it is JSON.
pub async fn read_input(spec: &str) -> Result<String, CliError> {
    let text = if spec == "-" {
        let bytes = tokio::task::spawn_blocking(|| read_stdin(MAX_INPUT_BYTES))
            .await
            .map_err(|e| CliError::Usage(format!("cannot read stdin: {e}")))??;
        String::from_utf8(bytes).map_err(|_| CliError::Usage("stdin is not UTF-8".into()))?
    } else if let Some(path) = spec.strip_prefix('@') {
        let bytes = read_capped(Path::new(path), MAX_INPUT_BYTES, "input file")?;
        String::from_utf8(bytes).map_err(|_| CliError::Usage(format!("input file {path} is not UTF-8")))?
    } else {
        if spec.len() as u64 > MAX_INPUT_BYTES {
            return Err(CliError::Usage(format!("--input-json exceeds {MAX_INPUT_BYTES} bytes")));
        }
        spec.to_string()
    };
    serde_json::from_str::<Value>(&text)
        .map_err(|e| CliError::Usage(format!("--input-json is not valid JSON: {e}")))?;
    Ok(text)
}

fn read_stdin(cap: u64) -> Result<Vec<u8>, CliError> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    std::io::stdin()
        .lock()
        .take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| CliError::Usage(format!("cannot read stdin: {e}")))?;
    if buf.len() as u64 > cap {
        return Err(CliError::Usage(format!("stdin input exceeds {cap} bytes")));
    }
    Ok(buf)
}

/// A per-call key: unique enough for one workstation (pid + wall-clock nanos + an in-process counter).
pub fn auto_idempotency_key() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("aa-cli:{}:{nanos}:{n}", std::process::id())
}

/// The `data` of a terminal result: the parsed output JSON (or the blob reference when it was too large).
pub fn result_data(r: &pb::InvokeResult) -> Value {
    if !r.output_json.is_empty() {
        return serde_json::from_str(&r.output_json).unwrap_or_else(|_| json!({ "text": r.output_json }));
    }
    if !r.output_blob_sha256.is_empty() {
        return json!({ "outputBlobSha256": r.output_blob_sha256 });
    }
    Value::Null
}

/// Classifies a terminal `InvokeResult` by its outcome (errors normally arrive as `error` events instead).
pub fn result_outcome(r: &pb::InvokeResult, action_id: &str) -> Result<(), pb::Error> {
    let status = format!("check the action: aa audit --action {action_id}");
    match r.outcome() {
        pb::Outcome::Completed | pb::Outcome::CompletedUnverified => Ok(()),
        pb::Outcome::Failed => Err(client_error(
            pb::ErrorCode::UpstreamError,
            "the target reported failure",
            status,
        )),
        pb::Outcome::Cancelled => Err(client_error(
            pb::ErrorCode::Cancelled,
            "the action was cancelled",
            status,
        )),
        pb::Outcome::NotDispatched => Err(client_error(
            pb::ErrorCode::Internal,
            "the engine returned a result for an undispatched action",
            status,
        )),
        pb::Outcome::Unknown => Err(client_error(
            pb::ErrorCode::ExecutionStateUnknown,
            "the action's outcome is unknown; it may or may not have happened",
            status,
        )),
    }
}

/// A parked confirmation is a terminal "needs a human" error in slice 1.
pub fn confirmation_error(c: &pb::PendingConfirmation) -> pb::Error {
    let mut e = client_error(
        pb::ErrorCode::ConfirmationRequired,
        format!("confirmation required for {} ({})", c.capability_id, c.confirmation_id),
        format!(
            "a human must approve it on operator.sock: aa approve {}",
            c.confirmation_id
        ),
    );
    e.recovery = Some(pb::Recovery {
        kind: pb::RecoveryKind::AwaitConfirmation as i32,
        operation: format!("aa approve {}", c.confirmation_id),
        arguments_json: String::new(),
    });
    e.details_json = serde_json::to_string(&to_json(c)).unwrap_or_default();
    e
}

pub async fn run(a: InvokeArgs, cfg: &ClientConfig, out: &Output) -> CmdResult {
    let input_json = read_input(&a.input_json).await?;
    let req = pb::InvokeRequest {
        capability_id: a.capability.clone(),
        input_json,
        session_id: a.session.unwrap_or_default(),
        dry_run: a.dry_run,
        idempotency_key: a.idempotency_key.unwrap_or_else(auto_idempotency_key),
        expected_catalog_digest: String::new(),
        expected_capability_digest: a.expect_capability_digest.unwrap_or_default(),
        deadline_ms: a.deadline_ms.unwrap_or(0),
        stated_reason: a.reason.unwrap_or_default(),
        wait_for_confirmation: false,
    };
    let mut c = RuntimeServiceClient::new(agent(cfg).await?);
    let mut stream = c.invoke(req).await?.into_inner();
    let mut action_id = String::new();
    loop {
        let msg = match stream.message().await {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(s) => return Err(Failure::with_action(CliError::Status(s), action_id)),
        };
        if !msg.action_id.is_empty() {
            action_id = msg.action_id.clone();
        }
        match msg.event {
            Some(pb::invoke_response::Event::Progress(p)) => out.progress(&p),
            Some(pb::invoke_response::Event::Result(r)) => {
                return match result_outcome(&r, &action_id) {
                    Ok(()) => Ok(Reply::with_action(result_data(&r), action_id)),
                    Err(e) => Err(Failure::with_action(CliError::Domain(e), action_id)),
                };
            }
            Some(pb::invoke_response::Event::Error(e)) => {
                return Err(Failure::with_action(CliError::Domain(e), action_id));
            }
            Some(pb::invoke_response::Event::Confirmation(conf)) => {
                return Err(Failure::with_action(
                    CliError::Domain(confirmation_error(&conf)),
                    action_id,
                ));
            }
            None => {}
        }
    }
    let remediation = if action_id.is_empty() {
        "check the audit log: aa audit --last 5".to_string()
    } else {
        format!("check the action before retrying: aa audit --action {action_id}")
    };
    Err(Failure::with_action(
        CliError::Domain(client_error(
            pb::ErrorCode::ExecutionStateUnknown,
            "the invoke stream ended without a terminal event",
            remediation,
        )),
        action_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exit::exit_code_for_error;

    #[tokio::test]
    async fn inline_and_file_inputs() {
        assert_eq!(read_input(r#"{"repo":"/x"}"#).await.unwrap(), r#"{"repo":"/x"}"#);
        assert!(matches!(read_input("{nope").await, Err(CliError::Usage(_))));
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("in.json");
        std::fs::write(&p, br#"{"a":1}"#).unwrap();
        assert_eq!(read_input(&format!("@{}", p.display())).await.unwrap(), r#"{"a":1}"#);
        assert!(matches!(
            read_input("@/definitely/missing.json").await,
            Err(CliError::Usage(_))
        ));
        let big = d.path().join("big.json");
        std::fs::write(&big, vec![b' '; (MAX_INPUT_BYTES + 1) as usize]).unwrap();
        assert!(matches!(
            read_input(&format!("@{}", big.display())).await,
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn result_data_shapes() {
        let r = pb::InvokeResult {
            output_json: r##"{"lines":["# branch.head main"]}"##.into(),
            ..Default::default()
        };
        assert_eq!(result_data(&r)["lines"][0], "# branch.head main");
        let r = pb::InvokeResult {
            output_blob_sha256: "ab".into(),
            ..Default::default()
        };
        assert_eq!(result_data(&r), json!({"outputBlobSha256": "ab"}));
        assert_eq!(result_data(&pb::InvokeResult::default()), Value::Null);
    }

    #[test]
    fn outcomes_classify() {
        let r = |o: pb::Outcome| pb::InvokeResult {
            outcome: o as i32,
            ..Default::default()
        };
        assert!(result_outcome(&r(pb::Outcome::Completed), "a").is_ok());
        assert!(result_outcome(&r(pb::Outcome::CompletedUnverified), "a").is_ok());
        assert_eq!(
            exit_code_for_error(&result_outcome(&r(pb::Outcome::Failed), "a").unwrap_err()),
            1
        );
        assert_eq!(
            exit_code_for_error(&result_outcome(&r(pb::Outcome::Unknown), "a").unwrap_err()),
            1
        );
        assert_eq!(
            exit_code_for_error(&result_outcome(&r(pb::Outcome::Cancelled), "a").unwrap_err()),
            130
        );
    }

    #[test]
    fn confirmations_need_a_human() {
        let c = pb::PendingConfirmation {
            confirmation_id: "cnf_1".into(),
            capability_id: "git.add".into(),
            ..Default::default()
        };
        let e = confirmation_error(&c);
        assert_eq!(exit_code_for_error(&e), 3);
        assert!(e.remediation.contains("aa approve cnf_1"));
    }

    #[test]
    fn idempotency_keys_differ() {
        assert_ne!(auto_idempotency_key(), auto_idempotency_key());
    }
}
