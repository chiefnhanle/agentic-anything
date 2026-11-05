//! `aa discover` (Harvest + Compile) and `aa verify` (Verify) (DistillService).

use super::{agent, catalog::list_all, client_error, short_name};
use crate::connect::ClientConfig;
use crate::exit::CliError;
use crate::output::{CmdResult, Failure, Output, Reply, to_json};
use aa_proto::v1 as pb;
use pb::distill_service_client::DistillServiceClient;
use serde_json::{Map, Value, json};

#[derive(Debug, clap::Args)]
pub struct DiscoverArgs {
    /// Target id, e.g. `git`.
    pub target: String,
    /// Restrict to these harvesters (repeatable); default: all applicable.
    #[arg(long = "harvester")]
    pub harvesters: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct VerifyArgs {
    /// Target id, e.g. `git`.
    pub target: String,
    /// Verify only these capability ids (repeatable); default: all.
    #[arg(long = "only")]
    pub only: Vec<String>,
    /// Run only these lanes (repeatable): static|parser|dry-run|sandbox-diff|read-back, or L0..L8.
    #[arg(long = "lane", value_parser = parse_lane)]
    pub lanes: Vec<pb::Lane>,
    /// Tools the lanes need (repeatable); a missing tool fails the lane instead of skipping it.
    #[arg(long = "require-tools")]
    pub require_tools: Vec<String>,
}

/// `sandbox-diff`, `sandbox_diff`, `LANE_SANDBOX_DIFF` and `L3` all name the same lane.
pub fn parse_lane(s: &str) -> Result<pb::Lane, String> {
    const BY_LEVEL: [pb::Lane; 9] = [
        pb::Lane::Static,
        pb::Lane::Parser,
        pb::Lane::DryRun,
        pb::Lane::SandboxDiff,
        pb::Lane::ReadBack,
        pb::Lane::Golden,
        pb::Lane::Differential,
        pb::Lane::Property,
        pb::Lane::Agent,
    ];
    let t = s.trim();
    if let Some(n) = t.strip_prefix(['L', 'l']).and_then(|n| n.parse::<usize>().ok()) {
        return BY_LEVEL.get(n).copied().ok_or_else(|| format!("no lane L{n}"));
    }
    let upper = t.to_ascii_uppercase().replace('-', "_");
    let name = if upper.starts_with("LANE_") {
        upper
    } else {
        format!("LANE_{upper}")
    };
    match pb::Lane::from_str_name(&name) {
        Some(pb::Lane::Unspecified) | None => Err(format!("unknown lane {s:?}")),
        Some(l) => Ok(l),
    }
}

/// Counts withheld reasons over capability summaries, e.g. `{"MIXED_AUTHORITY": 2, "NOT_VERIFIED": 1}`.
pub fn withheld_by_reason(caps: &[pb::CapabilitySummary]) -> Map<String, Value> {
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    for c in caps {
        if c.admission() == pb::AdmissionState::Servable {
            continue;
        }
        for w in &c.withheld {
            *counts
                .entry(short_name(w.reason().as_str_name(), "WITHHELD_REASON_").to_string())
                .or_default() += 1;
        }
    }
    counts.into_iter().map(|(k, v)| (k, json!(v))).collect()
}

fn admission_counts(caps: &[pb::CapabilitySummary]) -> (usize, usize, usize) {
    let servable = caps
        .iter()
        .filter(|c| c.admission() == pb::AdmissionState::Servable)
        .count();
    let excluded = caps
        .iter()
        .filter(|c| c.admission() == pb::AdmissionState::Excluded)
        .count();
    (servable, caps.len() - servable - excluded, excluded)
}

/// Per-capability undecided effect fields, from the `EFFECTS_UNKNOWN` withheld hints.
fn unknown_by_capability(caps: &[pb::CapabilitySummary]) -> Map<String, Value> {
    caps.iter()
        .filter_map(|c| {
            let hints: Vec<&str> = c
                .withheld
                .iter()
                .filter(|w| w.reason() == pb::WithheldReason::EffectsUnknown)
                .map(|w| w.hint.as_str())
                .collect();
            (!hints.is_empty()).then(|| (c.id.clone(), json!(hints.join("; "))))
        })
        .collect()
}

pub async fn discover(a: DiscoverArgs, cfg: &ClientConfig, out: &Output) -> CmdResult {
    let channel = agent(cfg).await?;
    let mut c = DistillServiceClient::new(channel.clone());

    // 1. Harvest (stream): per-harvester errors do not stop the harvest.
    let mut stream = c
        .harvest(pb::HarvestRequest {
            target_id: a.target.clone(),
            harvesters: a.harvesters.clone(),
            max_authority: pb::DiscoveryAuthority::Unspecified as i32,
            allow_network: false,
        })
        .await?
        .into_inner();
    let mut action_ids: Vec<String> = Vec::new();
    let mut sources = Vec::new();
    let mut errors: Vec<pb::Error> = Vec::new();
    let mut summary = None;
    while let Some(msg) = stream.message().await.map_err(|s| failure(s, &action_ids))? {
        if !msg.action_id.is_empty() && !action_ids.contains(&msg.action_id) {
            action_ids.push(msg.action_id.clone());
        }
        match msg.event {
            Some(pb::harvest_response::Event::Progress(p)) => out.progress(&p),
            Some(pb::harvest_response::Event::Source(s)) => sources.push(s),
            Some(pb::harvest_response::Event::Error(e)) => {
                out.progress(&format!("harvest error: {}", e.message));
                errors.push(e);
            }
            Some(pb::harvest_response::Event::Summary(s)) => {
                summary = Some(s);
                break;
            }
            None => {}
        }
    }
    let Some(summary) = summary else {
        let err = errors.into_iter().next().unwrap_or_else(|| {
            client_error(
                pb::ErrorCode::ExecutionStateUnknown,
                "the harvest stream ended without a summary",
                format!("inspect the pipeline actions: aa audit --last 20 (target {})", a.target),
            )
        });
        return Err(failure_domain(err, &action_ids));
    };
    if summary.sources.is_empty()
        && sources.is_empty()
        && let Some(err) = errors.first().cloned()
    {
        return Err(failure_domain(err, &action_ids));
    }

    // 2. Compile (pure).
    let compiled = c
        .compile(pb::CompileRequest {
            target_id: a.target.clone(),
            check: false,
        })
        .await
        .map_err(|s| {
            // The harvest result explains most compile failures (e.g. a skipped or failing harvester).
            let note = json!({
                "harvest": {
                    "sources": sources.len(),
                    "candidateCount": summary.candidate_count,
                    "skippedHarvesters": summary.skipped_harvesters,
                    "errors": errors.iter().map(to_json).collect::<Vec<_>>(),
                }
            });
            eprintln!("aa: discover {}: compile failed after harvest: {note}", a.target);
            failure(s, &action_ids)
        })?
        .into_inner();

    // 3. The compiled catalog for this target.
    let caps = list_all(channel, &a.target, false).await.map_err(|e| Failure {
        error: e,
        action_id: action_ids.last().cloned(),
    })?;
    let (servable, withheld, excluded) = admission_counts(&caps);
    let data = json!({
        "target": a.target,
        "harvest": {
            "actionIds": action_ids,
            "sources": sources.iter().map(to_json).collect::<Vec<_>>(),
            "errors": errors.iter().map(to_json).collect::<Vec<_>>(),
            "summary": to_json(&summary),
        },
        "compile": to_json(&compiled),
        "counts": {
            "candidates": caps.len(),
            "harvestedCandidates": summary.candidate_count,
            "unknownBits": compiled.effects_unknown_count,
            "unresolvedBranches": compiled.unresolved_branch_count,
            "writesWithoutOracle": compiled.writes_without_oracle_count,
            "servable": servable,
            "withheld": withheld,
            "excluded": excluded,
            "withheldByReason": withheld_by_reason(&caps),
        },
        "unknownByCapability": unknown_by_capability(&caps),
        "capabilities": caps.iter().map(to_json).collect::<Vec<_>>(),
    });
    Ok(Reply {
        data,
        action_id: action_ids.last().cloned(),
    })
}

pub async fn verify(a: VerifyArgs, cfg: &ClientConfig, out: &Output) -> CmdResult {
    let channel = agent(cfg).await?;
    let mut c = DistillServiceClient::new(channel.clone());
    let mut stream = c
        .verify(pb::VerifyRequest {
            target_id: a.target.clone(),
            capability_ids: a.only.clone(),
            lanes: a.lanes.iter().map(|l| *l as i32).collect(),
            require_tools: a.require_tools.clone(),
            min_isolation: pb::Isolation::NotProven as i32,
        })
        .await?
        .into_inner();
    let mut records = Vec::new();
    let mut summary = None;
    while let Some(msg) = stream.message().await? {
        match msg.event {
            Some(pb::verify_response::Event::Progress(p)) => out.progress(&p),
            Some(pb::verify_response::Event::Record(r)) => {
                out.progress(&format!(
                    "{} {}: {}",
                    r.capability_id,
                    short_name(r.lane().as_str_name(), "LANE_"),
                    short_name(r.status().as_str_name(), "LANE_STATUS_")
                ));
                records.push(r);
            }
            Some(pb::verify_response::Event::Summary(s)) => {
                summary = Some(s);
                break;
            }
            None => {}
        }
    }
    let Some(summary) = summary else {
        return Err(CliError::Domain(client_error(
            pb::ErrorCode::ExecutionStateUnknown,
            "the verify stream ended without a summary",
            format!("re-run: aa verify {}", a.target),
        ))
        .into());
    };
    let caps = list_all(channel, &a.target, false).await?;
    let (servable, withheld, excluded) = admission_counts(&caps);
    let data = json!({
        "target": a.target,
        "summary": to_json(&summary),
        "counts": {
            "servable": servable,
            "withheld": withheld,
            "excluded": excluded,
            "withheldByReason": withheld_by_reason(&caps),
        },
        "records": records.iter().map(to_json).collect::<Vec<_>>(),
        "capabilities": caps.iter().map(to_json).collect::<Vec<_>>(),
    });
    Ok(Reply::new(data))
}

fn failure(s: tonic::Status, action_ids: &[String]) -> Failure {
    Failure {
        error: CliError::Status(s),
        action_id: action_ids.last().cloned(),
    }
}

fn failure_domain(e: pb::Error, action_ids: &[String]) -> Failure {
    Failure {
        error: CliError::Domain(e),
        action_id: action_ids.last().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_parse() {
        assert_eq!(parse_lane("L0"), Ok(pb::Lane::Static));
        assert_eq!(parse_lane("l3"), Ok(pb::Lane::SandboxDiff));
        assert_eq!(parse_lane("sandbox-diff"), Ok(pb::Lane::SandboxDiff));
        assert_eq!(parse_lane("read_back"), Ok(pb::Lane::ReadBack));
        assert_eq!(parse_lane("LANE_PARSER"), Ok(pb::Lane::Parser));
        assert!(parse_lane("unspecified").is_err());
        assert!(parse_lane("L9").is_err());
        assert!(parse_lane("bogus").is_err());
    }

    #[test]
    fn withheld_counts() {
        let w = |r: pb::WithheldReason| pb::Withheld {
            reason: r as i32,
            hint: "h".into(),
            evidence_ids: vec![],
        };
        let caps = vec![
            pb::CapabilitySummary {
                id: "git.branch".into(),
                admission: pb::AdmissionState::Withheld as i32,
                withheld: vec![w(pb::WithheldReason::MixedAuthority)],
                ..Default::default()
            },
            pb::CapabilitySummary {
                id: "git.add".into(),
                admission: pb::AdmissionState::Withheld as i32,
                withheld: vec![
                    w(pb::WithheldReason::NotVerified),
                    w(pb::WithheldReason::EffectsUnknown),
                ],
                ..Default::default()
            },
            pb::CapabilitySummary {
                id: "git.status".into(),
                admission: pb::AdmissionState::Servable as i32,
                ..Default::default()
            },
        ];
        let m = withheld_by_reason(&caps);
        assert_eq!(m["MIXED_AUTHORITY"], 1);
        assert_eq!(m["NOT_VERIFIED"], 1);
        assert_eq!(m["EFFECTS_UNKNOWN"], 1);
        assert_eq!(admission_counts(&caps), (1, 2, 0));
        assert_eq!(unknown_by_capability(&caps)["git.add"], "h");
    }
}
