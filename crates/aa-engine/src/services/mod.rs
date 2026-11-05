//! gRPC services. Slice 1 implements the RPCs listed in build-plan §3.5; every other RPC returns
//! `UNIMPLEMENTED`. Every service is reachable only through a [`RoleLayer`](crate::RoleLayer).

mod audit;
mod catalog;
mod distill;
mod engine;
mod policy;
mod runtime;
mod target;

use crate::{Engine, RoleLayer};
use aa_core::pb;
use aa_proto::v1::{
    audit_service_server::AuditServiceServer, catalog_service_server::CatalogServiceServer,
    distill_service_server::DistillServiceServer, engine_service_server::EngineServiceServer,
    policy_service_server::PolicyServiceServer, runtime_service_server::RuntimeServiceServer,
    target_service_server::TargetServiceServer,
};
use std::sync::Arc;
use tower::layer::util::{Identity, Stack};

pub(crate) type Router = tonic::transport::server::Router<Stack<RoleLayer, Identity>>;

/// Every slice-1 service behind one role layer. `AdapterService` is served by adapters, not the engine.
pub(crate) fn router(engine: Arc<Engine>, layer: RoleLayer) -> Router {
    tonic::transport::Server::builder()
        .layer(layer)
        .add_service(EngineServiceServer::new(engine::EngineSvc(Arc::clone(&engine))))
        .add_service(TargetServiceServer::new(target::TargetSvc(Arc::clone(&engine))))
        .add_service(DistillServiceServer::new(distill::DistillSvc(Arc::clone(&engine))))
        .add_service(CatalogServiceServer::new(catalog::CatalogSvc(Arc::clone(&engine))))
        .add_service(RuntimeServiceServer::new(runtime::RuntimeSvc(Arc::clone(&engine))))
        .add_service(PolicyServiceServer::new(policy::PolicySvc))
        .add_service(AuditServiceServer::new(audit::AuditSvc(engine)))
}

pub(crate) type RpcResult<T> = Result<tonic::Response<T>, tonic::Status>;

pub(crate) fn unimplemented(rpc: &str) -> tonic::Status {
    let err = aa_core::AaError::new(
        pb::ErrorCode::Unspecified,
        format!("{rpc} is not implemented in this release"),
        "aa status",
    );
    crate::role_layer::status_for(tonic::Code::Unimplemented, &err)
}

/// Offset paging over an already-materialized list (`limit == 0` => `default_limit`).
pub(crate) fn page<T>(items: Vec<T>, page: Option<&pb::PageRequest>, default_limit: u32) -> (Vec<T>, pb::PageInfo) {
    let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let offset = page.map_or(0, |p| p.offset);
    let limit = match page.map_or(0, |p| p.limit) {
        0 => default_limit,
        l => l,
    };
    let start = (offset as usize).min(items.len());
    let end = start.saturating_add(limit as usize).min(items.len());
    let next_offset = if end < items.len() {
        u32::try_from(end).unwrap_or(0)
    } else {
        0
    };
    let out = items.into_iter().skip(start).take(end - start).collect();
    (out, pb::PageInfo { next_offset, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging() {
        let (v, p) = page(
            (0..10).collect::<Vec<_>>(),
            Some(&pb::PageRequest { offset: 3, limit: 4 }),
            100,
        );
        assert_eq!(v, vec![3, 4, 5, 6]);
        assert_eq!((p.next_offset, p.total), (7, 10));
        let (v, p) = page((0..10).collect::<Vec<_>>(), None, 100);
        assert_eq!(v.len(), 10);
        assert_eq!(p.next_offset, 0);
        let (v, _) = page(
            (0..3).collect::<Vec<_>>(),
            Some(&pb::PageRequest { offset: 9, limit: 0 }),
            100,
        );
        assert!(v.is_empty());
    }
}
