//! DistillService: Harvest (stream), Compile, Verify (stream). Package, GetProposalContext and
//! ApplyProposal are slice 2.

use super::{RpcResult, unimplemented};
use crate::{Engine, role_layer::principal_of, to_status};
use aa_core::pb;
use aa_proto::v1::distill_service_server::DistillService;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub(crate) struct DistillSvc(pub Arc<Engine>);

#[tonic::async_trait]
impl DistillService for DistillSvc {
    type HarvestStream = ReceiverStream<Result<pb::HarvestResponse, tonic::Status>>;
    type VerifyStream = ReceiverStream<Result<pb::VerifyResponse, tonic::Status>>;

    async fn harvest(&self, request: tonic::Request<pb::HarvestRequest>) -> RpcResult<Self::HarvestStream> {
        let p = principal_of(&request)?;
        let req = request.into_inner();
        // Unknown target is a transport error (no pipeline action ran).
        if self
            .0
            .store
            .read_target(&req.target_id)
            .map_err(|e| to_status(&e))?
            .is_none()
        {
            return Err(to_status(&crate::ports::not_found(
                format!("unknown target {:?}", req.target_id),
                "aa target add <id> ...",
            )));
        }
        let (tx, rx) = mpsc::channel(32);
        let engine = Arc::clone(&self.0);
        tokio::spawn(async move { engine.harvest(&p, req, tx).await });
        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }

    async fn compile(&self, request: tonic::Request<pb::CompileRequest>) -> RpcResult<pb::CompileResponse> {
        let p = principal_of(&request)?;
        let r = request.into_inner();
        let engine = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || engine.compile_target(&p, &r.target_id, r.check))
            .await
            .map_err(|e| tonic::Status::internal(format!("compile task failed: {e}")))?
            .map(tonic::Response::new)
            .map_err(|e| to_status(&e))
    }

    async fn verify(&self, request: tonic::Request<pb::VerifyRequest>) -> RpcResult<Self::VerifyStream> {
        let p = principal_of(&request)?;
        let req = request.into_inner();
        let (tx, rx) = mpsc::channel(32);
        let engine = Arc::clone(&self.0);
        tokio::spawn(async move { engine.verify(&p, req, tx).await });
        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }

    async fn package(&self, request: tonic::Request<pb::PackageRequest>) -> RpcResult<pb::PackageResponse> {
        principal_of(&request)?;
        Err(unimplemented("DistillService.Package"))
    }

    async fn get_proposal_context(
        &self,
        request: tonic::Request<pb::GetProposalContextRequest>,
    ) -> RpcResult<pb::GetProposalContextResponse> {
        principal_of(&request)?;
        Err(unimplemented("DistillService.GetProposalContext"))
    }

    async fn apply_proposal(
        &self,
        request: tonic::Request<pb::ApplyProposalRequest>,
    ) -> RpcResult<pb::ApplyProposalResponse> {
        principal_of(&request)?;
        Err(unimplemented("DistillService.ApplyProposal"))
    }
}
