//! Admission: which verification lanes a capability needs, and whether its records satisfy them
//! (architecture §9, ADR 0011).
//!
//! A record counts only if it is bound to the capability's current `capability_digest` and the target's current
//! `identity_digest` (and to `lock_digest`, when one is given). Only `PASSED` records without blocking findings
//! count as coverage; `SKIPPED`, `ERROR` and `NOT_RUN` never do.
//!
//! Lane observations enter the effects here: every matching record's `effect_signals` (normalized to
//! `verify:<record_id>`), plus the lowering the ADR 0011 table grants a PASSED, change-free L3 run at its
//! isolation. TRUE observations count from any record that ran.

use crate::{effects, pb};

/// Withheld reasons decided at compile time (not by lanes); [`compute`] keeps them from `cap.admission`.
pub const STICKY_REASONS: &[pb::WithheldReason] = &[
    pb::WithheldReason::Interactive,
    pb::WithheldReason::MixedAuthority,
    pb::WithheldReason::ExcludedByPolicy,
    pb::WithheldReason::OperatorExcluded,
    pb::WithheldReason::PreconditionUnmet,
];

/// A compile-time withheld entry (sticky reason, no lane evidence attached).
pub fn is_sticky(w: &pb::Withheld) -> bool {
    w.evidence_ids.is_empty() && pb::WithheldReason::try_from(w.reason).is_ok_and(|r| STICKY_REASONS.contains(&r))
}

/// Whether the capability has a path-typed WRITE parameter (blocks lowering `writes_filesystem`).
pub fn has_write_path_param(cap: &pb::Capability) -> bool {
    cap.scopes.iter().any(|s| s.kind == pb::ScopeKind::WritePath as i32)
}

fn is_cli(cap: &pb::Capability) -> bool {
    matches!(
        cap.binding.as_ref().and_then(|b| b.kind.as_ref()),
        Some(pb::binding::Kind::Cli(_))
    )
}

/// The admission matrix. UNKNOWN needs nothing because it is never servable. UI requires an oracle-backed lane
/// (the a11y oracle is M2+), so it is not servable in slice 1.
pub fn required_lanes(level: pb::EffectLevel, cap: &pb::Capability) -> Vec<pb::Lane> {
    let mut read = vec![pb::Lane::Static];
    if is_cli(cap) {
        read.push(pb::Lane::Parser);
    }
    read.push(pb::Lane::SandboxDiff);
    match level {
        pb::EffectLevel::Unknown => Vec::new(),
        pb::EffectLevel::Read => read,
        pb::EffectLevel::Ui => vec![pb::Lane::Static, pb::Lane::SandboxDiff, pb::Lane::ReadBack],
        pb::EffectLevel::Write | pb::EffectLevel::Destructive => {
            if cap.dry_run.as_ref().is_some_and(|d| d.supported) {
                read.push(pb::Lane::DryRun);
            }
            read.push(pb::Lane::ReadBack);
            read
        }
    }
}

/// Whether a record is a passing one (PASSED, no blocking findings).
pub fn passing(r: &pb::VerificationRecord) -> bool {
    r.status == pb::LaneStatus::Passed as i32
        && r.findings
            .iter()
            .all(|f| f.reason == pb::WithheldReason::Unspecified as i32)
}

fn ran(r: &pb::VerificationRecord) -> bool {
    matches!(
        pb::LaneStatus::try_from(r.status),
        Ok(pb::LaneStatus::Passed | pb::LaneStatus::Failed)
    )
}

/// The records bound to this capability's current digests (`lock_digest == ""` = not checked).
pub fn matching_records<'a>(
    cap: &pb::Capability,
    records: &'a [pb::VerificationRecord],
    lock_digest: &str,
    identity_digest: &str,
) -> Vec<&'a pb::VerificationRecord> {
    records
        .iter()
        .filter(|r| {
            r.capability_id == cap.id
                && !cap.capability_digest.is_empty()
                && r.capability_digest == cap.capability_digest
                && r.identity_digest == identity_digest
                && (lock_digest.is_empty() || r.lock_digest == lock_digest)
        })
        .collect()
}

fn ts(r: &pb::VerificationRecord) -> (i64, i32) {
    r.ran_at.as_ref().map_or((0, 0), |t| (t.seconds, t.nanos))
}

