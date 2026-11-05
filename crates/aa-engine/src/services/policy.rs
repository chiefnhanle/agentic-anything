//! PolicyService: confirmations, elicitation and grants are M3; every RPC returns UNIMPLEMENTED.

use super::{RpcResult, unimplemented};
use crate::role_layer::principal_of;
use aa_core::pb;
use aa_proto::v1::policy_service_server::PolicyService;

pub(crate) struct PolicySvc;

#[tonic::async_trait]
impl PolicyService for PolicySvc {
    async fn explain_policy(
        &self,
        request: tonic::Request<pb::ExplainPolicyRequest>,
    ) -> RpcResult<pb::ExplainPolicyResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.ExplainPolicy"))
    }

    async fn list_pending_confirmations(
        &self,
        request: tonic::Request<pb::ListPendingConfirmationsRequest>,
    ) -> RpcResult<pb::ListPendingConfirmationsResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.ListPendingConfirmations"))
    }

    async fn decide_confirmation(
        &self,
        request: tonic::Request<pb::DecideConfirmationRequest>,
    ) -> RpcResult<pb::DecideConfirmationResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.DecideConfirmation"))
    }

    async fn relay_elicitation(
        &self,
        request: tonic::Request<pb::RelayElicitationRequest>,
    ) -> RpcResult<pb::RelayElicitationResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.RelayElicitation"))
    }

    async fn issue_grant(&self, request: tonic::Request<pb::IssueGrantRequest>) -> RpcResult<pb::IssueGrantResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.IssueGrant"))
    }

    async fn list_grants(&self, request: tonic::Request<pb::ListGrantsRequest>) -> RpcResult<pb::ListGrantsResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.ListGrants"))
    }

    async fn revoke_grant(
        &self,
        request: tonic::Request<pb::RevokeGrantRequest>,
    ) -> RpcResult<pb::RevokeGrantResponse> {
        principal_of(&request)?;
        Err(unimplemented("PolicyService.RevokeGrant"))
    }
}
