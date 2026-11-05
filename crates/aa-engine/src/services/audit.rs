//! AuditService: QueryAudit, VerifyChain. A chain that fails to verify at runtime engages the kill switch.

use super::RpcResult;
use crate::{Engine, role_layer::principal_of, to_status};
use aa_core::pb;
use aa_proto::v1::audit_service_server::AuditService;
use std::sync::Arc;

pub(crate) struct AuditSvc(pub Arc<Engine>);

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 1000;

#[tonic::async_trait]
impl AuditService for AuditSvc {
    async fn query_audit(&self, request: tonic::Request<pb::QueryAuditRequest>) -> RpcResult<pb::QueryAuditResponse> {
        principal_of(&request)?;
        let r = request.into_inner();
        let limit = match r.limit {
            0 => DEFAULT_LIMIT,
            l => l.min(MAX_LIMIT),
        };
        let q = aa_store::AuditQuery {
            target_id: (!r.target_id.is_empty()).then_some(r.target_id),
            capability_id: (!r.capability_id.is_empty()).then_some(r.capability_id),
            action_id: (!r.action_id.is_empty()).then_some(r.action_id),
            after_seq: r.after_seq,
            limit,
            actions_only: r.actions_only,
        };
        let entries = self.0.audit.query(&q).map_err(|e| to_status(&e))?;
        let next_seq = if entries.len() >= limit as usize {
            entries.last().map_or(0, |e| e.seq)
        } else {
            0
        };
        Ok(tonic::Response::new(pb::QueryAuditResponse { entries, next_seq }))
    }

    async fn verify_chain(
        &self,
        request: tonic::Request<pb::VerifyChainRequest>,
    ) -> RpcResult<pb::VerifyChainResponse> {
        let p = principal_of(&request)?;
        let report = self.0.audit.verify();
        let intact = report.chain_ok && report.head_matches;
        let mut resp = pb::VerifyChainResponse {
            intact,
            entries: report.entries,
            first_broken_seq: report.first_broken_seq.unwrap_or(0),
            head_sha256: self.0.audit.head().1,
            head_matches: report.head_matches,
            error: None,
        };
        if !intact {
            let err = aa_core::AaError::new(
                pb::ErrorCode::AuditUnavailable,
                "the audit chain failed to verify; the engine is halted",
                "inspect audit/segment-*.jsonl, then aa unhalt --epoch <N> on operator.sock",
            );
            resp.error = Some(err.to_proto());
            if !self.0.halt.is_halted() {
                self.0.engage_halt(
                    &p,
                    pb::HaltSource::AuditVerifyFailed,
                    pb::HaltScope::Engine,
                    "",
                    "audit chain failed to verify at runtime",
                );
            }
        }
        Ok(tonic::Response::new(resp))
    }
}
