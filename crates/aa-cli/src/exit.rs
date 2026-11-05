//! Exit codes: 0 ok | 1 failed | 2 usage/schema | 3 needs human/authority | 4 engine unreachable or halted |
//! 130/143 signal | 141 EPIPE.
//!
//! [`exit_code_for_error`] is the single classifier from an engine `ErrorCode` to an exit code (ADR 0010). Every
//! other path ([`exit_code`] for transport failures, usage errors and tonic statuses) funnels into it whenever an
//! encoded `aa` error is available.

use aa_proto::v1 as pb;
use prost::Message as _;

/// Completed.
pub const EXIT_OK: i32 = 0;
/// The target ran (or tried to) and failed: upstream error, unreadable output, timeout, resource limit.
pub const EXIT_FAILED: i32 = 1;
/// Usage error, or input rejected by the canonical schema.
pub const EXIT_USAGE: i32 = 2;
/// Needs a human or more authority: confirmation, policy deny, withheld, identity drift.
pub const EXIT_NEEDS_HUMAN: i32 = 3;
/// Engine unreachable or halted: restart or unhalt the engine; do not retry the call.
pub const EXIT_ENGINE: i32 = 4;
/// Interrupted (SIGINT) after cleanup.
pub const EXIT_SIGINT: i32 = 130;
/// Terminated (SIGTERM) after cleanup.
pub const EXIT_SIGTERM: i32 = 143;
/// stdout closed by the reader (EPIPE / SIGPIPE).
pub const EXIT_EPIPE: i32 = 141;

