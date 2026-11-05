//! `aa status`, `aa halt`, `aa unhalt` (EngineService).

use super::agent;
use crate::connect::{ClientConfig, connect};
use crate::exit::CliError;
use crate::output::{CmdResult, Reply, to_json};
use aa_proto::v1 as pb;
use pb::engine_service_client::EngineServiceClient;

#[derive(Debug, clap::Args)]
pub struct HaltArgs {
    /// Why the brake was pulled (recorded in the audit log).
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct UnhaltArgs {
    /// The current halt epoch (`aa status --json | jq .halt.haltEpoch`; surrounding quotes are accepted).
    #[arg(long, value_parser = parse_epoch)]
    pub epoch: u64,
    /// Why the brake is released (recorded in the audit log).
    #[arg(long)]
    pub reason: Option<String>,
    /// Test-only: send the request over agent.sock to prove the role split refuses it.
    #[arg(long, hide = true)]
    pub via_agent_socket: bool,
}

/// Accepts `7` and `"7"` (proto3-JSON renders uint64 as a string, so `jq .halt.haltEpoch` prints `"7"`).
pub fn parse_epoch(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let t = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(t);
    t.parse::<u64>()
        .map_err(|e| format!("expected a non-negative integer epoch, got {s:?}: {e}"))
}

pub async fn status(cfg: &ClientConfig) -> CmdResult {
    let mut c = EngineServiceClient::new(agent(cfg).await?);
    let r = c.get_status(pb::GetStatusRequest {}).await?.into_inner();
    Ok(Reply::new(to_json(&r)))
}

/// Fetches the status (used by other commands, e.g. to find the workspace root).
pub(crate) async fn get_status(cfg: &ClientConfig) -> Result<pb::GetStatusResponse, CliError> {
    let mut c = EngineServiceClient::new(agent(cfg).await?);
    Ok(c.get_status(pb::GetStatusRequest {}).await?.into_inner())
}

pub async fn halt(a: HaltArgs, cfg: &ClientConfig) -> CmdResult {
    let mut c = EngineServiceClient::new(agent(cfg).await?);
    let r = c
        .halt(pb::HaltRequest {
            scope: pb::HaltScope::Engine as i32,
            scope_subject: String::new(),
            reason: a.reason.unwrap_or_default(),
        })
        .await?
        .into_inner();
    Ok(Reply::new(to_json(&r)))
}

pub async fn unhalt(a: UnhaltArgs, cfg: &ClientConfig) -> CmdResult {
    let channel = connect(cfg, !a.via_agent_socket).await?;
    let mut c = EngineServiceClient::new(channel);
    let r = c
        .unhalt(pb::UnhaltRequest {
            halt_epoch: a.epoch,
            reason: a.reason.unwrap_or_default(),
        })
        .await?
        .into_inner();
    Ok(Reply::new(to_json(&r)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_parsing() {
        assert_eq!(parse_epoch("7"), Ok(7));
        assert_eq!(parse_epoch("\"7\""), Ok(7));
        assert_eq!(parse_epoch(" 12 "), Ok(12));
        assert!(parse_epoch("-1").is_err());
        assert!(parse_epoch("x").is_err());
        assert!(parse_epoch("").is_err());
    }
}
