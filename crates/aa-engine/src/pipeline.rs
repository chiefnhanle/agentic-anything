//! Discover -> compile -> verify orchestration (DistillService) over the gate's pipeline discipline.
//!
//! Every effectful step (an INTROSPECT harvest, an L1/L3 lane run) is an audited pipeline action
//! (`aa.pipeline.harvest.<id>`, `aa.pipeline.verify.<lane>`) through [`Gate::pipeline`](crate::Gate::pipeline).

use crate::{Engine, now_ts, ports::RawMeta};
use aa_core::{
    AaError, AaResult,
    api::{HarvestCtx, Harvester, RawHarvest},
    manifest::CompileInput,
    pb,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::mpsc;

pub(crate) type HarvestTx = mpsc::Sender<Result<pb::HarvestResponse, tonic::Status>>;
pub(crate) type VerifyTx = mpsc::Sender<Result<pb::VerifyResponse, tonic::Status>>;

fn harvester_name(h: &dyn Harvester) -> String {
    format!("{}@{}", h.id(), h.version())
}

/// `name` is `"<id>"` or `"<id>@<version>"`.
fn matches(h: &dyn Harvester, name: &str) -> bool {
    name == h.id() || name == harvester_name(h)
}

fn failed_precondition(message: impl Into<String>, remediation: impl Into<String>) -> AaError {
    AaError::new(pb::ErrorCode::ConfigurationInvalid, message, remediation)
}

impl Engine {
    pub(crate) fn harvester(&self, name: &str) -> Option<Arc<dyn Harvester>> {
        self.deps.harvesters.iter().find(|h| matches(&***h, name)).cloned()
    }

    /// Canonical `<id>@<version>` names for a request list; empty => every builtin that applies.
    pub(crate) fn harvester_names(&self, target: &pb::Target, requested: &[String]) -> AaResult<Vec<String>> {
        if requested.is_empty() {
            return Ok(self
                .deps
                .harvesters
                .iter()
                .filter(|h| h.applies(target, &serde_json::Value::Null))
                .map(|h| harvester_name(&**h))
                .collect());
        }
        requested
            .iter()
            .map(|n| {
                self.harvester(n)
                    .map(|h| harvester_name(&*h))
                    .ok_or_else(|| AaError::invalid(format!("unknown harvester {n:?}")))
            })
            .collect()
    }

    /// Stores raw artifacts in the CAS and merges them into the harvest index (replacing earlier artifacts of
    /// the same harvesters). Returns the new `SourceRef`s.
    pub(crate) fn store_raws(
        &self,
        target_id: &str,
        action_id: &str,
        raws: &[RawHarvest],
    ) -> AaResult<Vec<pb::SourceRef>> {
        let mut fresh = Vec::new();
        let mut meta_fresh = Vec::new();
        for r in raws {
            let sha = self.store.cas_put(&r.bytes)?;
            fresh.push(pb::SourceRef {
                harvester: r.harvester.clone(),
                uri: r.uri.clone(),
                sha256: sha.clone(),
                authority: r.authority,
                harvested_at: Some(now_ts()),
                action_id: action_id.to_string(),
            });
            meta_fresh.push(RawMeta {
                harvester: r.harvester.clone(),
                name: r.name.clone(),
                media_type: r.media_type.clone(),
                uri: r.uri.clone(),
                authority: r.authority,
                sha256: sha,
            });
        }
        let replaced: Vec<&str> = raws.iter().map(|r| r.harvester.as_str()).collect();
        let mut index: Vec<pb::SourceRef> = self
            .store
            .read_harvest_index(target_id)?
            .into_iter()
            .filter(|s| !replaced.contains(&s.harvester.as_str()))
            .collect();
        index.extend(fresh.iter().cloned());
        self.store.write_harvest_index(target_id, &index)?;
        let mut meta: Vec<RawMeta> = self
            .store
            .read_raw_meta(target_id)?
            .into_iter()
            .filter(|m| !replaced.contains(&m.harvester.as_str()))
            .collect();
        meta.extend(meta_fresh);
        self.store.write_raw_meta(target_id, &meta)?;
        Ok(fresh)
    }

    fn target_or_not_found(&self, target_id: &str) -> AaResult<pb::Target> {
        self.store
            .read_target(target_id)?
            .ok_or_else(|| crate::ports::not_found(format!("unknown target {target_id:?}"), "aa target add <id> ..."))
    }

    /// DistillService.Harvest. Per-harvester failures are streamed and the harvest continues.
    pub(crate) async fn harvest(self: &Arc<Self>, principal: &pb::Principal, req: pb::HarvestRequest, tx: HarvestTx) {
        let send = |ev: pb::harvest_response::Event, action_id: &str| {
            let tx = tx.clone();
            let action_id = action_id.to_string();
            async move {
                let _ = tx
                    .send(Ok(pb::HarvestResponse {
                        action_id,
                        event: Some(ev),
                    }))
                    .await;
            }
        };
        let target = match self.target_or_not_found(&req.target_id) {
            Ok(t) => t,
            Err(e) => return drop(tx.send(Err(crate::to_status(&e))).await),
        };
        let overlay = match self
            .store
            .read_overlay(&target.id)
            .and_then(|o| o.map(|y| self.sem.parse_overlay(&y)).transpose())
        {
            Ok(o) => o,
            Err(e) => return drop(tx.send(Err(crate::to_status(&e))).await),
        };
        // (harvester, options) in overlay order; no overlay harvest list => every builtin that applies.
        let mut plan: Vec<(Arc<dyn Harvester>, serde_json::Value)> = Vec::new();
        let specs = overlay.as_ref().map(|o| o.harvest.clone()).unwrap_or_default();
        if specs.is_empty() {
            for h in &self.deps.harvesters {
                plan.push((Arc::clone(h), serde_json::Value::Null));
            }
        } else {
            for s in specs {
                match self.harvester(&s.harvester) {
                    Some(h) => {
                        let mut options = if s.options.is_null() {
                            serde_json::json!({})
                        } else {
                            s.options.clone()
                        };
                        // The overlay's curated command list and help argv travel to the harvester as options.
                        if let Some(obj) = options.as_object_mut() {
                            if !s.commands.is_empty() {
                                obj.entry("commands").or_insert(serde_json::json!(s.commands));
                            }
                            if !s.help_argv.is_empty() {
                                obj.entry("help_argv").or_insert(serde_json::json!(s.help_argv));
                            }
                        }
                        plan.push((h, options));
                    }
                    None => {
                        let e = AaError::invalid(format!("overlay names unknown harvester {:?}", s.harvester));
                        send(pb::harvest_response::Event::Error(e.to_proto()), "").await;
                    }
                }
            }
        }
        if !req.harvesters.is_empty() {
            plan.retain(|(h, _)| req.harvesters.iter().any(|n| matches(&**h, n)));
        }
        // A `--python-script` target's entry point is its console script's name; click-info needs it and the
        // overlay usually does not repeat it.
        if let Some(ep) = console_script_entry_point(&target) {
            for (h, options) in &mut plan {
                if h.id() != "click-info" {
                    continue;
                }
                if options.is_null() {
                    *options = serde_json::json!({});
                }
                if let Some(obj) = options.as_object_mut() {
                    obj.entry("entry_point").or_insert(serde_json::json!(ep));
                }
            }
        }
        let max = match req.max_authority() {
            pb::DiscoveryAuthority::Unspecified => pb::DiscoveryAuthority::Introspect,
            m => m,
        };
        let scratch = self.store.scratch_dir();
        let pp = self.pipeline_principal(principal);
        let gate = self.gate();
        let mut summary = pb::HarvestSummary::default();
        let mut candidates = 0usize;
        for (h, options) in plan {
            let name = harvester_name(&*h);
            if h.authority() as i32 > max as i32 {
                summary.skipped_harvesters.push(format!(
                    "{name}: authority {} above max {}",
                    h.authority().as_str_name(),
                    max.as_str_name()
                ));
                continue;
            }
            if !h.applies(&target, &options) {
                summary
                    .skipped_harvesters
                    .push(format!("{name}: does not apply to {}", target.id));
                continue;
            }
            send(pb::harvest_response::Event::Progress(format!("harvesting {name}")), "").await;
            let ctx = HarvestCtx {
                target: &target,
                options: &options,
                runner: &*self.deps.runner,
                scratch_parent: &scratch,
                allow_network: req.allow_network,
            };
            let cap_id = format!("aa.pipeline.harvest.{}", h.id());
            match gate
                .pipeline(&pp, &cap_id, &target, h.authority(), h.harvest(ctx))
                .await
            {
                Ok((raws, action_id)) => {
                    match self.store_raws(&target.id, &action_id, &raws) {
                        Ok(sources) => {
                            for s in sources {
                                send(pb::harvest_response::Event::Source(s.clone()), &action_id).await;
                                summary.sources.push(s);
                            }
                        }
                        Err(e) => send(pb::harvest_response::Event::Error(e.to_proto()), &action_id).await,
                    }
                    match h.candidates(&target, &raws) {
                        Ok(c) => candidates += c.len(),
                        Err(e) => send(pb::harvest_response::Event::Error(e.to_proto()), &action_id).await,
                    }
                }
                Err(e) => send(pb::harvest_response::Event::Error(e.to_proto()), "").await,
            }
        }
        summary.candidate_count = u32::try_from(candidates).unwrap_or(u32::MAX);
        send(pb::harvest_response::Event::Summary(summary), "").await;
    }

    /// Rebuilds exact `RawHarvest`s from the harvest index, the raw-meta sidecar and CAS bytes, grouped by
    /// harvester name.
    fn raws_by_harvester(
        &self,
        target_id: &str,
        sources: &[pb::SourceRef],
    ) -> AaResult<BTreeMap<String, Vec<RawHarvest>>> {
        let meta = self.store.read_raw_meta(target_id)?;
        let mut out: BTreeMap<String, Vec<RawHarvest>> = BTreeMap::new();
        for s in sources {
            let bytes = self.store.cas_get(&s.sha256)?;
            let m = meta
                .iter()
                .find(|m| m.sha256 == s.sha256 && m.harvester == s.harvester && m.uri == s.uri);
            out.entry(s.harvester.clone()).or_default().push(RawHarvest {
                harvester: s.harvester.clone(),
                name: m.map_or_else(|| s.uri.clone(), |m| m.name.clone()),
                media_type: m.map_or_else(|| "application/octet-stream".to_string(), |m| m.media_type.clone()),
                uri: s.uri.clone(),
                authority: s.authority,
                bytes,
            });
        }
        Ok(out)
    }

    /// DistillService.Compile: CAS -> candidates -> `aa_core::manifest::compile` -> write_lock.
    pub(crate) fn compile_target(
        &self,
        principal: &pb::Principal,
        target_id: &str,
        check: bool,
    ) -> AaResult<pb::CompileResponse> {
        let target = self.target_or_not_found(target_id)?;
        let overlay_text = self.store.read_overlay(target_id)?.ok_or_else(|| {
            failed_precondition(
                format!("{target_id} has no overlay"),
                format!("aa target add {target_id} ..."),
            )
        })?;
        let overlay = self.sem.parse_overlay(&overlay_text)?;
        let sources = self.store.read_harvest_index(target_id)?;
        let mut candidates = Vec::new();
        for (name, raws) in self.raws_by_harvester(target_id, &sources)? {
            match self.harvester(&name) {
                Some(h) => candidates.extend(h.candidates(&target, &raws)?),
                None => tracing::warn!(harvester = %name, "no harvester for recorded artifacts; skipped"),
            }
        }
        let verification = self.store.read_verifications(target_id)?;
        let previous = self.store.read_lock(target_id)?;
        let out = self.sem.compile(
            CompileInput {
                target,
                overlay: &overlay,
                overlay_sha256: aa_core::digest::sha256_hex(overlay_text.as_bytes()),
                sources,
                candidates,
                verification: &verification,
                audit: &*self.audit,
            },
            previous.as_ref(),
        )?;
        let lock = &out.lock;
        let drifted = previous.as_ref().map(|p| p.lock_digest.as_str()) != Some(lock.lock_digest.as_str());
        if !check {
            self.store.write_lock(lock)?;
            self.audit_admission_changes(principal, previous.as_ref(), lock);
        }
        let caps = &lock.capabilities;
        let withheld_for = |r: pb::WithheldReason| {
            caps.iter()
                .filter(|c| {
                    c.admission
                        .as_ref()
                        .is_some_and(|a| a.withheld.iter().any(|w| w.reason() == r))
                })
                .count()
        };
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(pb::CompileResponse {
            lock_digest: lock.lock_digest.clone(),
            added: out.added.clone(),
            removed: out.removed.clone(),
            changed: out.changed.clone(),
            effects_unknown_count: count(withheld_for(pb::WithheldReason::EffectsUnknown)),
            unresolved_branch_count: count(withheld_for(pb::WithheldReason::MixedAuthority)),
            writes_without_oracle_count: count(
                caps.iter()
                    .filter(|c| c.level >= pb::EffectLevel::Write as i32 && c.oracles.is_empty())
                    .count(),
            ),
            drifted: check && drifted,
        })
    }

    fn audit_admission_changes(
        &self,
        principal: &pb::Principal,
        previous: Option<&pb::CapabilityLock>,
        lock: &pb::CapabilityLock,
    ) {
        let state_of = |l: Option<&pb::CapabilityLock>, id: &str| {
            l.and_then(|l| l.capabilities.iter().find(|c| c.id == id))
                .map(|c| c.admission.as_ref().map_or(pb::AdmissionState::Withheld, |a| a.state()))
        };
        for c in &lock.capabilities {
            let now = c.admission.as_ref().map_or(pb::AdmissionState::Withheld, |a| a.state());
            let before = state_of(previous, &c.id);
            if before.is_some_and(|b| b != now) || (before.is_none() && now != pb::AdmissionState::Withheld) {
                let details = serde_json::json!({
                    "from": before.map(|b| b.as_str_name()), "to": now.as_str_name(),
                    "lock_digest": lock.lock_digest, "capability_digest": c.capability_digest,
                });
                if let Err(e) = self.admin(principal, pb::AdminEventKind::AdmissionChanged, &c.id, details) {
                    tracing::error!(error = %e, "could not audit ADMISSION_CHANGED");
                }
            }
        }
    }

    /// DistillService.Verify: L0, then L1 and L3 per capability; writes records and recompiles admission.
    pub(crate) async fn verify(self: &Arc<Self>, principal: &pb::Principal, req: pb::VerifyRequest, tx: VerifyTx) {
        let progress = |s: String| {
            let tx = tx.clone();
            async move {
                let _ = tx
                    .send(Ok(pb::VerifyResponse {
                        event: Some(pb::verify_response::Event::Progress(s)),
                    }))
                    .await;
            }
        };
        let fail = |e: AaError| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(Err(crate::to_status(&e))).await;
            }
        };
        let lock = match self.store.read_lock(&req.target_id) {
            Ok(Some(l)) => l,
            Ok(None) => {
                return fail(failed_precondition(
                    format!("{} has no lock", req.target_id),
                    format!("aa discover {}", req.target_id),
                ))
                .await;
            }
            Err(e) => return fail(e).await,
        };
        let target = lock.target.clone().unwrap_or_default();
        let available = self.deps.runner.max_isolation();
        if (available as i32) < req.min_isolation {
            return fail(
                AaError::new(
                    pb::ErrorCode::IsolationNotProven,
                    format!(
                        "lanes need {} but this host provides {}",
                        req.min_isolation().as_str_name(),
                        available.as_str_name()
                    ),
                    "aa status",
                )
                .with_denial(pb::DenialReason::EnforcementUnavailable),
            )
            .await;
        }
        for id in &req.capability_ids {
            if !lock.capabilities.iter().any(|c| &c.id == id) {
                return fail(crate::ports::not_found(
                    format!("unknown capability {id:?}"),
                    format!("aa list {}", target.id),
                ))
                .await;
            }
        }
        let caps: Vec<pb::Capability> = lock
            .capabilities
            .iter()
            .filter(|c| req.capability_ids.is_empty() || req.capability_ids.contains(&c.id))
            .cloned()
            .collect();
        let binding = self
            .deps
            .bindings
            .iter()
            .find(|b| b.kind() == aa_core::api::BindingKind::CliArgv)
            .cloned();
        let scratch = self.store.scratch_dir();
        let version = self.cfg.engine_version.clone();
        let pp = self.pipeline_principal(principal);
        let gate = self.gate();
        let (mut failed, mut skipped) = (0u32, 0u32);
        for cap in &caps {
            let adm = cap.admission.clone().unwrap_or_default();
            if adm.state() == pb::AdmissionState::Excluded
                || adm
                    .withheld
                    .iter()
                    .any(|w| w.reason() == pb::WithheldReason::Interactive)
            {
                progress(format!("{}: not verified (excluded or interactive)", cap.id)).await;
                continue;
            }
            let level = pb::EffectLevel::try_from(cap.level).unwrap_or_default();
            // Default: the observation lanes of this release (L0, L1, L3) plus whatever the level requires.
            // A capability with undecided effects has level UNKNOWN (no required lanes), and L3 is exactly
            // what decides them, so the observation lanes always run.
            let mut lanes: Vec<pb::Lane> = if req.lanes.is_empty() {
                let mut l = vec![pb::Lane::Static, pb::Lane::Parser, pb::Lane::SandboxDiff];
                l.extend(self.sem.required_lanes(level, cap));
                l
            } else {
                req.lanes().collect()
            };
            if !lanes.contains(&pb::Lane::Static) {
                lanes.insert(0, pb::Lane::Static);
            }
            lanes.sort_by_key(|l| *l as i32);
            lanes.dedup();
            let is_cli = matches!(
                cap.binding.as_ref().and_then(|b| b.kind.as_ref()),
                Some(pb::binding::Kind::Cli(_))
            );
            for lane in lanes {
                let cap_id = format!("aa.pipeline.verify.{}", lane_slug(lane));
                let result = match lane {
                    pb::Lane::Static => {
                        let fut = async { Ok(self.sem.run_l0(&lock, cap, &version)) };
                        gate.pipeline(&pp, &cap_id, &target, pb::DiscoveryAuthority::Static, fut)
                            .await
                    }
                    pb::Lane::Parser | pb::Lane::SandboxDiff => {
                        let Some(b) = binding.as_ref().filter(|_| is_cli) else {
                            progress(format!("{}: {} needs a cli_argv binding", cap.id, lane.as_str_name())).await;
                            continue;
                        };
                        let ctx = aa_exec::lanes::LaneCtx {
                            lock: &lock,
                            binding: &**b,
                            runner: &*self.deps.runner,
                            scratch_parent: &scratch,
                            engine_version: &version,
                        };
                        let fut = async {
                            Ok(if lane == pb::Lane::Parser {
                                self.sem.run_l1(&ctx, cap).await
                            } else {
                                self.sem.run_l3(&ctx, cap).await
                            })
                        };
                        gate.pipeline(&pp, &cap_id, &target, pb::DiscoveryAuthority::Probe, fut)
                            .await
                    }
                    other => {
                        progress(format!(
                            "{}: {} is not available in this release",
                            cap.id,
                            other.as_str_name()
                        ))
                        .await;
                        continue;
                    }
                };
                let mut rec = match result {
                    Ok((rec, action_id)) => {
                        let mut rec: pb::VerificationRecord = rec;
                        rec.action_ids.push(action_id);
                        rec
                    }
                    Err(e) => {
                        progress(format!(
                            "{}: {} could not run: {}",
                            cap.id,
                            lane.as_str_name(),
                            e.message
                        ))
                        .await;
                        if e.code == pb::ErrorCode::EngineHalted {
                            return fail(e).await;
                        }
                        failed += 1;
                        continue;
                    }
                };
                if rec.status() == pb::LaneStatus::Skipped
                    && req.require_tools.iter().any(|t| rec.skip_reason.contains(t.as_str()))
                {
                    rec.status = pb::LaneStatus::Failed as i32;
                    rec.skip_reason = format!("required tool missing: {}", rec.skip_reason);
                }
                if rec.record_id.is_empty() {
                    rec.record_id = match aa_core::digest::self_digest(&rec, |r| r.record_id.clear()) {
                        Ok(d) => format!("vr_{d}"),
                        Err(e) => return fail(e).await,
                    };
                }
                match rec.status() {
                    pb::LaneStatus::Failed | pb::LaneStatus::Error => failed += 1,
                    pb::LaneStatus::Skipped | pb::LaneStatus::NotRun => skipped += 1,
                    pb::LaneStatus::Passed => {}
                }
                if let Err(e) = self.store.write_verification(&rec) {
                    return fail(e).await;
                }
                let stop = lane == pb::Lane::Static && rec.status() != pb::LaneStatus::Passed;
                let _ = tx
                    .send(Ok(pb::VerifyResponse {
                        event: Some(pb::verify_response::Event::Record(rec)),
                    }))
                    .await;
                if stop {
                    break;
                }
            }
        }
        // Recompile so admission counts the new records (records bind to the capability digest).
        let compiled = match self.compile_target(principal, &target.id, false) {
            Ok(c) => c,
            Err(e) => return fail(e).await,
        };
        let new_lock = match self.store.read_lock(&target.id) {
            Ok(Some(l)) => l,
            Ok(None) => return fail(AaError::internal("lock vanished after compile")).await,
            Err(e) => return fail(e).await,
        };
        let in_scope = |c: &&pb::Capability| caps.iter().any(|x| x.id == c.id);
        let servable = new_lock
            .capabilities
            .iter()
            .filter(in_scope)
            .filter(|c| {
                c.admission
                    .as_ref()
                    .is_some_and(|a| a.state() == pb::AdmissionState::Servable)
            })
            .count();
        let total = new_lock.capabilities.iter().filter(in_scope).count();
        let any_servable = new_lock.capabilities.iter().any(|c| {
            c.admission
                .as_ref()
                .is_some_and(|a| a.state() == pb::AdmissionState::Servable)
        });
        let status = if any_servable {
            pb::TargetStatus::LiveVerified
        } else {
            pb::TargetStatus::Scaffolded
        };
        if let Ok(Some(mut t)) = self.store.read_target(&target.id)
            && t.status != status as i32
        {
            t.status = status as i32;
            if let Err(e) = self.store.write_target(&t) {
                tracing::warn!(error = %e, "could not update target status");
            }
        }
        let summary = pb::VerifySummary {
            servable: u32::try_from(servable).unwrap_or(u32::MAX),
            withheld: u32::try_from(total - servable).unwrap_or(u32::MAX),
            failed,
            skipped,
            target_status: status as i32,
            lock_digest: compiled.lock_digest,
        };
        let _ = tx
            .send(Ok(pb::VerifyResponse {
                event: Some(pb::verify_response::Event::Summary(summary)),
            }))
            .await;
    }
}

/// The entry point of a target registered with `python_console_script`: the script's file name (pinned as the
/// `executable` role next to an `interpreter`).
fn console_script_entry_point(target: &pb::Target) -> Option<String> {
    if target.kind != pb::TargetKind::CliTool as i32 {
        return None;
    }
    let files = &target.identity.as_ref()?.files;
    files.iter().find(|f| f.role == "interpreter")?;
    let exe = files.iter().find(|f| f.role == "executable")?;
    std::path::Path::new(&exe.realpath)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

fn lane_slug(l: pb::Lane) -> &'static str {
    match l {
        pb::Lane::Static => "l0",
        pb::Lane::Parser => "l1",
        pb::Lane::DryRun => "l2",
        pb::Lane::SandboxDiff => "l3",
        pb::Lane::ReadBack => "l4",
        pb::Lane::Golden => "l5",
        pb::Lane::Differential => "l6",
        pb::Lane::Property => "l7",
        pb::Lane::Agent => "l8",
        pb::Lane::Unspecified => "unspecified",
    }
}
