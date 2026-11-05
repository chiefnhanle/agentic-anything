//! CatalogService: ListCapabilities, DescribeCapability, SearchCapabilities. GetExposure and WatchCatalog
//! are slice 2.

use super::{RpcResult, page, unimplemented};
use crate::{Engine, role_layer::principal_of, to_status};
use aa_core::{AaResult, pb};
use aa_proto::v1::catalog_service_server::CatalogService;
use std::sync::Arc;

pub(crate) struct CatalogSvc(pub Arc<Engine>);

fn summary(c: &pb::Capability) -> pb::CapabilitySummary {
    let adm = c.admission.clone().unwrap_or_default();
    let proj = c.projections.clone().unwrap_or_default();
    pb::CapabilitySummary {
        id: c.id.clone(),
        title: c.title.clone(),
        level: c.level,
        tier: c.tier,
        admission: adm.state,
        withheld: adm.withheld,
        mcp_name: proj.mcp_name,
        cli_path: proj.cli_path,
    }
}

impl Engine {
    /// Capabilities of one target (or all), in target then lock order.
    pub(crate) fn capabilities(&self, target_id: &str) -> AaResult<Vec<pb::Capability>> {
        let ids = if target_id.is_empty() {
            self.store.list_targets()?
        } else {
            vec![target_id.to_string()]
        };
        let mut out = Vec::new();
        for id in ids {
            if let Some(lock) = self.store.read_lock(&id)? {
                out.extend(lock.capabilities);
            }
        }
        Ok(out)
    }
}

#[tonic::async_trait]
impl CatalogService for CatalogSvc {
    type WatchCatalogStream = tokio_stream::wrappers::ReceiverStream<Result<pb::WatchCatalogResponse, tonic::Status>>;

    async fn list_capabilities(
        &self,
        request: tonic::Request<pb::ListCapabilitiesRequest>,
    ) -> RpcResult<pb::ListCapabilitiesResponse> {
        principal_of(&request)?;
        let r = request.into_inner();
        if !r.target_id.is_empty()
            && self
                .0
                .store
                .read_target(&r.target_id)
                .map_err(|e| to_status(&e))?
                .is_none()
        {
            return Err(to_status(&crate::ports::not_found(
                format!("unknown target {:?}", r.target_id),
                "aa target add <id> ...",
            )));
        }
        let caps = self.0.capabilities(&r.target_id).map_err(|e| to_status(&e))?;
        let filtered: Vec<pb::CapabilitySummary> = caps
            .iter()
            .filter(|c| r.family.is_empty() || c.family == r.family)
            .filter(|c| {
                !r.servable_only
                    || c.admission
                        .as_ref()
                        .is_some_and(|a| a.state() == pb::AdmissionState::Servable)
            })
            .filter(|c| r.max_level == 0 || (c.level != 0 && c.level <= r.max_level))
            .map(summary)
            .collect();
        let (capabilities, page) = page(filtered, r.page.as_ref(), 500);
        Ok(tonic::Response::new(pb::ListCapabilitiesResponse {
            capabilities,
            page: Some(page),
        }))
    }

    async fn search_capabilities(
        &self,
        request: tonic::Request<pb::SearchCapabilitiesRequest>,
    ) -> RpcResult<pb::SearchCapabilitiesResponse> {
        principal_of(&request)?;
        let r = request.into_inner();
        let q = r.query.to_lowercase();
        let limit = if r.limit == 0 { 20 } else { r.limit.min(500) } as usize;
        let text = |t: &Option<pb::TargetText>| t.as_ref().map(|t| t.text.to_lowercase()).unwrap_or_default();
        let results = self
            .0
            .capabilities(&r.target_id)
            .map_err(|e| to_status(&e))?
            .iter()
            .filter(|c| {
                q.is_empty()
                    || c.id.to_lowercase().contains(&q)
                    || text(&c.title).contains(&q)
                    || text(&c.description).contains(&q)
            })
            .take(limit)
            .map(summary)
            .collect();
        Ok(tonic::Response::new(pb::SearchCapabilitiesResponse { results }))
    }

    async fn describe_capability(
        &self,
        request: tonic::Request<pb::DescribeCapabilityRequest>,
    ) -> RpcResult<pb::DescribeCapabilityResponse> {
        principal_of(&request)?;
        let id = &request.get_ref().capability_id;
        let resolved = self.0.resolve(id).map_err(|e| to_status(&e))?;
        let target_id = resolved.target().id.clone();
        let records = self
            .0
            .store
            .read_verifications(&target_id)
            .map_err(|e| to_status(&e))?
            .into_iter()
            .filter(|r| &r.capability_id == id)
            .collect();
        Ok(tonic::Response::new(pb::DescribeCapabilityResponse {
            capability: Some(resolved.cap().clone()),
            records,
        }))
    }

    async fn get_exposure(
        &self,
        request: tonic::Request<pb::GetExposureRequest>,
    ) -> RpcResult<pb::GetExposureResponse> {
        principal_of(&request)?;
        Err(unimplemented("CatalogService.GetExposure"))
    }

    async fn watch_catalog(
        &self,
        request: tonic::Request<pb::WatchCatalogRequest>,
    ) -> RpcResult<Self::WatchCatalogStream> {
        principal_of(&request)?;
        Err(unimplemented("CatalogService.WatchCatalog"))
    }
}
