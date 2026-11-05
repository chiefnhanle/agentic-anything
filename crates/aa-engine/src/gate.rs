//! The single enforcement point (architecture §5.3 steps 2-13). Slice 1 has no "ask" path: ask =>
//! PendingConfirmation is NOT minted; Error CONFIRMATION_REQUIRED, recovery AWAIT_CONFIRMATION "aa approve (M3)".
//!
//! Step 1 (listener / role / halt) is the [`RoleLayer`](crate::RoleLayer). From step 2 on (the capability
//! resolved), exactly one `ActionRecord` is appended per attempt, whatever the outcome; before that, errors
//! are transport statuses and nothing is recorded.

use crate::{
    Engine, now_ts,
    sessions::{EphemeralGuard, NewSession, Session},
    status::StopCause,
};
use aa_core::{
    AaError, AaResult,
    api::{Binding, ConfinementSpec, InvokeCtx},
    pb,
    policy::ActionDescriptor,
};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

/// Output above this size is returned by CAS digest instead of inline.
const INLINE_OUTPUT_BUDGET: usize = 1 << 20;
const DEFAULT_DEADLINE_MS: u32 = 30_000;
const MAX_DEADLINE_MS: u32 = 600_000;

/// Holds `Arc<Engine>` internals.
pub struct Gate {
    engine: Arc<Engine>,
}

pub enum GateEvent {
    Progress(String),
    Result(aa_core::pb::InvokeResult),
    Error(aa_core::pb::Error),
}

/// Everything the checks established, handed to dispatch.
struct Prepared {
    cap: pb::Capability,
    target: pb::Target,
    input: Value,
    effective: pb::Effects,
    level: pb::EffectLevel,
    binding: Arc<dyn Binding>,
    state_handle: Option<PathBuf>,
    confinement: Option<ConfinementSpec>,
    deadline: Duration,
}

enum Interrupt {
    Halt,
    Cancel,
    ClientGone,
}

