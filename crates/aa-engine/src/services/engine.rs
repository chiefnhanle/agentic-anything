//! EngineService: GetStatus, Halt, Unhalt, Shutdown.

use super::RpcResult;
use crate::{Engine, role_layer::principal_of, to_status};
use aa_core::pb;
use aa_proto::v1::engine_service_server::EngineService;
use std::sync::Arc;

pub(crate) struct EngineSvc(pub Arc<Engine>);

impl Engine {
    pub(crate) fn status_for(&self, caller: pb::Role) -> pb::GetStatusResponse {
        let (agent_listen, operator_listen) = crate::status::lock(&self.listen).clone();
        let (targets, servable) = match self.store.list_targets() {
            Ok(ids) => {
                let servable = ids
                    .iter()
                    .filter_map(|t| self.store.read_lock(t).ok().flatten())
                    .flat_map(|l| l.capabilities)
                    .filter(|c| {
                        c.admission
                            .as_ref()
                            .is_some_and(|a| a.state() == pb::AdmissionState::Servable)
                    })
                    .count();
                (ids.len(), servable)
            }
            Err(e) => {
                tracing::warn!(error = %e, "GetStatus could not list targets");
                (0, 0)
            }
        };
        pb::GetStatusResponse {
            engine_version: self.cfg.engine_version.clone(),
            protocol_version: "agentic_anything.v1".into(),
            workspace_root: self.store.root().display().to_string(),
            agent_listen,
            operator_listen,
            caller_role: caller as i32,
            halt: Some(self.halt.current()),
            assurance: self.assurance() as i32,
            confinement_available: self.deps.runner.max_isolation() as i32 >= pb::Isolation::Confined as i32,
            target_count: u32::try_from(targets).unwrap_or(u32::MAX),
            servable_capability_count: u32::try_from(servable).unwrap_or(u32::MAX),
            audit_head_sha256: self.audit.head().1,
        }
    }
}

#[tonic::async_trait]
impl EngineService for EngineSvc {
    async fn get_status(&self, request: tonic::Request<pb::GetStatusRequest>) -> RpcResult<pb::GetStatusResponse> {
        let p = principal_of(&request)?;
        Ok(tonic::Response::new(self.0.status_for(p.role())))
    }

    async fn halt(&self, request: tonic::Request<pb::HaltRequest>) -> RpcResult<pb::HaltResponse> {
        let p = principal_of(&request)?;
        let r = request.into_inner();
        let reason = if r.reason.is_empty() {
            format!("Halt RPC from {}", p.role().as_str_name())
        } else {
            r.reason.chars().take(512).collect()
        };
        let (halt, cancelled) = self
            .0
            .engage_halt(&p, pb::HaltSource::Rpc, r.scope(), &r.scope_subject, &reason);
        Ok(tonic::Response::new(pb::HaltResponse {
            halt: Some(halt),
            cancelled_actions: cancelled,
        }))
    }

    async fn unhalt(&self, request: tonic::Request<pb::UnhaltRequest>) -> RpcResult<pb::UnhaltResponse> {
        let p = principal_of(&request)?;
        let r = request.into_inner();
        let before = self.0.halt.current();
        let result = self.0.halt.unhalt(r.halt_epoch);
        let details = serde_json::json!({
            "requested_epoch": r.halt_epoch, "current_epoch": before.halt_epoch, "reason": r.reason,
            "accepted": result.is_ok(), "previous_source": before.source().as_str_name(),
        });
        if let Err(e) = self.0.admin(&p, pb::AdminEventKind::Unhalt, "engine", details) {
            tracing::error!(error = %e, "could not audit UNHALT");
        }
        match result {
            Ok(halt) => Ok(tonic::Response::new(pb::UnhaltResponse { halt: Some(halt) })),
            Err(e) => Err(to_status(&e)),
        }
    }

    async fn shutdown(&self, request: tonic::Request<pb::ShutdownRequest>) -> RpcResult<pb::ShutdownResponse> {
        let p = principal_of(&request)?;
        let drain = request.get_ref().drain;
        if let Err(e) = self.0.admin(
            &p,
            pb::AdminEventKind::Unspecified,
            "/agentic_anything.v1.EngineService/Shutdown",
            serde_json::json!({"drain": drain}),
        ) {
            tracing::error!(error = %e, "could not audit Shutdown");
        }
        self.0.request_shutdown();
        Ok(tonic::Response::new(pb::ShutdownResponse {}))
    }
}
