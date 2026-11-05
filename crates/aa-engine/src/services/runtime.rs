//! RuntimeService: OpenSession, CloseSession, Invoke (stream), Cancel, GetActionStatus. Resume is M3.

use super::{RpcResult, unimplemented};
use crate::{
    Engine, GateEvent,
    role_layer::principal_of,
    sessions::{NewSession, invalid_root},
    status::StopCause,
    to_status,
};
use aa_core::pb;
use aa_proto::v1::runtime_service_server::RuntimeService;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub(crate) struct RuntimeSvc(pub Arc<Engine>);

fn to_response(action_id: &str, ev: GateEvent) -> pb::InvokeResponse {
    use pb::invoke_response::Event;
    pb::InvokeResponse {
        action_id: action_id.to_string(),
        event: Some(match ev {
            GateEvent::Progress(s) => Event::Progress(s),
            GateEvent::Result(r) => Event::Result(r),
            GateEvent::Error(e) => Event::Error(e),
        }),
    }
}

#[tonic::async_trait]
impl RuntimeService for RuntimeSvc {
    type InvokeStream = ReceiverStream<Result<pb::InvokeResponse, tonic::Status>>;
    type ResumeStream = ReceiverStream<Result<pb::ResumeResponse, tonic::Status>>;

    async fn open_session(
        &self,
        request: tonic::Request<pb::OpenSessionRequest>,
    ) -> RpcResult<pb::OpenSessionResponse> {
        let p = principal_of(&request)?;
        let r = request.into_inner();
        let e = &self.0;
        for t in &r.target_ids {
            if e.store.read_target(t).map_err(|e| to_status(&e))?.is_none() {
                return Err(to_status(&crate::ports::not_found(
                    format!("unknown target {t:?}"),
                    "aa target add <id> ...",
                )));
            }
        }
        let mut read_roots = Vec::new();
        for root in &r.read_roots {
            let c = std::fs::canonicalize(PathBuf::from(root)).map_err(|_| to_status(&invalid_root(root.as_ref())))?;
            if !e.policy.roots.permits_read_root(&c) {
                return Err(to_status(&invalid_root(&c)));
            }
            read_roots.push(c);
        }
        let s = e
            .sessions
            .open(
                &e.store.sessions_dir(),
                NewSession {
                    ceiling: r.authority_ceiling(),
                    target_ids: r.target_ids,
                    read_roots,
                    owner_uid: p.uid,
                    ephemeral: false,
                },
            )
            .map_err(|e| to_status(&e))?;
        Ok(tonic::Response::new(pb::OpenSessionResponse {
            session_id: s.id.clone(),
            workspace_path: s.workspace.display().to_string(),
            effective_ceiling: s.ceiling as i32,
        }))
    }

    async fn close_session(
        &self,
        request: tonic::Request<pb::CloseSessionRequest>,
    ) -> RpcResult<pb::CloseSessionResponse> {
        let p = principal_of(&request)?;
        let r = request.into_inner();
        let kept = self
            .0
            .sessions
            .close(&r.session_id, p.uid, r.keep_workspace)
            .map_err(|e| to_status(&e))?;
        Ok(tonic::Response::new(pb::CloseSessionResponse {
            retained_workspace_path: kept.map(|k| k.display().to_string()).unwrap_or_default(),
        }))
    }

    async fn invoke(&self, request: tonic::Request<pb::InvokeRequest>) -> RpcResult<Self::InvokeStream> {
        let p = principal_of(&request)?;
        let req = request.into_inner();
        let gate = self.0.gate();
        // Unknown session/capability: NOT_FOUND, and no ActionRecord (architecture §4).
        gate.preflight(&p, &req).map_err(|e| to_status(&e))?;
        let action_id = aa_core::ids::new_action_id();
        let (gate_tx, mut gate_rx) = mpsc::channel::<GateEvent>(16);
        let (out_tx, out_rx) = mpsc::channel(16);
        let id = action_id.clone();
        tokio::spawn(async move {
            gate.invoke_as(id, &p, req, gate_tx).await;
        });
        tokio::spawn(async move {
            // Dropping `gate_rx` (client gone) is what tells the gate to cancel the dispatch.
            loop {
                tokio::select! {
                    ev = gate_rx.recv() => match ev {
                        Some(ev) => {
                            if out_tx.send(Ok(to_response(&action_id, ev))).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                    () = out_tx.closed() => break,
                }
            }
        });
        Ok(tonic::Response::new(ReceiverStream::new(out_rx)))
    }

    async fn resume(&self, request: tonic::Request<pb::ResumeRequest>) -> RpcResult<Self::ResumeStream> {
        principal_of(&request)?;
        Err(unimplemented("RuntimeService.Resume"))
    }

    async fn cancel(&self, request: tonic::Request<pb::CancelRequest>) -> RpcResult<pb::CancelResponse> {
        principal_of(&request)?;
        let id = &request.get_ref().action_id;
        if self.0.inflight.cancel(id, StopCause::CancelRpc) {
            return Ok(tonic::Response::new(pb::CancelResponse {
                outcome: pb::Outcome::Cancelled as i32,
            }));
        }
        match self.0.audit.find_action(id).map_err(|e| to_status(&e))? {
            Some(r) => Ok(tonic::Response::new(pb::CancelResponse { outcome: r.outcome })),
            None => Err(to_status(&crate::ports::not_found(
                format!("unknown action {id:?}"),
                "aa audit --last 20",
            ))),
        }
    }

    async fn get_action_status(
        &self,
        request: tonic::Request<pb::GetActionStatusRequest>,
    ) -> RpcResult<pb::GetActionStatusResponse> {
        principal_of(&request)?;
        use pb::get_action_status_request::Key;
        let e = &self.0;
        let found = match &request.get_ref().key {
            Some(Key::ActionId(id)) => {
                if e.inflight.contains(id) {
                    Some(pb::ActionRecord {
                        action_id: id.clone(),
                        outcome: pb::Outcome::Unknown as i32,
                        ..Default::default()
                    })
                } else {
                    e.audit.find_action(id).map_err(|e| to_status(&e))?
                }
            }
            Some(Key::IdempotencyKey(k)) => e.audit.find_by_idempotency_key(k).map_err(|e| to_status(&e))?,
            None => {
                return Err(to_status(&aa_core::AaError::invalid(
                    "action_id or idempotency_key is required",
                )));
            }
        };
        match found {
            Some(record) => Ok(tonic::Response::new(pb::GetActionStatusResponse {
                record: Some(record),
            })),
            None => Err(to_status(&crate::ports::not_found(
                "no such action",
                "aa audit --last 20",
            ))),
        }
    }
}