/// The latest record per lane (by `ran_at`, then `record_id`).
pub fn latest_per_lane<'a>(
    records: &[&'a pb::VerificationRecord],
) -> std::collections::BTreeMap<i32, &'a pb::VerificationRecord> {
    let mut sorted: Vec<&pb::VerificationRecord> = records.to_vec();
    sorted.sort_by(|a, b| (ts(a), &a.record_id).cmp(&(ts(b), &b.record_id)));
    sorted.into_iter().map(|r| (r.lane, r)).collect()
}

fn source_of(r: &pb::VerificationRecord) -> String {
    format!(
        "verify:{}",
        if r.record_id.is_empty() {
            "unidentified"
        } else {
            &r.record_id
        }
    )
}

/// Lane-sourced signals of the given (already matched) records. A record's explicit signals count from PASSED
/// and FAILED runs (a run that failed on a finding still observed what it observed); the table-derived
/// lowering counts only from a passing L3 run with no observed change.
pub fn lane_signals(records: &[&pb::VerificationRecord], has_write_path_param: bool) -> Vec<pb::EffectSignal> {
    let mut out = Vec::new();
    for r in records.iter().filter(|r| ran(r)) {
        let ok = passing(r);
        for s in &r.effect_signals {
            let mut s = s.clone();
            if !s.source.starts_with("verify:") {
                s.source = source_of(r);
            }
            if s.isolation == pb::Isolation::NotProven as i32 {
                s.isolation = r.isolation;
            }
            out.push(s);
        }
        if r.lane != pb::Lane::SandboxDiff as i32 {
            continue;
        }
        if !r.observed_files.is_empty() {
            let paths: Vec<String> = r
                .observed_files
                .iter()
                .map(|f| format!("{}:{}", f.root, f.path))
                .collect();
            out.push(pb::EffectSignal {
                field: "writes_filesystem".into(),
                value: pb::Tri::True as i32,
                confidence: pb::Confidence::Observed as i32,
                source: source_of(r),
                rationale: format!("observed file changes: {}", paths.join(", ")),
                isolation: r.isolation,
                ..Default::default()
            });
        }
        if !ok {
            continue;
        }
        let iso = pb::Isolation::try_from(r.isolation).unwrap_or(pb::Isolation::NotProven);
        for f in effects::TARGET_FIELDS {
            if (*f == "mutates_target" || *f == "writes_filesystem") && !r.observed_files.is_empty() {
                continue;
            }
            let Some(min) = effects::min_isolation_to_lower(f, has_write_path_param) else {
                continue;
            };
            if effects::isolation_rank(iso) < effects::isolation_rank(min) {
                continue;
            }
            out.push(pb::EffectSignal {
                field: (*f).into(),
                value: pb::Tri::False as i32,
                confidence: pb::Confidence::Observed as i32,
                source: source_of(r),
                rationale: format!(
                    "L3 passed at {} with no observed change; lowering table (ADR 0011)",
                    iso.as_str_name()
                ),
                isolation: r.isolation,
                ..Default::default()
            });
        }
    }
    crate::characterize::sort_signals(&mut out);
    out
}

fn withheld(reason: pb::WithheldReason, hint: impl Into<String>, evidence: Vec<String>) -> pb::Withheld {
    pb::Withheld {
        reason: reason as i32,
        hint: hint.into(),
        evidence_ids: evidence,
    }
}

fn lane_label(l: pb::Lane) -> &'static str {
    match l {
        pb::Lane::Static => "L0 static",
        pb::Lane::Parser => "L1 parser",
        pb::Lane::DryRun => "L2 dry-run",
        pb::Lane::SandboxDiff => "L3 sandbox-diff",
        pb::Lane::ReadBack => "L4 read-back",
        pb::Lane::Golden => "L5 golden",
        pb::Lane::Differential => "L6 differential",
        pb::Lane::Property => "L7 property",
        pb::Lane::Agent => "L8 agent",
        pb::Lane::Unspecified => "unspecified lane",
    }
}

/// The hint for undecided target fields (L0 requires one per UNKNOWN field).
pub fn effects_unknown_hint(cap: &pb::Capability, unknown: &[&str]) -> String {
    let leaf = cap.id.rsplit('.').next().unwrap_or_default();
    let likely = crate::characterize::rules::readish(leaf)
        .map(|w| format!("likely READ ('{w}'); "))
        .unwrap_or_default();
    format!(
        "undecided: {}; {likely}lanes or an operator assumption must decide them: aa verify {}",
        unknown.join(", "),
        cap.target_id
    )
}

