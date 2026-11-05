//! `aa audit [--last N] [--action id] [--verify]` (AuditService).

use super::{agent, client_error};
use crate::connect::ClientConfig;
use crate::exit::CliError;
use crate::output::{CmdResult, Reply, to_json};
use aa_proto::v1 as pb;
use pb::audit_service_client::AuditServiceClient;
use serde_json::json;

/// Entries shown when neither `--last` nor `--action` is given.
pub const DEFAULT_LAST: u64 = 20;
const PAGE: u32 = 500;
const MAX_PAGES: usize = 100_000;

#[derive(Debug, clap::Args)]
pub struct AuditArgs {
    /// Show the last N entries (default 20 unless --action is given).
    #[arg(long)]
    pub last: Option<u64>,
    /// Show only the entries of this action (intent + record).
    #[arg(long)]
    pub action: Option<String>,
    /// Verify the hash chain and the HEAD mirror; exit 1 if broken.
    #[arg(long)]
    pub verify: bool,
}

pub async fn run(a: AuditArgs, cfg: &ClientConfig) -> CmdResult {
    let mut c = AuditServiceClient::new(agent(cfg).await?);
    let last = match (a.last, &a.action) {
        (Some(n), _) => Some(n),
        (None, Some(_)) => None,
        (None, None) => Some(DEFAULT_LAST),
    };

    // VerifyChain also tells us how many entries exist, so `--last` can start near the end.
    let chain = if a.verify || last.is_some() {
        Some(c.verify_chain(pb::VerifyChainRequest {}).await?.into_inner())
    } else {
        None
    };
    let start_after = match (last, &chain) {
        (Some(n), Some(ch)) => ch.entries.saturating_sub(n),
        _ => 0,
    };

    let mut entries: Vec<pb::AuditEntry> = Vec::new();
    let mut after_seq = start_after;
    for _ in 0..MAX_PAGES {
        let r = c
            .query_audit(pb::QueryAuditRequest {
                target_id: String::new(),
                capability_id: String::new(),
                action_id: a.action.clone().unwrap_or_default(),
                after_seq,
                limit: PAGE,
                actions_only: false,
            })
            .await?
            .into_inner();
        let max_seq = r.entries.iter().map(|e| e.seq).max().unwrap_or(after_seq);
        let empty = r.entries.is_empty();
        entries.extend(r.entries);
        if empty || r.next_seq == 0 || max_seq <= after_seq {
            break;
        }
        after_seq = max_seq;
    }
    if let Some(n) = last {
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        if entries.len() > n {
            entries.drain(..entries.len() - n);
        }
    }

    let mut data = json!({ "entries": entries.iter().map(to_json).collect::<Vec<_>>() });
    if a.verify
        && let Some(ch) = &chain
    {
        data["chain"] = to_json(ch);
        if !ch.intact || !ch.head_matches {
            let mut err = ch.error.clone().filter(|e| e.code != 0).unwrap_or_else(|| {
                let msg = if ch.intact {
                    "audit HEAD does not match the last entry (truncated log?)".to_string()
                } else {
                    format!("audit chain broken at seq {}", ch.first_broken_seq)
                };
                let mut e = client_error(
                    pb::ErrorCode::Internal,
                    msg,
                    "inspect the log; the engine halts on chain failure: aa status",
                );
                e.category = pb::ErrorCategory::Integrity as i32;
                e
            });
            if err.details_json.is_empty() {
                err.details_json = serde_json::to_string(&data["chain"]).unwrap_or_default();
            }
            return Err(CliError::Domain(err).into());
        }
    }
    Ok(Reply::new(data))
}
