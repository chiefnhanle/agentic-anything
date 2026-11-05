//! Rendering: pretty JSON on a TTY, compact when piped or with `--json`; `--full-output` wraps every result in
//! the envelope `{ok, data|error, meta:{action_id}}` on stdout.
//!
//! Without `--full-output`, data goes to stdout and errors go to stderr as `{"error": {...}}`. A closed stdout
//! (EPIPE) yields exit 141.

use crate::exit::{self, CliError, decode_status_error, exit_code};
use aa_proto::v1 as pb;
use serde_json::{Value, json};
use std::io::{IsTerminal as _, Write as _};

/// A successful command result.
#[derive(Debug, Clone)]
pub struct Reply {
    pub data: Value,
    pub action_id: Option<String>,
}

impl Reply {
    pub fn new(data: Value) -> Self {
        Reply { data, action_id: None }
    }

    pub fn with_action(data: Value, action_id: impl Into<String>) -> Self {
        let a = action_id.into();
        Reply {
            data,
            action_id: (!a.is_empty()).then_some(a),
        }
    }
}

/// A failed command result; `action_id` is set when an ActionRecord exists for the attempt.
#[derive(Debug)]
pub struct Failure {
    pub error: CliError,
    pub action_id: Option<String>,
}

impl Failure {
    pub fn with_action(error: CliError, action_id: impl Into<String>) -> Self {
        let a = action_id.into();
        Failure {
            error,
            action_id: (!a.is_empty()).then_some(a),
        }
    }
}

impl From<CliError> for Failure {
    fn from(error: CliError) -> Self {
        Failure { error, action_id: None }
    }
}

impl From<tonic::Status> for Failure {
    fn from(s: tonic::Status) -> Self {
        CliError::Status(s).into()
    }
}

impl From<pb::Error> for Failure {
    fn from(e: pb::Error) -> Self {
        CliError::Domain(e).into()
    }
}

pub type CmdResult = Result<Reply, Failure>;

/// Output settings resolved from the global flags and the terminal state.
#[derive(Debug, Clone, Copy)]
pub struct Output {
    /// Compact JSON (`--json`, or stdout is not a terminal).
    pub compact: bool,
    /// `--full-output` envelope.
    pub full: bool,
    /// Progress lines on stderr (only when stderr is a terminal).
    pub progress: bool,
}

impl Output {
    pub fn new(json: bool, full: bool) -> Self {
        Output {
            compact: json || !std::io::stdout().is_terminal(),
            full,
            progress: std::io::stderr().is_terminal(),
        }
    }

    pub fn progress(&self, msg: &str) {
        if self.progress {
            let _ = writeln!(std::io::stderr().lock(), "aa: {msg}");
        }
    }

    fn render(&self, v: &Value) -> String {
        let s = if self.compact {
            serde_json::to_string(v)
        } else {
            serde_json::to_string_pretty(v)
        };
        s.unwrap_or_else(|_| "null".to_string())
    }

    /// Prints the result and returns the process exit code.
    pub fn emit(&self, result: CmdResult) -> i32 {
        match result {
            Ok(reply) => {
                let v = if self.full {
                    envelope_ok(reply.data, reply.action_id.as_deref())
                } else {
                    reply.data
                };
                match write_line(&mut std::io::stdout().lock(), &self.render(&v)) {
                    Ok(()) => exit::EXIT_OK,
                    Err(code) => code,
                }
            }
            Err(failure) => {
                let code = exit_code(&failure.error);
                let err = error_json(&failure.error);
                let written = if self.full {
                    let v = envelope_err(err, failure.action_id.as_deref());
                    write_line(&mut std::io::stdout().lock(), &self.render(&v))
                } else {
                    let mut v = json!({ "error": err });
                    if let Some(a) = &failure.action_id {
                        v["meta"] = json!({ "action_id": a });
                    }
                    let s = if std::io::stderr().is_terminal() {
                        serde_json::to_string_pretty(&v)
                    } else {
                        serde_json::to_string(&v)
                    }
                    .unwrap_or_default();
                    write_line(&mut std::io::stderr().lock(), &s)
                };
                match written {
                    Ok(()) => code,
                    Err(c) if c == exit::EXIT_EPIPE => c,
                    Err(_) => code,
                }
            }
        }
    }
}

/// Writes `s` plus a newline; EPIPE => 141, any other I/O error => 1.
pub fn write_line(w: &mut dyn std::io::Write, s: &str) -> Result<(), i32> {
    let r = w
        .write_all(s.as_bytes())
        .and_then(|()| w.write_all(b"\n"))
        .and_then(|()| w.flush());
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Err(exit::EXIT_EPIPE),
        Err(_) => Err(exit::EXIT_FAILED),
    }
}

pub fn envelope_ok(data: Value, action_id: Option<&str>) -> Value {
    json!({ "ok": true, "data": data, "meta": meta(action_id) })
}