pub fn compute(
    cap: &pb::Capability,
    records: &[pb::VerificationRecord],
    lock_digest: &str,
    identity_digest: &str,
) -> pb::Admission {
    let prior = cap.admission.clone().unwrap_or_default();
    if prior.state == pb::AdmissionState::Excluded as i32 {
        return prior;
    }
    let target = &cap.target_id;
    let mut out_withheld: Vec<pb::Withheld> = prior.withheld.iter().filter(|w| is_sticky(w)).cloned().collect();

    let matching = matching_records(cap, records, lock_digest, identity_digest);
    let wpp = has_write_path_param(cap);
    let mut signals = cap.signals.clone();
    signals.extend(lane_signals(&matching, wpp));
    let eff = effects::combine(&signals, wpp);
    let lvl = effects::level(&eff);
    let latest = latest_per_lane(&matching);

    if lvl == pb::EffectLevel::Unknown {
        let unknown = effects::unknown_target_fields(&eff);
        out_withheld.push(withheld(
            pb::WithheldReason::EffectsUnknown,
            effects_unknown_hint(cap, &unknown),
            Vec::new(),
        ));
    }
    if cap.branches.iter().any(effects::branch_unresolved)
        && !out_withheld
            .iter()
            .any(|w| w.reason == pb::WithheldReason::MixedAuthority as i32)
    {
        out_withheld.push(withheld(
            pb::WithheldReason::MixedAuthority,
            "an effect branch is unresolved; resolve it in target.aa.yaml",
            Vec::new(),
        ));
    }

    // Findings of the latest record of every lane block admission.
    for r in latest.values() {
        for f in &r.findings {
            if f.reason == pb::WithheldReason::Unspecified as i32 {
                continue;
            }
            let mut f = f.clone();
            if !r.record_id.is_empty() && !f.evidence_ids.contains(&r.record_id) {
                f.evidence_ids.push(r.record_id.clone());
            }
            out_withheld.push(f);
        }
    }

    let required = if lvl == pb::EffectLevel::Unknown {
        required_lanes(pb::EffectLevel::Read, cap)
    } else {
        required_lanes(lvl, cap)
    };
    let mut ids = Vec::new();
    let mut all_passed = true;
    for lane in &required {
        match latest.get(&(*lane as i32)) {
            Some(r) if passing(r) => ids.push(r.record_id.clone()),
            Some(r) => {
                all_passed = false;
                if r.findings
                    .iter()
                    .all(|f| f.reason == pb::WithheldReason::Unspecified as i32)
                {
                    let status = pb::LaneStatus::try_from(r.status).unwrap_or(pb::LaneStatus::NotRun);
                    let why = if r.skip_reason.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", r.skip_reason)
                    };
                    out_withheld.push(withheld(
                        pb::WithheldReason::NotVerified,
                        format!(
                            "{} {}{why}: aa verify {target}",
                            lane_label(*lane),
                            status.as_str_name()
                        ),
                        vec![r.record_id.clone()],
                    ));
                }
            }
            None => {
                all_passed = false;
                let hint = if *lane == pb::Lane::ReadBack {
                    format!(
                        "{} needs the L4 read-back lane (slice 2) for the current capability digest: aa verify {target}",
                        lvl.as_str_name()
                    )
                } else {
                    format!(
                        "no PASSED {} record for the current capability digest: aa verify {target}",
                        lane_label(*lane)
                    )
                };
                out_withheld.push(withheld(pb::WithheldReason::NotVerified, hint, Vec::new()));
            }
        }
    }
    if effects::level_rank(lvl) >= effects::level_rank(pb::EffectLevel::Write)
        && lvl != pb::EffectLevel::Unknown
        && cap.oracles.is_empty()
    {
        out_withheld.push(withheld(
            pb::WithheldReason::NoOracle,
            format!(
                "{} needs a read-back or negative oracle: add capabilities.{}.oracle to target.aa.yaml, then: aa verify {target}",
                lvl.as_str_name(),
                cap.id
            ),
            Vec::new(),
        ));
    }

    out_withheld.sort_by(|a, b| (a.reason, &a.hint, &a.evidence_ids).cmp(&(b.reason, &b.hint, &b.evidence_ids)));
    out_withheld.dedup();
    ids.sort();
    ids.dedup();
    let servable = lvl != pb::EffectLevel::Unknown && all_passed && out_withheld.is_empty();
    let isolation = if servable {
        latest
            .get(&(pb::Lane::SandboxDiff as i32))
            .map_or(pb::Isolation::NotProven as i32, |r| r.isolation)
    } else {
        pb::Isolation::NotProven as i32
    };
    pb::Admission {
        state: if servable {
            pb::AdmissionState::Servable as i32
        } else {
            pb::AdmissionState::Withheld as i32
        },
        withheld: if servable { Vec::new() } else { out_withheld },
        verification_record_ids: if servable { ids } else { Vec::new() },
        required_runtime_isolation: isolation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_cap() -> pb::Capability {
        let mut signals = crate::characterize::rules::binding_intrinsic_signals(
            &pb::Target::default(),
            Some(&pb::Binding {
                kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
                    kill_scope: pb::KillScope::OwnedGroup as i32,
                    ..Default::default()
                })),
            }),
        );
        signals.retain(|s| s.field != effects::MUTATES_SESSION);
        pb::Capability {
            id: "t.list".into(),
            target_id: "t".into(),
            capability_digest: "d1".into(),
            signals,
            binding: Some(pb::Binding {
                kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding::default())),
            }),
            ..Default::default()
        }
    }

    fn rec(lane: pb::Lane, status: pb::LaneStatus, iso: pb::Isolation) -> pb::VerificationRecord {
        pb::VerificationRecord {
            record_id: format!("vr_{}", lane as i32),
            capability_id: "t.list".into(),
            capability_digest: "d1".into(),
            identity_digest: "id1".into(),
            lock_digest: "l1".into(),
            lane: lane as i32,
            status: status as i32,
            isolation: iso as i32,
            ..Default::default()
        }
    }

    #[test]
    fn read_needs_three_passed_lanes_at_confined() {
        let cap = read_cap();
        let a = compute(&cap, &[], "", "id1");
        assert_eq!(a.state, pb::AdmissionState::Withheld as i32);
        assert!(
            a.withheld
                .iter()
                .any(|w| w.reason == pb::WithheldReason::EffectsUnknown as i32)
        );
        let recs = vec![
            rec(pb::Lane::Static, pb::LaneStatus::Passed, pb::Isolation::None),
            rec(pb::Lane::Parser, pb::LaneStatus::Passed, pb::Isolation::Confined),
            rec(pb::Lane::SandboxDiff, pb::LaneStatus::Passed, pb::Isolation::Confined),
        ];
        let a = compute(&cap, &recs, "", "id1");
        assert_eq!(a.state, pb::AdmissionState::Servable as i32, "{a:?}");
        assert_eq!(a.required_runtime_isolation, pb::Isolation::Confined as i32);
        assert_eq!(a.verification_record_ids.len(), 3);
        // Wrong identity or lock digest: nothing counts.
        assert_eq!(
            compute(&cap, &recs, "", "id2").state,
            pb::AdmissionState::Withheld as i32
        );
        assert_eq!(
            compute(&cap, &recs, "l2", "id1").state,
            pb::AdmissionState::Withheld as i32
        );
        // At PROCESS isolation, network/launch cannot be lowered.
        let mut proc = recs.clone();
        proc[2].isolation = pb::Isolation::Process as i32;
        assert_eq!(
            compute(&cap, &proc, "", "id1").state,
            pb::AdmissionState::Withheld as i32
        );
        // A SIDE_EFFECT finding blocks and is reported.
        let mut side = recs.clone();
        side[2].status = pb::LaneStatus::Failed as i32;
        side[2].findings.push(pb::Withheld {
            reason: pb::WithheldReason::SideEffect as i32,
            hint: "cwd:repo/.git/index MODIFIED".into(),
            ..Default::default()
        });
        side[2].observed_files.push(pb::ObservedFileEffect {
            path: "repo/.git/index".into(),
            root: "private:cwd".into(),
            change: pb::FileChange::Modified as i32,
            ..Default::default()
        });
        let a = compute(&cap, &side, "", "id1");
        assert!(
            a.withheld
                .iter()
                .any(|w| w.reason == pb::WithheldReason::SideEffect as i32)
        );
        // Skipped never counts.
        let mut skip = recs;
        skip[1].status = pb::LaneStatus::Skipped as i32;
        assert_eq!(
            compute(&cap, &skip, "", "id1").state,
            pb::AdmissionState::Withheld as i32
        );
    }

    #[test]
    fn write_needs_read_back() {
        let mut cap = read_cap();
        let lanes = required_lanes(pb::EffectLevel::Write, &cap);
        assert!(lanes.contains(&pb::Lane::ReadBack));
        cap.dry_run = Some(pb::DryRun {
            supported: true,
            ..Default::default()
        });
        assert!(required_lanes(pb::EffectLevel::Destructive, &cap).contains(&pb::Lane::DryRun));
        assert!(required_lanes(pb::EffectLevel::Unknown, &cap).is_empty());
    }
}
