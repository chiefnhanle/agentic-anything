//! `aa list` and `aa describe` (CatalogService).

use super::agent;
use crate::connect::ClientConfig;
use crate::exit::CliError;
use crate::output::{CmdResult, Reply, to_json};
use aa_proto::v1 as pb;
use pb::catalog_service_client::CatalogServiceClient;

/// Hard stop for a server that never ends its paging.
const MAX_PAGES: usize = 10_000;

#[derive(Debug, clap::Args)]
pub struct ListArgs {
    /// Target id; default: all targets.
    pub target: Option<String>,
    /// Only capabilities that are SERVABLE now.
    #[arg(long)]
    pub servable: bool,
}

#[derive(Debug, clap::Args)]
pub struct DescribeArgs {
    /// Capability id, e.g. `git.status`.
    pub capability: String,
}

/// All capability summaries for `target` ("" = all), following the offset paging to the end.
pub(crate) async fn list_all(
    channel: tonic::transport::Channel,
    target: &str,
    servable_only: bool,
) -> Result<Vec<pb::CapabilitySummary>, CliError> {
    let mut c = CatalogServiceClient::new(channel);
    let mut caps = Vec::new();
    let mut offset = 0u32;
    for _ in 0..MAX_PAGES {
        let r = c
            .list_capabilities(pb::ListCapabilitiesRequest {
                target_id: target.to_string(),
                family: String::new(),
                servable_only,
                max_level: pb::EffectLevel::Unknown as i32,
                page: Some(pb::PageRequest { offset, limit: 0 }),
            })
            .await?
            .into_inner();
        let n = r.capabilities.len();
        caps.extend(r.capabilities);
        match r.page {
            Some(p) if p.next_offset > offset && n > 0 => offset = p.next_offset,
            _ => break,
        }
    }
    Ok(caps)
}

pub async fn list(a: ListArgs, cfg: &ClientConfig) -> CmdResult {
    let caps = list_all(agent(cfg).await?, a.target.as_deref().unwrap_or(""), a.servable).await?;
    Ok(Reply::new(
        serde_json::json!({ "capabilities": caps.iter().map(to_json).collect::<Vec<_>>() }),
    ))
}

pub async fn describe(a: DescribeArgs, cfg: &ClientConfig) -> CmdResult {
    let mut c = CatalogServiceClient::new(agent(cfg).await?);
    let r = c
        .describe_capability(pb::DescribeCapabilityRequest {
            capability_id: a.capability,
        })
        .await?
        .into_inner();
    Ok(Reply::new(to_json(&r)))
}