#[derive(Debug)]
pub enum CliError {
    /// The engine socket could not be reached (absent, refused, permission denied).
    Unreachable(String),
    /// A tonic status before the capability resolved; the aa `Error` (if any) is in `details`.
    Status(tonic::Status),
    /// An in-band engine error.
    Domain(aa_proto::v1::Error),
    /// Bad command line or unreadable local input.
    Usage(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Unreachable(m) => write!(f, "engine unreachable: {m}"),
            CliError::Status(s) => match decode_status_error(s) {
                Some(e) => write!(f, "{}: {}", e.code().as_str_name(), e.message),
                None => write!(f, "{:?}: {}", s.code(), s.message()),
            },
            CliError::Domain(e) => write!(f, "{}: {}", e.code().as_str_name(), e.message),
            CliError::Usage(m) => write!(f, "usage: {m}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<tonic::Status> for CliError {
    fn from(s: tonic::Status) -> Self {
        CliError::Status(s)
    }
}

impl From<pb::Error> for CliError {
    fn from(e: pb::Error) -> Self {
        CliError::Domain(e)
    }
}

/// Decodes the aa `Error` the engine encodes into `Status::details` (prost bytes of `agentic_anything.v1.Error`).
///
/// Returns `None` when the details are empty, undecodable, or carry no error code.
pub fn decode_status_error(status: &tonic::Status) -> Option<pb::Error> {
    let details = status.details();
    if details.is_empty() {
        return None;
    }
    let err = pb::Error::decode(details).ok()?;
    (err.code != 0).then_some(err)
}

pub fn exit_code(err: &CliError) -> i32 {
    match err {
        CliError::Unreachable(_) => EXIT_ENGINE,
        CliError::Usage(_) => EXIT_USAGE,
        CliError::Domain(e) => exit_code_for_error(e),
        CliError::Status(s) => match decode_status_error(s) {
            Some(e) => exit_code_for_error(&e),
            None => exit_code_for_grpc(s.code()),
        },
    }
}

/// Fallback for a status that carries no encoded aa `Error` (e.g. a transport failure mid-call).
fn exit_code_for_grpc(code: tonic::Code) -> i32 {
    use tonic::Code;
    match code {
        Code::Ok => EXIT_OK,
        Code::InvalidArgument | Code::NotFound | Code::OutOfRange | Code::AlreadyExists | Code::Unimplemented => {
            EXIT_USAGE
        }
        Code::PermissionDenied | Code::Unauthenticated | Code::FailedPrecondition => EXIT_NEEDS_HUMAN,
        Code::Unavailable => EXIT_ENGINE,
        Code::Cancelled => EXIT_SIGINT,
        Code::Unknown
        | Code::DeadlineExceeded
        | Code::ResourceExhausted
        | Code::Aborted
        | Code::Internal
        | Code::DataLoss => EXIT_FAILED,
    }
}

/// The single classifier. Exhaustive over `ErrorCode` (no wildcard arm): a new code fails to compile here.
pub fn exit_code_for_error(err: &aa_proto::v1::Error) -> i32 {
    use pb::ErrorCode as C;
    match err.code() {
        // Usage / schema.
        C::InvalidRequest | C::UnresolvedSelector | C::ConfigurationInvalid => EXIT_USAGE,
        // Needs a human or more authority.
        C::AccessDenied
        | C::PolicyDenied
        | C::Excluded
        | C::ConfirmationRequired
        | C::ConfirmationDeclined
        | C::ConfirmationExpired
        | C::CapabilityWithheld
        | C::CapabilityUnavailable
        | C::TargetIdentityChanged
        | C::TargetStateChanged
        | C::CatalogStale
        | C::IsolationNotProven => EXIT_NEEDS_HUMAN,
        // Engine halted, or it cannot make intents durable: restart/unhalt, do not retry.
        C::EngineHalted | C::AuditUnavailable => EXIT_ENGINE,
        // The action was cancelled (client drop or Cancel RPC).
        C::Cancelled => EXIT_SIGINT,
        // Ran (or may have run) and failed.
        C::Unspecified
        | C::CircuitOpen
        | C::TargetUnavailable
        | C::ActionUnconfirmed
        | C::ExecutionStateUnknown
        | C::UpstreamError
        | C::UnreadableOutput
        | C::Timeout
        | C::ResourceConstraint
        | C::CleanupIncomplete
        | C::AdapterProtocol
        | C::Internal => EXIT_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(code: pb::ErrorCode) -> pb::Error {
        pb::Error {
            code: code as i32,
            message: "m".into(),
            ..Default::default()
        }
    }

    #[test]
    fn table_rows() {
        assert_eq!(exit_code_for_error(&err(pb::ErrorCode::InvalidRequest)), 2);
        assert_eq!(exit_code_for_error(&err(pb::ErrorCode::CapabilityWithheld)), 3);
        assert_eq!(exit_code_for_error(&err(pb::ErrorCode::PolicyDenied)), 3);
        assert_eq!(exit_code_for_error(&err(pb::ErrorCode::EngineHalted)), 4);
        assert_eq!(exit_code_for_error(&err(pb::ErrorCode::UpstreamError)), 1);
        assert_eq!(exit_code_for_error(&err(pb::ErrorCode::Cancelled)), 130);
    }

    #[test]
    fn status_details_are_decoded() {
        let e = err(pb::ErrorCode::AccessDenied);
        let s = tonic::Status::with_details(tonic::Code::PermissionDenied, "nope", e.encode_to_vec().into());
        assert_eq!(
            decode_status_error(&s).map(|e| e.code()),
            Some(pb::ErrorCode::AccessDenied)
        );
        assert_eq!(exit_code(&CliError::Status(s)), 3);
        // Details win over the transport code.
        let e = err(pb::ErrorCode::EngineHalted);
        let s = tonic::Status::with_details(tonic::Code::Unavailable, "halted", e.encode_to_vec().into());
        assert_eq!(exit_code(&CliError::Status(s)), 4);
        let e = err(pb::ErrorCode::UnresolvedSelector);
        let s = tonic::Status::with_details(tonic::Code::NotFound, "no such", e.encode_to_vec().into());
        assert_eq!(exit_code(&CliError::Status(s)), 2);
    }

    #[test]
    fn status_without_details_uses_grpc_code() {
        assert_eq!(exit_code(&CliError::Status(tonic::Status::permission_denied("x"))), 3);
        assert_eq!(exit_code(&CliError::Status(tonic::Status::unavailable("x"))), 4);
        assert_eq!(exit_code(&CliError::Status(tonic::Status::not_found("x"))), 2);
        assert_eq!(exit_code(&CliError::Status(tonic::Status::invalid_argument("x"))), 2);
        assert_eq!(exit_code(&CliError::Status(tonic::Status::internal("x"))), 1);
        assert_eq!(exit_code(&CliError::Status(tonic::Status::cancelled("x"))), 130);
        // Garbage details fall back to the code.
        let s = tonic::Status::with_details(tonic::Code::NotFound, "x", vec![0xff, 0xff, 0xff].into());
        assert_eq!(exit_code(&CliError::Status(s)), 2);
    }

    #[test]
    fn non_domain_errors() {
        assert_eq!(exit_code(&CliError::Unreachable("no socket".into())), 4);
        assert_eq!(exit_code(&CliError::Usage("bad".into())), 2);
        assert_eq!(exit_code(&CliError::Domain(err(pb::ErrorCode::Timeout))), 1);
    }
}