pub fn envelope_err(error: Value, action_id: Option<&str>) -> Value {
    json!({ "ok": false, "error": error, "meta": meta(action_id) })
}

fn meta(action_id: Option<&str>) -> Value {
    match action_id {
        Some(a) => json!({ "action_id": a }),
        None => json!({}),
    }
}

/// Proto3-JSON of any pbjson message (camelCase names, enum names, zero values omitted).
pub fn to_json<T: serde::Serialize>(m: &T) -> Value {
    serde_json::to_value(m).unwrap_or(Value::Null)
}

/// The error as an `agentic_anything.v1.Error` JSON object, whatever its origin.
pub fn error_json(err: &CliError) -> Value {
    to_json(&as_proto_error(err))
}

/// Normalizes any CLI failure into an engine-shaped `Error` for display.
pub fn as_proto_error(err: &CliError) -> pb::Error {
    match err {
        CliError::Domain(e) => e.clone(),
        CliError::Status(s) => decode_status_error(s).unwrap_or_else(|| status_fallback(s)),
        CliError::Unreachable(m) => pb::Error {
            code: pb::ErrorCode::Unspecified as i32,
            category: pb::ErrorCategory::Unavailable as i32,
            message: format!("engine unreachable: {m}"),
            retryable: false,
            remediation:
                "start the engine with `aa daemon` (or point AA_RUNTIME_DIR at its runtime dir), then check `aa status`"
                    .into(),
            recovery: Some(pb::Recovery {
                kind: pb::RecoveryKind::RestartEngine as i32,
                operation: "aa daemon".into(),
                arguments_json: String::new(),
            }),
            ..Default::default()
        },
        CliError::Usage(m) => pb::Error {
            code: pb::ErrorCode::InvalidRequest as i32,
            category: pb::ErrorCategory::InvalidInput as i32,
            message: m.clone(),
            remediation: "see `aa --help`".into(),
            ..Default::default()
        },
    }
}

fn status_fallback(s: &tonic::Status) -> pb::Error {
    use tonic::Code;
    let (code, category) = match s.code() {
        Code::PermissionDenied | Code::Unauthenticated => {
            (pb::ErrorCode::AccessDenied, pb::ErrorCategory::Authorization)
        }
        Code::NotFound => (pb::ErrorCode::UnresolvedSelector, pb::ErrorCategory::InvalidInput),
        Code::InvalidArgument | Code::OutOfRange => (pb::ErrorCode::InvalidRequest, pb::ErrorCategory::InvalidInput),
        Code::Unavailable => (pb::ErrorCode::Unspecified, pb::ErrorCategory::Unavailable),
        Code::DeadlineExceeded => (pb::ErrorCode::Timeout, pb::ErrorCategory::Timeout),
        Code::Cancelled => (pb::ErrorCode::Cancelled, pb::ErrorCategory::Cancelled),
        _ => (pb::ErrorCode::Unspecified, pb::ErrorCategory::Execution),
    };
    let remediation = match s.code() {
        Code::Unimplemented => "this engine does not implement the RPC yet; check `aa status` for its version",
        Code::Unavailable => "check the engine: `aa status`",
        _ => "see `aa --help`",
    };
    pb::Error {
        code: code as i32,
        category: category as i32,
        message: format!("{:?}: {}", s.code(), s.message()),
        remediation: remediation.into(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes() {
        let v = envelope_ok(json!({"x": 1}), Some("act_1"));
        assert_eq!(v, json!({"ok": true, "data": {"x": 1}, "meta": {"action_id": "act_1"}}));
        let v = envelope_err(json!({"code": "E"}), None);
        assert_eq!(v, json!({"ok": false, "error": {"code": "E"}, "meta": {}}));
    }

    #[test]
    fn errors_render_as_proto_json() {
        let e = pb::Error {
            code: pb::ErrorCode::CapabilityWithheld as i32,
            message: "withheld".into(),
            denial_reason: pb::DenialReason::OutsidePathScope as i32,
            ..Default::default()
        };
        let v = error_json(&CliError::Domain(e));
        assert_eq!(v["code"], "ERROR_CODE_CAPABILITY_WITHHELD");
        assert_eq!(v["denialReason"], "DENIAL_REASON_OUTSIDE_PATH_SCOPE");
        let v = error_json(&CliError::Unreachable("x".into()));
        assert_eq!(v["recovery"]["kind"], "RECOVERY_KIND_RESTART_ENGINE");
        let v = error_json(&CliError::Status(tonic::Status::permission_denied("no")));
        assert_eq!(v["code"], "ERROR_CODE_ACCESS_DENIED");
    }

    #[test]
    fn broken_pipe_is_141() {
        struct Closed;
        impl std::io::Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(write_line(&mut Closed, "x"), Err(141));
        let mut buf = Vec::new();
        assert_eq!(write_line(&mut buf, "x"), Ok(()));
        assert_eq!(buf, b"x\n");
    }
}
