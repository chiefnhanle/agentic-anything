//! The engine-wide error type. It converts losslessly into `pb::Error`, which travels either in a tonic
//! `Status` detail (before a capability resolves) or in-band in an `ActionRecord` (after).

use crate::pb;

/// An engine error. `message` is engine-authored and never echoes target text; target-originated failure text
/// goes in `upstream`. `remediation` is always one concrete next step.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AaError {
    pub code: pb::ErrorCode,
    pub message: String,
    pub remediation: String,
    pub recovery: Option<pb::Recovery>,
    pub denial_reason: pb::DenialReason,
    pub upstream: Option<pb::TargetText>,
    pub details_json: String,
}

/// Result alias used across the workspace.
pub type AaResult<T> = Result<T, AaError>;

/// Remediation used by [`AaError::invalid`].
pub const INVALID_REMEDIATION: &str = "inspect the input schema: aa describe <id>";
/// Remediation used by [`AaError::internal`].
pub const INTERNAL_REMEDIATION: &str = "check the engine status and log, then report the bug: aa status";

impl AaError {
    pub fn new(code: pb::ErrorCode, message: impl Into<String>, remediation: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: remediation.into(),
            recovery: None,
            denial_reason: pb::DenialReason::Unspecified,
            upstream: None,
            details_json: String::new(),
        }
    }

    /// `INVALID_REQUEST` with the standard "inspect the input contract" remediation and recovery.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(pb::ErrorCode::InvalidRequest, message, INVALID_REMEDIATION).with_recovery(
            pb::RecoveryKind::InspectInputContract,
            "CatalogService.DescribeCapability",
        )
    }

    /// `INTERNAL`: a bug or an unexpected local failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(pb::ErrorCode::Internal, message, INTERNAL_REMEDIATION)
    }

    pub fn with_recovery(mut self, kind: pb::RecoveryKind, operation: impl Into<String>) -> Self {
        self.recovery = Some(pb::Recovery {
            kind: kind as i32,
            operation: operation.into(),
            arguments_json: String::new(),
        });
        self
    }

    pub fn with_denial(mut self, reason: pb::DenialReason) -> Self {
        self.denial_reason = reason;
        self
    }

    /// Total mapping from code to category (no wildcard arm: a new code must be classified here).
    pub fn category(&self) -> pb::ErrorCategory {
        category_of(self.code)
    }

    /// Whether a blind retry of the same request is safe and may succeed. Conservative: anything that may have
    /// run (timeouts, unknown execution state) is not retryable; the caller must check status first.
    pub fn retryable(&self) -> bool {
        if let Some(r) = &self.recovery {
            match pb::RecoveryKind::try_from(r.kind) {
                Ok(pb::RecoveryKind::Retry | pb::RecoveryKind::RetryCleanup) => return true,
                Ok(
                    pb::RecoveryKind::CheckStatusBeforeRetry
                    | pb::RecoveryKind::AwaitConfirmation
                    | pb::RecoveryKind::RestartEngine
                    | pb::RecoveryKind::Redistill
                    | pb::RecoveryKind::InspectInputContract
                    | pb::RecoveryKind::ReobserveTarget,
                ) => return false,
                Ok(pb::RecoveryKind::Unspecified) | Err(_) => {}
            }
        }
        matches!(
            self.code,
            pb::ErrorCode::CircuitOpen | pb::ErrorCode::ResourceConstraint | pb::ErrorCode::TargetUnavailable
        )
    }

    pub fn to_proto(&self) -> pb::Error {
        pb::Error {
            code: self.code as i32,
            category: self.category() as i32,
            message: self.message.clone(),
            retryable: self.retryable(),
            remediation: self.remediation.clone(),
            recovery: self.recovery.clone(),
            denial_reason: self.denial_reason as i32,
            upstream: self.upstream.clone(),
            details_json: self.details_json.clone(),
        }
    }
}

/// The single code -> category classifier.
pub fn category_of(code: pb::ErrorCode) -> pb::ErrorCategory {
    use pb::ErrorCategory as C;
    use pb::ErrorCode as E;
    match code {
        E::InvalidRequest | E::UnresolvedSelector | E::ConfigurationInvalid => C::InvalidInput,
        E::AccessDenied
        | E::PolicyDenied
        | E::Excluded
        | E::ConfirmationRequired
        | E::ConfirmationDeclined
        | E::ConfirmationExpired => C::Authorization,
        E::EngineHalted | E::CircuitOpen | E::CapabilityWithheld | E::IsolationNotProven => C::SafetyStop,
        E::CapabilityUnavailable | E::TargetUnavailable | E::AuditUnavailable => C::Unavailable,
        E::TargetIdentityChanged | E::TargetStateChanged | E::CatalogStale => C::Integrity,
        E::Timeout => C::Timeout,
        E::Cancelled => C::Cancelled,
        E::ResourceConstraint => C::Resource,
        E::ActionUnconfirmed
        | E::ExecutionStateUnknown
        | E::UpstreamError
        | E::UnreadableOutput
        | E::CleanupIncomplete
        | E::AdapterProtocol
        | E::Internal
        | E::Unspecified => C::Execution,
    }
}

impl From<std::io::Error> for AaError {
    fn from(e: std::io::Error) -> Self {
        AaError::internal(format!("I/O error: {e}"))
    }
}

impl From<serde_json::Error> for AaError {
    fn from(e: serde_json::Error) -> Self {
        AaError::invalid(format!("JSON error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_has_standard_remediation() {
        let e = AaError::invalid("bad");
        assert_eq!(e.code, pb::ErrorCode::InvalidRequest);
        assert_eq!(e.remediation, INVALID_REMEDIATION);
        assert_eq!(e.category(), pb::ErrorCategory::InvalidInput);
        assert!(!e.retryable());
        assert_eq!(e.to_string(), "InvalidRequest: bad");
    }

    #[test]
    fn every_code_has_a_category_and_remediation_survives_to_proto() {
        for n in 0..=40 {
            let Ok(code) = pb::ErrorCode::try_from(n) else { continue };
            let e = AaError::new(code, "m", "aa status").with_denial(pb::DenialReason::DenyRule);
            let p = e.to_proto();
            assert_eq!(p.code, n);
            assert_ne!(p.category, pb::ErrorCategory::Unspecified as i32, "{code:?}");
            assert_eq!(p.remediation, "aa status");
            assert_eq!(p.denial_reason, pb::DenialReason::DenyRule as i32);
        }
    }

    #[test]
    fn recovery_drives_retryable() {
        let e = AaError::new(pb::ErrorCode::Timeout, "t", "aa status");
        assert!(!e.retryable());
        let e = e.with_recovery(pb::RecoveryKind::Retry, "RuntimeService.Invoke");
        assert!(e.retryable());
        let e = AaError::new(pb::ErrorCode::TargetUnavailable, "t", "aa status").with_recovery(
            pb::RecoveryKind::CheckStatusBeforeRetry,
            "RuntimeService.GetActionStatus",
        );
        assert!(!e.retryable());
        assert!(AaError::new(pb::ErrorCode::TargetUnavailable, "t", "aa status").retryable());
    }

    #[test]
    fn conversions() {
        let io = std::io::Error::other("boom");
        assert_eq!(AaError::from(io).code, pb::ErrorCode::Internal);
        let je = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        assert_eq!(AaError::from(je).code, pb::ErrorCode::InvalidRequest);
    }
}
