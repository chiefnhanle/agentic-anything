//! L3 sandbox-diff lane: run ≥ 2 examples in fresh fixtures, under confinement when available, and diff the
//! private root (`home/`, `cwd/`) and the state handle (`tmp/` is ignored).
//!
//! Signals (all `CONFIDENCE_OBSERVED`, at the isolation the runs actually had):
//! - `mutates_target`/`writes_filesystem` FALSE when every run succeeded and every diff is empty;
//! - `accesses_network`/`launches_process` FALSE only when every run was CONFINED, succeeded, and reported no
//!   confinement denial;
//! - TRUE for each observed change: home/cwd changes → `writes_filesystem`; state-handle changes →
//!   `mutates_target` and `writes_filesystem`; a denied exec → `launches_process`; any other EPERM denial →
//!   `writes_filesystem`.
//!
//! Findings: `SIDE_EFFECT` for each TRUE field not declared TRUE; `UNCONFIRMED` for an example that did not
//! succeed; `UNEXPLAINED_CHANGE` for process-group survivors. PASSED requires no findings.

use super::{
    LaneCtx, NON_CLAIM_PROCESS, NON_CLAIM_REMOTE, declared, finding, finish, parse_input, record, run_example, signal,
};
use aa_core::pb;
use std::collections::BTreeMap;

const MIN_EXAMPLES: usize = 2;

/// L3.
pub async fn run_sandbox_diff_lane(
    ctx: &LaneCtx<'_>,
    cap: &aa_core::pb::Capability,
) -> aa_core::pb::VerificationRecord {
    let mut rec = record(ctx, cap, pb::Lane::SandboxDiff);
    rec.non_claims = vec![
        NON_CLAIM_REMOTE.to_string(),
        "only the example inputs' code paths were exercised".to_string(),
    ];
    if cap.examples.len() < MIN_EXAMPLES {
        return finish(rec, pb::LaneStatus::Skipped, "needs ≥2 examples");
    }

    let mut isolation = pb::Isolation::Confined;
    let mut all_ok = true;
    let mut denials: Vec<String> = Vec::new();
    let mut observed: Vec<pb::ObservedFileEffect> = Vec::new();
    for (i, ex) in cap.examples.iter().enumerate() {
        let input = match parse_input(&ex.input_json) {
            Ok(v) => v,
            Err(e) => return finish(rec, pb::LaneStatus::Error, format!("example {i}: {}", e.message)),
        };
        let run = match run_example(ctx, cap, &input, &ex.fixture_id, None).await {
            Ok(r) => r,
            Err(e) => return finish(rec, pb::LaneStatus::Error, format!("example {i}: {}", e.message)),
        };
        let out = &run.out;
        if out.isolation != pb::Isolation::Confined {
            isolation = out.isolation;
        }
        for d in &out.confinement_denials {
            if !denials.contains(d) {
                denials.push(d.clone());
            }
        }
        let ok = out.exit_code == Some(0) && !out.timed_out && out.cleanup == pb::CleanupStatus::Complete;
        if !ok {
            all_ok = false;
            if out.cleanup != pb::CleanupStatus::Complete {
                rec.findings.push(finding(
                    pb::WithheldReason::UnexplainedChange,
                    format!("example {i}: processes of the owned group survived ({:?})", out.cleanup),
                ));
            }
            if out.exit_code != Some(0) || out.timed_out {
                rec.findings.push(finding(
                    pb::WithheldReason::Unconfirmed,
                    format!(
                        "example {i} did not succeed (exit {:?}, timed out {}): {}",
                        out.exit_code,
                        out.timed_out,
                        out.stderr_excerpt.lines().next().unwrap_or("").trim()
                    ),
                ));
            }
        }
        observed.extend(run.observed);
    }
    rec.isolation = isolation as i32;
    if isolation != pb::Isolation::Confined {
        rec.non_claims.push(NON_CLAIM_PROCESS.to_string());
    }

    // field -> evidence lines
    let mut raised: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for o in &observed {
        let what = format!("{}:{} {}", o.root, o.path, change_name(o.change));
        raised.entry("writes_filesystem").or_default().push(what.clone());
        if o.root == "scope:state_handle" {
            raised.entry("mutates_target").or_default().push(what);
        }
    }
    for d in &denials {
        let field = if d.starts_with("process-exec:") {
            "launches_process"
        } else {
            "writes_filesystem"
        };
        raised.entry(field).or_default().push(format!("confinement denial {d}"));
    }

    let n = cap.examples.len();
    for (field, evidence) in &raised {
        let shown: Vec<&str> = evidence.iter().take(5).map(String::as_str).collect();
        let more = evidence.len().saturating_sub(shown.len());
        let mut why = shown.join(", ");
        if more > 0 {
            why.push_str(&format!(" (+{more} more)"));
        }
        rec.effect_signals
            .push(signal(field, pb::Tri::True, isolation, format!("observed: {why}")));
        if declared(cap.effects.as_ref(), field) != pb::Tri::True {
            rec.findings.push(finding(
                pb::WithheldReason::SideEffect,
                format!("observed {field} TRUE, which is not declared: {why}"),
            ));
        }
    }
    if all_ok && observed.is_empty() && denials.is_empty() {
        for field in ["mutates_target", "writes_filesystem"] {
            let scope = if isolation == pb::Isolation::Confined {
                "writes outside the root were denied and none were attempted"
            } else {
                "writes outside the root were not observable"
            };
            rec.effect_signals.push(signal(
                field,
                pb::Tri::False,
                isolation,
                format!("{n} examples; private-root and state-handle tree diff empty; {scope}"),
            ));
        }
    }
    if all_ok && isolation == pb::Isolation::Confined && denials.is_empty() {
        rec.effect_signals.push(signal(
            "accesses_network",
            pb::Tri::False,
            isolation,
            format!("{n} examples succeeded with network-outbound denied and no denial reported"),
        ));
        rec.effect_signals.push(signal(
            "launches_process",
            pb::Tri::False,
            isolation,
            format!("{n} examples succeeded with the exec allow-list = pinned identity files only"),
        ));
    }
    rec.observed_files = observed;
    let status = if rec.findings.is_empty() && all_ok {
        pb::LaneStatus::Passed
    } else {
        pb::LaneStatus::Failed
    };
    finish(rec, status, "")
}

fn change_name(c: i32) -> &'static str {
    match pb::FileChange::try_from(c) {
        Ok(pb::FileChange::Created) => "CREATED",
        Ok(pb::FileChange::Modified) => "MODIFIED",
        Ok(pb::FileChange::Deleted) => "DELETED",
        _ => "CHANGED",
    }
}