impl Gate {
    pub(crate) fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }

    /// Exactly one record written.
    ///
    /// Exception (frozen contract, architecture §4 error table): if the session or the capability does not
    /// resolve, nothing is recorded; the returned record is a placeholder (`OUTCOME_NOT_DISPATCHED`, error
    /// `UNRESOLVED_SELECTOR`) and an `Error` event is sent. `RuntimeService.Invoke` calls
    /// [`Gate::preflight`] first so this surfaces as `NOT_FOUND` on the wire.
    pub async fn invoke(
        &self,
        principal: &aa_core::pb::Principal,
        req: aa_core::pb::InvokeRequest,
        events: tokio::sync::mpsc::Sender<GateEvent>,
    ) -> aa_core::pb::ActionRecord {
        self.invoke_as(aa_core::ids::new_action_id(), principal, req, events)
            .await
    }

    /// The pre-resolution checks of `invoke` (session, capability). Errors here have no ActionRecord.
    pub(crate) fn preflight(&self, principal: &pb::Principal, req: &pb::InvokeRequest) -> AaResult<()> {
        if !req.session_id.is_empty() {
            self.engine.sessions.get(&req.session_id, principal.uid)?;
        }
        self.engine.resolve(&req.capability_id).map(|_| ())
    }

    /// `invoke` with a caller-chosen action id (so a streaming front end can label every event).
    pub(crate) async fn invoke_as(
        &self,
        action_id: String,
        principal: &pb::Principal,
        req: pb::InvokeRequest,
        events: mpsc::Sender<GateEvent>,
    ) -> pb::ActionRecord {
        let e = &self.engine;
        // Subscribe before any check so a halt that lands mid-attempt is never missed.
        let mut halt_rx = e.halt.subscribe();
        let entry_epoch = halt_rx.borrow_and_update().halt_epoch;
        let mut rec = pb::ActionRecord {
            action_id,
            capability_id: req.capability_id.clone(),
            principal: Some(principal.clone()),
            dry_run: req.dry_run,
            idempotency_key: req.idempotency_key.clone(),
            requested_at: Some(now_ts()),
            outcome: pb::Outcome::NotDispatched as i32,
            ..Default::default()
        };
        let session = match self.session_for(principal, &req) {
            Ok(s) => s,
            Err(err) => return unrecorded(rec, err, &events).await,
        };
        let _ephemeral = EphemeralGuard(Arc::clone(&session));
        rec.session_id = session.id.clone();
        let resolved = match e.resolve(&req.capability_id) {
            Ok(r) => r,
            Err(err) => return unrecorded(rec, err, &events).await,
        };
        // ---- From here on: exactly one ActionRecord. ----
        let _ = events.send(GateEvent::Progress("resolved".into())).await;
        let prepared = match self.prepare(&mut rec, &resolved, &session, &req, entry_epoch) {
            Ok(p) => p,
            Err(err) => return self.refuse(rec, err, &events).await,
        };

        // Step 10: intent before dispatch for WRITE and above (durable, or nothing runs).
        if prepared.level as i32 >= pb::EffectLevel::Write as i32 {
            let intent = pb::ActionIntent {
                action_id: rec.action_id.clone(),
                capability_id: rec.capability_id.clone(),
                capability_digest: rec.capability_digest.clone(),
                action_digest: rec.action_digest.clone(),
                args_commitment: rec.args_commitment.clone(),
                level: prepared.level as i32,
                decision: rec.decision.clone(),
                idempotency_key: rec.idempotency_key.clone(),
            };
            if let Err(err) = e.audit.append_intent(principal, e.halt.current().halt_epoch, intent) {
                return self.refuse(rec, audit_unavailable(&err), &events).await;
            }
        }

        // Steps 11-12: dispatch, racing the kill switch, Cancel and the client going away.
        let (_inflight, mut cancel_rx) = e.inflight.register(&rec.action_id);
        if halt_rx.borrow_and_update().halted {
            return self.refuse(rec, halted_error(e), &events).await;
        }
        let ctx = InvokeCtx {
            action_id: rec.action_id.clone(),
            identity: prepared.target.identity.clone().unwrap_or_default(),
            state_handle: prepared.state_handle.clone(),
            scratch_parent: e.store.scratch_dir(),
            deadline: prepared.deadline,
            dry_run: req.dry_run,
            confinement: prepared.confinement.clone(),
        };
        rec.runtime_isolation = if ctx.confinement.is_some() {
            pb::Isolation::Confined as i32
        } else {
            pb::Isolation::Process as i32
        };
        let _ = events.send(GateEvent::Progress("dispatching".into())).await;
        rec.dispatched_at = Some(now_ts());
        let run = prepared.binding.invoke(&prepared.cap, &prepared.input, &ctx);
        let result = tokio::select! {
            biased;
            () = wait_halted(&mut halt_rx) => Err(Interrupt::Halt),
            () = wait_cancelled(&mut cancel_rx) => Err(Interrupt::Cancel),
            () = events.closed() => Err(Interrupt::ClientGone),
            r = run => Ok(r),
        };
        let writeish = prepared.level as i32 >= pb::EffectLevel::Write as i32;
        match result {
            Err(why) => {
                // The binding future was dropped above: its drop guard killed the owned process group.
                let (code, msg) = match why {
                    Interrupt::Halt => (pb::ErrorCode::EngineHalted, "halted while in flight"),
                    Interrupt::Cancel => (pb::ErrorCode::Cancelled, "cancelled while in flight"),
                    Interrupt::ClientGone => (pb::ErrorCode::Cancelled, "client went away while in flight"),
                };
                let err = if writeish {
                    rec.outcome = pb::Outcome::Unknown as i32;
                    AaError::new(
                        pb::ErrorCode::ExecutionStateUnknown,
                        format!("{msg}; the action may or may not have taken effect"),
                        format!("aa audit --action {}", rec.action_id),
                    )
                    .with_recovery(
                        pb::RecoveryKind::CheckStatusBeforeRetry,
                        "RuntimeService.GetActionStatus",
                    )
                } else {
                    rec.outcome = pb::Outcome::Cancelled as i32;
                    AaError::new(code, msg, "aa status")
                };
                rec.process = Some(pb::ProcessReport {
                    cleanup: pb::CleanupStatus::Unverified as i32,
                    ..Default::default()
                });
                rec.error = Some(err.to_proto());
                self.finish(rec, None, &events).await
            }
            Ok(Err(err)) => {
                rec.outcome = if writeish {
                    pb::Outcome::Unknown as i32
                } else {
                    pb::Outcome::Failed as i32
                };
                rec.error = Some(err.to_proto());
                self.finish(rec, None, &events).await
            }
            Ok(Ok(out)) => {
                rec.outcome = out.outcome as i32;
                rec.process = Some(out.process);
                rec.observed_files = out.observed_files;
                rec.error = out.error.map(|e| e.to_proto());
                let mut result = pb::InvokeResult {
                    output_origin: pb::ContentOrigin::Target as i32,
                    outcome: rec.outcome,
                    ..Default::default()
                };
                if let Some(v) = out.output_json {
                    let bytes = serde_json::to_vec(&v).unwrap_or_default();
                    match e.store.cas_put(&bytes) {
                        Ok(sha) => rec.output_sha256 = sha,
                        Err(err) => tracing::warn!(error = %err, "could not store output in the CAS"),
                    }
                    if bytes.len() <= INLINE_OUTPUT_BUDGET || rec.output_sha256.is_empty() {
                        result.output_json = String::from_utf8(bytes).unwrap_or_default();
                    } else {
                        result.output_blob_sha256 = rec.output_sha256.clone();
                    }
                    // Step 13: the session has now seen target content.
                    session.taint();
                }
                self.post_observe(&mut rec, &prepared);
                result.outcome = rec.outcome;
                self.finish(rec, Some(result), &events).await
            }
        }
    }

    fn session_for(&self, principal: &pb::Principal, req: &pb::InvokeRequest) -> AaResult<Arc<Session>> {
        let e = &self.engine;
        if req.session_id.is_empty() {
            e.sessions.open(
                &e.store.sessions_dir(),
                NewSession {
                    target_ids: Vec::new(),
                    ceiling: pb::AuthorityTier::Unknown,
                    read_roots: Vec::new(),
                    owner_uid: principal.uid,
                    ephemeral: true,
                },
            )
        } else {
            e.sessions.get(&req.session_id, principal.uid)
        }
    }

    /// Steps 2-9 and the binding lookup. Fills `rec` as facts are established.
    fn prepare(
        &self,
        rec: &mut pb::ActionRecord,
        resolved: &crate::Resolved,
        session: &Session,
        req: &pb::InvokeRequest,
        entry_epoch: u64,
    ) -> AaResult<Prepared> {
        let e = &self.engine;
        let sem = &*e.sem;
        let cap = resolved.cap();
        let target = resolved.target();
        let lock = &resolved.lock;
        let identity = target.identity.clone().unwrap_or_default();
        let effects = cap.effects.unwrap_or_default();
        let binding_pb = cap.binding.clone().unwrap_or_default();
        rec.capability_digest = cap.capability_digest.clone();
        rec.lock_digest = lock.lock_digest.clone();
        rec.identity_digest = identity.identity_digest.clone();
        rec.declared_effects = Some(effects);
        rec.level = cap.level;
        rec.tier = cap.tier;

        // Step 2: digests the caller expects. Slice 1 has no exposure projection, so the catalog identity of
        // a capability is its lock digest.
        if !req.expected_capability_digest.is_empty() && req.expected_capability_digest != cap.capability_digest {
            return Err(stale("capability digest", &cap.id));
        }
        if !req.expected_catalog_digest.is_empty() && req.expected_catalog_digest != lock.lock_digest {
            return Err(stale("catalog digest", &cap.id));
        }

        let input: AaResult<Value> = if req.input_json.trim().is_empty() {
            Ok(Value::Object(Default::default()))
        } else {
            serde_json::from_str(&req.input_json).map_err(|err| {
                AaError::invalid(format!("input_json is not JSON: {err}")).with_recovery(
                    pb::RecoveryKind::InspectInputContract,
                    "CatalogService.DescribeCapability",
                )
            })
        };
        let schema: Value = if cap.input_schema_json.trim().is_empty() {
            serde_json::json!({"type": "object", "additionalProperties": false})
        } else {
            serde_json::from_str(&cap.input_schema_json)
                .map_err(|err| AaError::internal(format!("lock input schema of {} is not JSON: {err}", cap.id)))?
        };
        if let Ok(v) = &input {
            rec.args_commitment = aa_core::digest::args_commitment(&e.hmac_key, v)?;
            rec.input_redacted_json = serde_json::to_string(&redact(v, &schema)).unwrap_or_default();
        }

        // Step 3: admission. Anything but SERVABLE is withheld; EXCLUDED is final.
        let admission = cap.admission.clone().unwrap_or_default();
        if admission.state() != pb::AdmissionState::Servable {
            let excluded = admission.state() == pb::AdmissionState::Excluded;
            let remediation = if excluded {
                format!("aa describe {}", cap.id)
            } else {
                format!("aa verify {}", target.id)
            };
            let mut err = AaError::new(
                pb::ErrorCode::CapabilityWithheld,
                format!("{} is {}", cap.id, if excluded { "excluded" } else { "withheld" }),
                remediation,
            );
            err.details_json = serde_json::to_string(&admission.withheld).unwrap_or_default();
            return Err(err);
        }

        // Step 4: recompute projections; a mismatch with the lock is corrupt state => halt.
        let level = sem.level(&effects);
        let tier = sem.tier(&effects, &binding_pb);
        if level as i32 != cap.level || tier as i32 != cap.tier {
            let reason = format!(
                "lock projections of {} do not match its effects (corrupt state)",
                cap.id
            );
            e.halt
                .halt(pb::HaltSource::Unspecified, pb::HaltScope::Engine, &cap.id, &reason);
            return Err(AaError::internal(reason).with_recovery(pb::RecoveryKind::Redistill, "aa discover"));
        }
        let input = input?;
        let effective = sem.apply_branches(&effects, &cap.branches, &input);
        let eff_level = sem.level(&effective);
        let eff_tier = sem.tier(&effective, &binding_pb);
        rec.level = eff_level as i32;
        rec.tier = eff_tier as i32;
        if eff_level == pb::EffectLevel::Unknown {
            let mut err = AaError::new(
                pb::ErrorCode::CapabilityWithheld,
                format!("{}: effects are unknown for this input", cap.id),
                format!("aa describe {}", cap.id),
            );
            err.details_json = serde_json::to_string(&[pb::Withheld {
                reason: pb::WithheldReason::EffectsUnknown as i32,
                hint: "a matching effect branch left a field undecided".into(),
                evidence_ids: vec![],
            }])
            .unwrap_or_default();
            return Err(err);
        }
        if !session.target_ids.is_empty() && !session.target_ids.contains(&target.id) {
            return Err(policy_denied(
                format!("session {} is not opened for target {}", session.id, target.id),
                "open a session that includes the target",
                pb::DenialReason::DenyRule,
            ));
        }
        if session.ceiling != pb::AuthorityTier::Unknown && sem.tier_rank(eff_tier) > sem.tier_rank(session.ceiling) {
            return Err(policy_denied(
                format!(
                    "{} needs {} above the session ceiling {}",
                    cap.id,
                    eff_tier.as_str_name(),
                    session.ceiling.as_str_name()
                ),
                "open a session with a higher authority ceiling",
                pb::DenialReason::DenyRule,
            ));
        }

        // Step 5: canonical closed schema, then path scopes (deny wins).
        let validator = jsonschema::validator_for(&schema)
            .map_err(|err| AaError::internal(format!("lock input schema of {} does not compile: {err}", cap.id)))?;
        let problems: Vec<String> = validator
            .iter_errors(&input)
            .take(5)
            .map(|err| format!("at '{}': {err}", err.instance_path()))
            .collect();
        if !input.is_object() || !problems.is_empty() {
            let detail = if input.is_object() {
                problems.join("; ")
            } else {
                "input must be a JSON object".into()
            };
            return Err(
                AaError::invalid(format!("input rejected by the schema of {}: {detail}", cap.id)).with_recovery(
                    pb::RecoveryKind::InspectInputContract,
                    "CatalogService.DescribeCapability",
                ),
            );
        }
        if req.dry_run && !cap.dry_run.as_ref().is_some_and(|d| d.supported) {
            return Err(AaError::invalid(format!("{} has no dry run", cap.id)));
        }
        let cli = match &binding_pb.kind {
            Some(pb::binding::Kind::Cli(c)) => Some(c.clone()),
            _ => None,
        };
        let scope = e.policy.roots.scope_for(&session.workspace, &session.read_roots);
        let mut scoped: Vec<(String, bool)> = cap
            .scopes
            .iter()
            .filter_map(|s| match s.kind() {
                pb::ScopeKind::ReadPath => Some((s.pointer.clone(), false)),
                pb::ScopeKind::WritePath => Some((s.pointer.clone(), true)),
                pb::ScopeKind::StateHandle => Some((s.pointer.clone(), true)),
                _ => None,
            })
            .collect();
        if let Some(c) = &cli {
            if !c.cwd_pointer.is_empty() && !scoped.iter().any(|(p, _)| *p == c.cwd_pointer) {
                scoped.push((c.cwd_pointer.clone(), false));
            }
            if !c.state_handle_engine_supplied
                && !c.state_handle_pointer.is_empty()
                && !scoped.iter().any(|(p, _)| *p == c.state_handle_pointer)
            {
                scoped.push((c.state_handle_pointer.clone(), true));
            }
        }
        let mut read_paths = Vec::new();
        let mut write_paths = Vec::new();
        let mut state_handle = None;
        let registered = !target.authorization_statement.trim().is_empty();
        let assurance = e.assurance();
        for (pointer, write) in &scoped {
            for raw in path_values(&input, pointer) {
                let p = PathBuf::from(&raw);
                let p = if p.is_absolute() { p } else { session.workspace.join(p) };
                // An unresolvable path is the canonicalizer's error (usually INVALID_REQUEST); a symlink
                // escape or a path outside the roots is OUTSIDE_PATH_SCOPE.
                let canonical = sem.canonicalize_for_scope(&p, *write).map_err(|mut err| {
                    err.message = format!("{pointer}: {}", err.message);
                    err
                })?;
                let checked = sem.check_scope(&scope, &canonical, *write).map(|()| canonical);
                match checked {
                    Ok(c) => {
                        if cli
                            .as_ref()
                            .is_some_and(|b| !b.state_handle_engine_supplied && b.state_handle_pointer == *pointer)
                        {
                            state_handle = Some(c.clone());
                        }
                        if *write {
                            write_paths.push(c)
                        } else {
                            read_paths.push(c)
                        }
                    }
                    Err(reason) => {
                        let d = descriptor(
                            target,
                            cap,
                            eff_level,
                            eff_tier,
                            &effective,
                            &read_paths,
                            &write_paths,
                            registered,
                            session,
                            assurance,
                        );
                        return Err(policy_denied(
                            format!("{pointer} is outside the session PathScope ({})", reason.as_str_name()),
                            e.policy.eval.remediation_for(reason, &d),
                            reason,
                        ));
                    }
                }
            }
        }
        if cli
            .as_ref()
            .is_some_and(|c| c.state_handle_engine_supplied && !c.state_handle_flag.is_empty())
        {
            // Engine-supplied state handle (CLI-Anything --project): a project file in the session workspace.
            state_handle = Some(session.workspace.join("project.json"));
        }

        // Step 6: brakes.
        let now = e.halt.current();
        if now.halted || now.halt_epoch != entry_epoch {
            return Err(halted_error(e));
        }

        // Step 7: identity, confinement availability, binding.
        sem.verify_identity(&identity).map_err(|err| {
            if err.code == pb::ErrorCode::TargetIdentityChanged {
                err
            } else {
                AaError::new(
                    pb::ErrorCode::TargetIdentityChanged,
                    err.message,
                    format!("aa discover {}", target.id),
                )
                .with_recovery(pb::RecoveryKind::Redistill, format!("aa discover {}", target.id))
            }
        })?;
        let required = match admission.required_runtime_isolation() {
            pb::Isolation::NotProven | pb::Isolation::None => pb::Isolation::Process,
            other => other,
        };
        let available = e.deps.runner.max_isolation();
        if (available as i32) < (required as i32) {
            return Err(AaError::new(
                pb::ErrorCode::IsolationNotProven,
                format!(
                    "{} must run under {} but this host provides {}",
                    cap.id,
                    required.as_str_name(),
                    available.as_str_name()
                ),
                "aa status (confinement_available must be true)",
            )
            .with_denial(pb::DenialReason::EnforcementUnavailable));
        }
        let kind = aa_core::api::binding_kind(&binding_pb);
        let binding = kind
            .and_then(|k| e.deps.bindings.iter().find(|b| b.kind() == k).cloned())
            .ok_or_else(|| {
                AaError::new(
                    pb::ErrorCode::CapabilityUnavailable,
                    format!("no binding of kind {kind:?} is available for {}", cap.id),
                    "aa status",
                )
            })?;

        // Step 8: policy (hard exclusions first, all matching rules, strictest wins, default deny).
        let d = descriptor(
            target,
            cap,
            eff_level,
            eff_tier,
            &effective,
            &read_paths,
            &write_paths,
            registered,
            session,
            assurance,
        );
        let mut decision = e.policy.eval.evaluate(&d);
        if decision.policy_digest.is_empty() {
            decision.policy_digest = e.policy.digest.clone();
        }
        decision.assurance = assurance as i32;
        rec.decision = Some(decision.clone());
        match decision.verdict() {
            pb::Verdict::Deny => {
                let reason = match decision.denial_reason() {
                    pb::DenialReason::Unspecified => pb::DenialReason::NoMatchingAllow,
                    r => r,
                };
                let rule = if decision.deciding_rule_id.is_empty() {
                    "default deny".to_string()
                } else {
                    format!("rule {}", decision.deciding_rule_id)
                };
                let code = if reason == pb::DenialReason::HardExclusion {
                    pb::ErrorCode::Excluded
                } else {
                    pb::ErrorCode::PolicyDenied
                };
                return Err(AaError::new(
                    code,
                    format!("{} denied by {rule}", cap.id),
                    e.policy.eval.remediation_for(reason, &d),
                )
                .with_denial(reason));
            }
            pb::Verdict::Ask => {
                return Err(AaError::new(
                    pb::ErrorCode::ConfirmationRequired,
                    format!("{} needs a human confirmation (not available in this release)", cap.id),
                    "aa approve (M3)",
                )
                .with_recovery(pb::RecoveryKind::AwaitConfirmation, "aa approve (M3)"));
            }
            pb::Verdict::Allow => {}
        }

        // Step 9 (ask) is M3; bind the action digest now so the record and intent carry it.
        rec.action_digest = aa_core::digest::action_digest(&pb::ActionDigestInput {
            capability_id: cap.id.clone(),
            capability_digest: cap.capability_digest.clone(),
            lock_digest: lock.lock_digest.clone(),
            identity_digest: identity.identity_digest.clone(),
            generation: 0,
            args_commitment: rec.args_commitment.clone(),
            policy_digest: decision.policy_digest.clone(),
            dry_run: req.dry_run,
        })?;

        // Idempotency for non-idempotent WRITE+.
        if eff_level as i32 >= pb::EffectLevel::Write as i32 {
            if req.idempotency_key.is_empty() && effective.idempotent() != pb::Tri::True {
                return Err(AaError::invalid(format!(
                    "{} is not idempotent: idempotency_key is required",
                    cap.id
                )));
            }
            if !req.idempotency_key.is_empty() {
                let prior = e
                    .audit
                    .find_by_idempotency_key(&req.idempotency_key)
                    .map_err(|err| audit_unavailable(&err))?;
                if let Some(p) = prior.filter(|p| p.outcome() != pb::Outcome::NotDispatched) {
                    return Err(
                        AaError::invalid(format!("idempotency key already used by {}", p.action_id)).with_recovery(
                            pb::RecoveryKind::CheckStatusBeforeRetry,
                            "RuntimeService.GetActionStatus",
                        ),
                    );
                }
            }
        }

        let confinement = (required as i32 >= pb::Isolation::Confined as i32).then(|| {
            let mut writable = vec![session.workspace.clone()];
            writable.extend(write_paths.iter().cloned());
            ConfinementSpec {
                deny_network: effective.accesses_network() != pb::Tri::True,
                writable,
                exec_allow: identity.files.iter().map(|f| PathBuf::from(&f.realpath)).collect(),
            }
        });
        let binding_deadline = cli.as_ref().map_or(0, |c| c.deadline_ms);
        let deadline_ms = match (req.deadline_ms, binding_deadline) {
            (0, 0) => DEFAULT_DEADLINE_MS,
            (0, b) => b,
            (r, 0) => r,
            (r, b) => r.min(b),
        }
        .min(MAX_DEADLINE_MS);
        Ok(Prepared {
            cap: cap.clone(),
            target: target.clone(),
            input,
            effective,
            level: eff_level,
            binding,
            state_handle,
            confinement,
            deadline: Duration::from_millis(u64::from(deadline_ms)),
        })
    }

    /// Step 12: observed changes in scoped roots that the capability did not declare fail the action and
    /// auto-withhold the capability (`SIDE_EFFECT`). Changes inside the private root are not effects.
    fn post_observe(&self, rec: &mut pb::ActionRecord, p: &Prepared) {
        let declared_writes =
            p.effective.writes_filesystem() == pb::Tri::True || p.effective.mutates_target() == pb::Tri::True;
        if declared_writes {
            return;
        }
        let undeclared: Vec<String> = rec
            .observed_files
            .iter()
            .filter(|f| !f.root.starts_with("private:"))
            .map(|f| format!("writes_filesystem: {}:{}", f.root, f.path))
            .collect();
        if undeclared.is_empty() {
            return;
        }
        rec.outcome = pb::Outcome::Failed as i32;
        let err = AaError::new(
            pb::ErrorCode::CapabilityWithheld,
            format!(
                "{} changed files it did not declare; it is now withheld (SIDE_EFFECT)",
                p.cap.id
            ),
            format!("aa verify {}", p.target.id),
        );
        rec.error = Some(err.to_proto());
        rec.undeclared_effects = undeclared.clone();
        let principal = rec.principal.clone().unwrap_or_default();
        if let Err(err) = self
            .engine
            .auto_withhold(&principal, &p.target.id, &p.cap.id, &undeclared.join(", "))
        {
            tracing::error!(error = %err, capability = %p.cap.id, "could not auto-withhold");
        }
    }

    /// A refused attempt: `OUTCOME_NOT_DISPATCHED` with the error.
    async fn refuse(
        &self,
        mut rec: pb::ActionRecord,
        err: AaError,
        events: &mpsc::Sender<GateEvent>,
    ) -> pb::ActionRecord {
        rec.outcome = pb::Outcome::NotDispatched as i32;
        rec.error = Some(err.to_proto());
        self.finish(rec, None, events).await
    }

    /// Step 13: append the one ActionRecord and send the terminal event.
    async fn finish(
        &self,
        mut rec: pb::ActionRecord,
        result: Option<pb::InvokeResult>,
        events: &mpsc::Sender<GateEvent>,
    ) -> pb::ActionRecord {
        rec.completed_at = Some(now_ts());
        let principal = rec.principal.clone().unwrap_or_default();
        self.engine.record_action(&principal, &rec);
        let ev = match (&rec.error, result) {
            (Some(err), _) => GateEvent::Error(err.clone()),
            (None, Some(r)) => GateEvent::Result(r),
            (None, None) => GateEvent::Result(pb::InvokeResult {
                outcome: rec.outcome,
                ..Default::default()
            }),
        };
        let _ = events.send(ev).await;
        rec
    }

    /// Pipeline actions (aa.pipeline.harvest.<id>, aa.pipeline.verify.<lane>) through the same audit discipline.
    /// Returns the closure's value and the pipeline action id.
    ///
    /// STATIC actions are recorded; INTROSPECT and above (which run target code) are refused for a target
    /// without an authorization statement and write a durable intent before `f` is first polled. `f` is
    /// dropped (never polled) when refused, and dropped mid-flight when the kill switch engages.
    pub async fn pipeline<T>(
        &self,
        principal: &aa_core::pb::Principal,
        capability_id: &str,
        target: &aa_core::pb::Target,
        authority: aa_core::pb::DiscoveryAuthority,
        f: impl std::future::Future<Output = aa_core::AaResult<T>> + Send,
    ) -> aa_core::AaResult<(T, String)> {
        let e = &self.engine;
        let mut halt_rx = e.halt.subscribe();
        halt_rx.borrow_and_update();
        // UNSPECIFIED is treated as INTROSPECT (fail-safe).
        let authority = match authority {
            pb::DiscoveryAuthority::Unspecified => pb::DiscoveryAuthority::Introspect,
            a => a,
        };
        let effectful = authority as i32 >= pb::DiscoveryAuthority::Introspect as i32;
        let decision = pb::PolicyDecision {
            verdict: pb::Verdict::Allow as i32,
            matched_rule_ids: vec!["builtin:pipeline".into()],
            deciding_rule_id: "builtin:pipeline".into(),
            approval_source: pb::ApprovalSource::PolicyAllow as i32,
            assurance: e.assurance() as i32,
            policy_digest: e.policy.digest.clone(),
            reasons: vec![format!(
                "{} pipeline action on a registered target",
                authority.as_str_name()
            )],
            ..Default::default()
        };
        let mut rec = pb::ActionRecord {
            action_id: aa_core::ids::new_action_id(),
            capability_id: capability_id.to_string(),
            identity_digest: target
                .identity
                .as_ref()
                .map(|i| i.identity_digest.clone())
                .unwrap_or_default(),
            principal: Some(principal.clone()),
            level: if effectful {
                pb::EffectLevel::Unknown
            } else {
                pb::EffectLevel::Read
            } as i32,
            tier: if effectful {
                pb::AuthorityTier::Unknown
            } else {
                pb::AuthorityTier::Observe
            } as i32,
            runtime_isolation: e.deps.runner.max_isolation() as i32,
            requested_at: Some(now_ts()),
            outcome: pb::Outcome::NotDispatched as i32,
            residual_unknowns: vec![format!("discovery authority {}", authority.as_str_name())],
            ..Default::default()
        };
        let refuse = |mut rec: pb::ActionRecord, err: AaError| {
            rec.error = Some(err.to_proto());
            rec.completed_at = Some(now_ts());
            e.record_action(principal, &rec);
            Err(err)
        };
        if e.halt.is_halted() {
            return refuse(rec, halted_error(e));
        }
        if effectful && target.authorization_statement.trim().is_empty() {
            let mut d = decision.clone();
            d.verdict = pb::Verdict::Deny as i32;
            d.denial_reason = pb::DenialReason::DenyRule as i32;
            d.approval_source = pb::ApprovalSource::None as i32;
            rec.decision = Some(d);
            let err = AaError::new(
                pb::ErrorCode::PolicyDenied,
                format!(
                    "{} runs target code ({}) and {} has no authorization statement",
                    capability_id,
                    authority.as_str_name(),
                    target.id
                ),
                format!("aa target add {} --authorization \"...\"", target.id),
            )
            .with_denial(pb::DenialReason::DenyRule);
            return refuse(rec, err);
        }
        rec.decision = Some(decision.clone());
        if effectful {
            let intent = pb::ActionIntent {
                action_id: rec.action_id.clone(),
                capability_id: capability_id.to_string(),
                level: rec.level,
                decision: Some(decision),
                ..Default::default()
            };
            if let Err(err) = e.audit.append_intent(principal, e.halt.current().halt_epoch, intent) {
                return refuse(rec, audit_unavailable(&err));
            }
        }
        let (_inflight, mut cancel_rx) = e.inflight.register(&rec.action_id);
        rec.dispatched_at = Some(now_ts());
        let result = tokio::select! {
            biased;
            () = wait_halted(&mut halt_rx) => Err(Interrupt::Halt),
            () = wait_cancelled(&mut cancel_rx) => Err(Interrupt::Cancel),
            r = f => Ok(r),
        };
        rec.completed_at = Some(now_ts());
        match result {
            Ok(Ok(v)) => {
                rec.outcome = pb::Outcome::Completed as i32;
                e.record_action(principal, &rec);
                Ok((v, rec.action_id))
            }
            Ok(Err(err)) => {
                rec.outcome = pb::Outcome::Failed as i32;
                rec.error = Some(err.to_proto());
                e.record_action(principal, &rec);
                Err(err)
            }
            Err(why) => {
                rec.outcome = if effectful {
                    pb::Outcome::Unknown
                } else {
                    pb::Outcome::Cancelled
                } as i32;
                let err = match why {
                    Interrupt::Halt => halted_error(e),
                    _ => AaError::new(
                        pb::ErrorCode::Cancelled,
                        format!("{capability_id} cancelled"),
                        "aa status",
                    ),
                };
                rec.error = Some(err.to_proto());
                e.record_action(principal, &rec);
                Err(err)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn descriptor<'a>(
    target: &'a pb::Target,
    cap: &'a pb::Capability,
    level: pb::EffectLevel,
    tier: pb::AuthorityTier,
    effects: &'a pb::Effects,
    read_paths: &'a [PathBuf],
    write_paths: &'a [PathBuf],
    target_registered: bool,
    session: &Session,
    assurance: pb::AssuranceTier,
) -> ActionDescriptor<'a> {
    ActionDescriptor {
        target_id: &target.id,
        capability_id: &cap.id,
        level,
        tier,
        effects,
        read_paths,
        write_paths,
        target_registered,
        session_tainted: session.tainted(),
        assurance,
    }
}

/// Resolution failed: no record is written (see [`Gate::invoke`]).
async fn unrecorded(mut rec: pb::ActionRecord, err: AaError, events: &mpsc::Sender<GateEvent>) -> pb::ActionRecord {
    rec.outcome = pb::Outcome::NotDispatched as i32;
    rec.error = Some(err.to_proto());
    let _ = events.send(GateEvent::Error(err.to_proto())).await;
    rec.action_id.clear();
    rec
}

async fn wait_halted(rx: &mut watch::Receiver<pb::HaltState>) {
    loop {
        if rx.borrow_and_update().halted {
            return;
        }
        if rx.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

async fn wait_cancelled(rx: &mut watch::Receiver<Option<StopCause>>) {
    loop {
        if rx.borrow_and_update().is_some() {
            return;
        }
        if rx.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

pub(crate) fn halted_error(e: &Engine) -> AaError {
    let epoch = e.halt.current().halt_epoch;
    AaError::new(
        pb::ErrorCode::EngineHalted,
        format!("the engine is halted (epoch {epoch})"),
        format!("operator: aa unhalt --epoch {epoch}"),
    )
    .with_recovery(pb::RecoveryKind::RestartEngine, "aa unhalt")
}

fn audit_unavailable(err: &AaError) -> AaError {
    AaError::new(
        pb::ErrorCode::AuditUnavailable,
        format!(
            "the intent could not be made durable; nothing was dispatched ({})",
            err.message
        ),
        "aa audit --verify",
    )
}

fn stale(what: &str, cap_id: &str) -> AaError {
    AaError::new(
        pb::ErrorCode::CatalogStale,
        format!("expected {what} of {cap_id} does not match the current lock"),
        format!("aa describe {cap_id}"),
    )
    .with_recovery(pb::RecoveryKind::ReobserveTarget, "CatalogService.DescribeCapability")
}

fn policy_denied(message: String, remediation: impl Into<String>, reason: pb::DenialReason) -> AaError {
    AaError::new(pb::ErrorCode::PolicyDenied, message, remediation).with_denial(reason)
}

/// String values at a JSON pointer (a string, or an array of strings). Missing => none.
fn path_values(input: &Value, pointer: &str) -> Vec<String> {
    match input.pointer(pointer) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

/// Input with `x-aa-secret` top-level properties blanked.
fn redact(input: &Value, schema: &Value) -> Value {
    let mut out = input.clone();
    if let (Some(obj), Some(props)) = (out.as_object_mut(), schema.get("properties").and_then(Value::as_object)) {
        for (k, s) in props {
            if s.get("x-aa-secret").and_then(Value::as_bool) == Some(true) && obj.contains_key(k) {
                obj.insert(k.clone(), Value::String("<redacted>".into()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_and_pointers() {
        let schema = serde_json::json!({"properties": {"token": {"type": "string", "x-aa-secret": true}, "a": {}}});
        let v = redact(&serde_json::json!({"token": "s3cret", "a": 1}), &schema);
        assert_eq!(v, serde_json::json!({"token": "<redacted>", "a": 1}));
        let input = serde_json::json!({"repo": "/r", "paths": ["a", 1, "b"]});
        assert_eq!(path_values(&input, "/repo"), vec!["/r"]);
        assert_eq!(path_values(&input, "/paths"), vec!["a", "b"]);
        assert!(path_values(&input, "/missing").is_empty());
    }
}
