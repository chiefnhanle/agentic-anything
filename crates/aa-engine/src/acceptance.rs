//! Acceptance tests (build-plan §3.5), in-process over the duplex transport or a temp-dir UDS, against
//! `FakeRunner`, `MemoryAudit` and the other fakes in `crate::testing`.

use crate::testing::{FakePolicy, Harness, HarnessBuilder, capability, decode_status, read_effects, write_effects};
use aa_core::{
    AaResult,
    api::{Candidate, HarvestCtx, Harvester, RawHarvest, RunSpec, testing::ok_output},
    pb,
};
use aa_proto::v1::{
    distill_service_client::DistillServiceClient, engine_service_client::EngineServiceClient,
    runtime_service_client::RuntimeServiceClient, target_service_client::TargetServiceClient,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tonic::transport::Channel;

type Events = Vec<pb::InvokeResponse>;

async fn invoke(ch: &Channel, cap: &str, input: &str) -> Result<Events, tonic::Status> {
    let req = pb::InvokeRequest {
        capability_id: cap.into(),
        input_json: input.into(),
        idempotency_key: format!("k-{cap}-{input}"),
        ..Default::default()
    };
    let mut s = RuntimeServiceClient::new(ch.clone()).invoke(req).await?.into_inner();
    let mut out = Vec::new();
    while let Some(m) = s.message().await? {
        out.push(m);
    }
    Ok(out)
}

/// The in-band error of the terminal event, if any.
fn terminal_error(evs: &Events) -> Option<pb::Error> {
    match evs.last()?.event.as_ref()? {
        pb::invoke_response::Event::Error(e) => Some(e.clone()),
        _ => None,
    }
}

fn terminal_result(evs: &Events) -> Option<pb::InvokeResult> {
    match evs.last()?.event.as_ref()? {
        pb::invoke_response::Event::Result(r) => Some(r.clone()),
        _ => None,
    }
}

/// Exactly one new ActionRecord since `before`, labelled with the stream's action id.
fn one_new_record(h: &Harness, before: usize, evs: &Events) -> pb::ActionRecord {
    let actions = h.actions();
    assert_eq!(actions.len(), before + 1, "exactly one ActionRecord per attempt");
    let rec = actions.last().unwrap().clone();
    assert!(
        evs.iter().all(|e| e.action_id == rec.action_id),
        "every event carries the action id"
    );
    rec
}

fn tool(h: &Harness, caps: Vec<pb::Capability>) -> std::path::PathBuf {
    let exe = h.fake_exe("tool");
    h.install("tool", &exe, caps);
    exe
}

/// A fake executable in its own temp dir (kept for the test's lifetime), so the runner can be scripted
/// with its path before the harness is built.
fn exe_in_tmp(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join(name);
    std::fs::write(&exe, format!("#!/bin/sh\n# {name}\n")).unwrap();
    let exe = std::fs::canonicalize(exe).unwrap();
    (dir, exe)
}

fn exe_str(p: &std::path::Path) -> &str {
    p.to_str().unwrap()
}

// ---------------------------------------------------------------------------------------------------------
// 1. Role split
// ---------------------------------------------------------------------------------------------------------

/// A raw unary call with an empty body to any path (the role layer answers before the body is decoded).
async fn raw_call(ch: &Channel, path: &'static str) -> tonic::Status {
    let mut g = tonic::client::Grpc::new(ch.clone());
    g.ready().await.unwrap();
    let codec = tonic_prost::ProstCodec::<pbjson_types::Empty, pbjson_types::Empty>::default();
    let path = http::uri::PathAndQuery::from_static(path);
    match g.unary(tonic::Request::new(pbjson_types::Empty {}), path, codec).await {
        Ok(_) => tonic::Status::ok(""),
        Err(s) => s,
    }
}

#[tokio::test]
async fn role_split_operator_only_rpcs_denied_to_agent() {
    let h = Harness::new();
    let agent = h.channel(pb::Role::Agent).await;
    let operator_only: Vec<_> = aa_proto::RPC_POLICY
        .iter()
        .filter(|r| r.roles == [pb::Role::Operator as i32])
        .collect();
    let names: Vec<&str> = operator_only
        .iter()
        .map(|r| r.path.rsplit('/').next().unwrap())
        .collect();
    for expected in [
        "Unhalt",
        "Shutdown",
        "AssumeEffects",
        "DecideConfirmation",
        "IssueGrant",
    ] {
        assert!(names.contains(&expected), "{expected} is operator-only in RPC_POLICY");
    }
    for row in &operator_only {
        let st = raw_call(&agent, row.path).await;
        assert_eq!(
            st.code(),
            tonic::Code::PermissionDenied,
            "{} on the agent role",
            row.path
        );
        assert_eq!(decode_status(&st).code(), pb::ErrorCode::AccessDenied);
    }
    // ENGINE_INTERNAL-only (AdapterService) is unreachable from either listener.
    let operator = h.channel(pb::Role::Operator).await;
    for row in aa_proto::RPC_POLICY
        .iter()
        .filter(|r| r.roles == [pb::Role::EngineInternal as i32])
    {
        assert_eq!(raw_call(&agent, row.path).await.code(), tonic::Code::PermissionDenied);
        assert_eq!(
            raw_call(&operator, row.path).await.code(),
            tonic::Code::PermissionDenied
        );
    }
    // Operator-only RPCs pass the role layer on operator.sock (slice 2 ones are then UNIMPLEMENTED).
    for path in [
        "/agentic_anything.v1.PolicyService/DecideConfirmation",
        "/agentic_anything.v1.PolicyService/IssueGrant",
        "/agentic_anything.v1.TargetService/AssumeEffects",
    ] {
        assert_eq!(
            raw_call(&operator, path).await.code(),
            tonic::Code::Unimplemented,
            "{path}"
        );
    }
}

#[tokio::test]
async fn halt_on_both_roles_and_stale_epoch_unhalt_refused() {
    let h = Harness::new();
    let mut agent = EngineServiceClient::new(h.channel(pb::Role::Agent).await);
    let mut operator = EngineServiceClient::new(h.channel(pb::Role::Operator).await);

    let r = agent.halt(pb::HaltRequest::default()).await.unwrap().into_inner();
    let first = r.halt.unwrap();
    assert!(first.halted);
    assert_eq!(first.source(), pb::HaltSource::Rpc);
    let r = operator
        .halt(pb::HaltRequest {
            reason: "again".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let second = r.halt.unwrap();
    assert_eq!(second.halt_epoch, first.halt_epoch + 1);

    // Agent cannot unhalt at all.
    let st = agent
        .unhalt(pb::UnhaltRequest {
            halt_epoch: second.halt_epoch,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::PermissionDenied);
    // Operator with a stale epoch: refused, still halted.
    let st = operator
        .unhalt(pb::UnhaltRequest {
            halt_epoch: first.halt_epoch,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::InvalidArgument);
    assert!(h.engine.halt.is_halted());
    // Current epoch: released and persisted.
    let r = operator
        .unhalt(pb::UnhaltRequest {
            halt_epoch: second.halt_epoch,
            reason: "ok".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!r.halt.unwrap().halted);
    assert!(!crate::ports::HaltPersist::load(&*h.halt).halted);
    // Every halt/unhalt attempt is audited.
    let kinds: Vec<i32> = h
        .entries()
        .iter()
        .filter_map(|e| match &e.body {
            Some(pb::audit_entry::Body::Admin(a)) => Some(a.kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == pb::AdminEventKind::Halt as i32).count(),
        2
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == pb::AdminEventKind::Unhalt as i32)
            .count(),
        2
    );
}

#[tokio::test]
async fn status_reports_role_halt_and_assurance() {
    let h = Harness::new();
    let s = EngineServiceClient::new(h.channel(pb::Role::Agent).await)
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(s.caller_role(), pb::Role::Agent);
    assert_eq!(s.protocol_version, "agentic_anything.v1");
    assert!(!s.halt.as_ref().unwrap().halted);
    assert_eq!(s.assurance(), pb::AssuranceTier::SamePrincipal);
    assert!(!s.confinement_available, "FakeRunner reports ISOLATION_PROCESS");
    assert!(!s.audit_head_sha256.is_empty(), "ENGINE_START was written");
    let s = EngineServiceClient::new(h.channel(pb::Role::Operator).await)
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(s.caller_role(), pb::Role::Operator);
}

// ---------------------------------------------------------------------------------------------------------
// 3. Gate order and records
// ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn gate_order_and_exactly_one_record() {
    let (_exe_dir, exe) = exe_in_tmp("tool");
    let b = HarnessBuilder {
        policy: FakePolicy {
            allow_levels: vec![pb::EffectLevel::Read],
            deny_caps: vec!["tool.denied".into()],
        },
        runner: aa_core::api::testing::FakeRunner::new()
            .on(&[exe_str(&exe), "status"], ok_output("# branch.head main\n")),
        ..Default::default()
    };
    let h = b.build();
    h.install(
        "tool",
        &exe,
        vec![
            capability("tool", "status", read_effects(), true),
            capability("tool", "withheld", read_effects(), false),
            capability("tool", "denied", read_effects(), true),
        ],
    );
    let ch = h.channel(pb::Role::Agent).await;

    // Unknown capability: NOT_FOUND, no record.
    let before = h.actions().len();
    let st = invoke(&ch, "tool.nope", "{}").await.unwrap_err();
    assert_eq!(st.code(), tonic::Code::NotFound);
    assert_eq!(decode_status(&st).code(), pb::ErrorCode::UnresolvedSelector);
    assert_eq!(h.actions().len(), before, "no record before resolution");

    // Schema-invalid input: NOT_DISPATCHED + INVALID_REQUEST.
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.status", r#"{"bogus": 1}"#).await.unwrap();
    let rec = one_new_record(&h, before, &evs);
    assert_eq!(rec.outcome(), pb::Outcome::NotDispatched);
    assert_eq!(rec.error.as_ref().unwrap().code(), pb::ErrorCode::InvalidRequest);
    assert_eq!(terminal_error(&evs).unwrap().code(), pb::ErrorCode::InvalidRequest);
    assert!(!rec.args_commitment.is_empty(), "args are committed even when rejected");
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.status", "not json").await.unwrap();
    assert_eq!(
        one_new_record(&h, before, &evs).error.unwrap().code(),
        pb::ErrorCode::InvalidRequest
    );

    // Withheld: CAPABILITY_WITHHELD with the reasons.
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.withheld", "{}").await.unwrap();
    let rec = one_new_record(&h, before, &evs);
    let err = rec.error.clone().unwrap();
    assert_eq!(err.code(), pb::ErrorCode::CapabilityWithheld);
    assert!(
        err.details_json.contains("WITHHELD_REASON_NOT_VERIFIED"),
        "{}",
        err.details_json
    );
    assert_eq!(rec.outcome(), pb::Outcome::NotDispatched);

    // Policy deny: the record names the deciding rule.
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.denied", "{}").await.unwrap();
    let rec = one_new_record(&h, before, &evs);
    assert_eq!(rec.error.as_ref().unwrap().code(), pb::ErrorCode::PolicyDenied);
    assert_eq!(rec.error.as_ref().unwrap().denial_reason(), pb::DenialReason::DenyRule);
    let decision = rec.decision.unwrap();
    assert_eq!(decision.deciding_rule_id, "deny-tool.denied");
    assert!(!decision.policy_digest.is_empty());
    assert!(!rec.error.unwrap().remediation.is_empty());

    // READ allowed: dispatched once, COMPLETED, output inline with TARGET origin.
    assert!(h.runner.calls().is_empty(), "nothing dispatched so far");
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.status", "{}").await.unwrap();
    let rec = one_new_record(&h, before, &evs);
    assert_eq!(rec.outcome(), pb::Outcome::Completed, "{:?}", rec.error);
    let result = terminal_result(&evs).unwrap();
    assert_eq!(result.output_origin(), pb::ContentOrigin::Target);
    assert!(result.output_json.contains("# branch.head"), "{}", result.output_json);
    assert!(!rec.output_sha256.is_empty());
    assert_eq!(rec.decision.unwrap().deciding_rule_id, "allow-EFFECT_LEVEL_READ");
    assert_eq!(h.runner.calls().len(), 1);
    assert_eq!(h.runner.calls()[0].argv[1], "status");
    // READ needs no intent.
    assert!(
        !h.entries()
            .iter()
            .any(|e| matches!(e.body, Some(pb::audit_entry::Body::Intent(_))))
    );
    assert!(h.audit.mem.verify_chain());
}

// ---------------------------------------------------------------------------------------------------------
// 4. Intent discipline
// ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn write_with_failed_intent_is_audit_unavailable_and_never_dispatched() {
    let mut b = HarnessBuilder::default();
    b.policy.allow_levels = vec![pb::EffectLevel::Read, pb::EffectLevel::Write];
    let h = b.build();
    tool(&h, vec![capability("tool", "add", write_effects(), true)]);
    let ch = h.channel(pb::Role::Agent).await;
    h.audit.mem.fail_next_intent();
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.add", r#"{"text":"x"}"#).await.unwrap();
    let rec = one_new_record(&h, before, &evs);
    assert_eq!(rec.error.as_ref().unwrap().code(), pb::ErrorCode::AuditUnavailable);
    assert_eq!(rec.outcome(), pb::Outcome::NotDispatched);
    assert!(
        h.runner.calls().is_empty(),
        "nothing dispatched without a durable intent"
    );
}

#[tokio::test]
async fn write_writes_intent_before_dispatch() {
    let (_exe_dir, exe) = exe_in_tmp("tool");
    let mut b = HarnessBuilder::default();
    b.policy.allow_levels = vec![pb::EffectLevel::Write];
    b.runner = aa_core::api::testing::FakeRunner::new().on(&[exe_str(&exe)], ok_output("done"));
    let h = b.build();
    h.install("tool", &exe, vec![capability("tool", "add", write_effects(), true)]);
    let ch = h.channel(pb::Role::Agent).await;
    let evs = invoke(&ch, "tool.add", r#"{"text":"x"}"#).await.unwrap();
    assert!(terminal_result(&evs).is_some(), "{:?}", terminal_error(&evs));
    let entries = h.entries();
    let intent = entries
        .iter()
        .position(|e| matches!(&e.body, Some(pb::audit_entry::Body::Intent(i)) if i.capability_id == "tool.add"))
        .expect("intent written");
    let action = entries
        .iter()
        .position(|e| matches!(&e.body, Some(pb::audit_entry::Body::Action(a)) if a.capability_id == "tool.add"))
        .unwrap();
    assert!(intent < action);
    // A non-idempotent WRITE without an idempotency key is rejected.
    let req = pb::InvokeRequest {
        capability_id: "tool.add".into(),
        input_json: "{}".into(),
        ..Default::default()
    };
    let mut s = RuntimeServiceClient::new(ch.clone())
        .invoke(req)
        .await
        .unwrap()
        .into_inner();
    let mut evs = Vec::new();
    while let Some(m) = s.message().await.unwrap() {
        evs.push(m);
    }
    assert_eq!(terminal_error(&evs).unwrap().code(), pb::ErrorCode::InvalidRequest);
    // Reusing a key of a dispatched action is refused with CHECK_STATUS_BEFORE_RETRY.
    let evs = invoke(&ch, "tool.add", r#"{"text":"x"}"#).await.unwrap();
    let err = terminal_error(&evs).unwrap();
    assert_eq!(err.recovery.unwrap().kind(), pb::RecoveryKind::CheckStatusBeforeRetry);
    assert_eq!(h.runner.calls().len(), 1);
}

/// An INTROSPECT harvester that checks the intent is already durable when it first runs the target.
struct IntrospectProbe {
    audit: Arc<crate::testing::TestAudit>,
    saw_intent: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Harvester for IntrospectProbe {
    fn id(&self) -> &'static str {
        "probe"
    }
    fn version(&self) -> u32 {
        1
    }
    fn authority(&self) -> pb::DiscoveryAuthority {
        pb::DiscoveryAuthority::Introspect
    }
    fn applies(&self, _t: &pb::Target, _o: &serde_json::Value) -> bool {
        true
    }
    async fn harvest(&self, ctx: HarvestCtx<'_>) -> AaResult<Vec<RawHarvest>> {
        let intent_first = self.audit.mem.entries().iter().any(|e| {
            matches!(&e.body, Some(pb::audit_entry::Body::Intent(i)) if i.capability_id == "aa.pipeline.harvest.probe")
        });
        self.saw_intent.store(intent_first, Ordering::SeqCst);
        let exe = ctx.target.identity.as_ref().unwrap().files[0].realpath.clone();
        let out = ctx
            .runner
            .run(RunSpec {
                argv: vec![exe, "--help".into()],
                cwd: ctx.scratch_parent.to_path_buf(),
                env: BTreeMap::new(),
                stdin: None,
                deadline: Duration::from_secs(5),
                max_stdout: 1024,
                max_stderr_excerpt: 256,
                confinement: None,
                run_id: "probe".into(),
            })
            .await?;
        Ok(vec![RawHarvest {
            harvester: "probe@1".into(),
            name: "help".into(),
            media_type: "text/plain".into(),
            uri: "argv:tool --help".into(),
            authority: pb::DiscoveryAuthority::Introspect as i32,
            bytes: out.stdout,
        }])
    }
    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> AaResult<Vec<Candidate>> {
        Ok(raw
            .iter()
            .flat_map(|r| {
                String::from_utf8_lossy(&r.bytes)
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .map(|verb| Candidate {
                native_path: vec![verb.clone()],
                capability: pb::Capability {
                    id: format!("{}.{verb}", target.id),
                    binding: Some(pb::Binding {
                        kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
                            executable_role: "executable".into(),
                            command_path: vec![verb.clone()],
                            ..Default::default()
                        })),
                    }),
                    title: Some(pb::TargetText {
                        text: format!("Run {verb}"),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                native_signals: vec![],
                interactive: false,
            })
            .collect())
    }
}

fn probe_harness(exe_out: &str) -> (Harness, Arc<AtomicBool>, std::path::PathBuf, tempfile::TempDir) {
    let (_exe_dir, exe) = exe_in_tmp("tool");
    let saw = Arc::new(AtomicBool::new(false));
    let audit = Arc::new(crate::testing::TestAudit {
        mem: aa_core::api::testing::MemoryAudit::new(),
        tampered: AtomicBool::new(false),
    });
    let b = HarnessBuilder {
        runner: aa_core::api::testing::FakeRunner::new()
            .on(&[exe_str(&exe), "--help"], ok_output(exe_out))
            .on(&[exe_str(&exe)], ok_output("ok\n")),
        harvesters: vec![Arc::new(IntrospectProbe {
            audit: Arc::clone(&audit),
            saw_intent: Arc::clone(&saw),
        })],
        audit: Some(audit),
        ..Default::default()
    };
    (b.build(), saw, exe, _exe_dir)
}

async fn harvest(ch: &Channel, target: &str) -> Result<Vec<pb::HarvestResponse>, tonic::Status> {
    let mut s = DistillServiceClient::new(ch.clone())
        .harvest(pb::HarvestRequest {
            target_id: target.into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    let mut out = Vec::new();
    while let Some(m) = s.message().await? {
        out.push(m);
    }
    Ok(out)
}

#[tokio::test]
async fn introspect_harvest_writes_intent_before_the_runner() {
    let (h, saw, exe, _d) = probe_harness("status\nlog\n");
    h.install("tool", &exe, vec![]);
    let ch = h.channel(pb::Role::Agent).await;
    let evs = harvest(&ch, "tool").await.unwrap();
    assert!(
        saw.load(Ordering::SeqCst),
        "intent was durable before the harvester ran target code"
    );
    assert_eq!(h.runner.calls().len(), 1);
    let summary = evs
        .iter()
        .find_map(|e| match &e.event {
            Some(pb::harvest_response::Event::Summary(s)) => Some(s.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(summary.candidate_count, 2);
    assert_eq!(summary.sources.len(), 1);
    let src = &summary.sources[0];
    assert!(!src.action_id.is_empty());
    // The pipeline action is recorded as COMPLETED, and the intent precedes it.
    let rec = h
        .actions()
        .into_iter()
        .find(|a| a.capability_id == "aa.pipeline.harvest.probe")
        .unwrap();
    assert_eq!(rec.outcome(), pb::Outcome::Completed);
    assert_eq!(rec.action_id, src.action_id);
    assert_eq!(rec.principal.unwrap().role(), pb::Role::EngineInternal);
}

#[tokio::test]
async fn introspect_refused_without_authorization_statement() {
    let (h, saw, exe, _d) = probe_harness("status\n");
    h.install("tool", &exe, vec![]);
    let mut t = crate::ports::Store::read_target(&*h.store, "tool").unwrap().unwrap();
    t.authorization_statement.clear();
    crate::ports::Store::write_target(&*h.store, &t).unwrap();
    let ch = h.channel(pb::Role::Agent).await;
    let evs = harvest(&ch, "tool").await.unwrap();
    let err = evs
        .iter()
        .find_map(|e| match &e.event {
            Some(pb::harvest_response::Event::Error(e)) => Some(e.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(err.code(), pb::ErrorCode::PolicyDenied);
    assert!(h.runner.calls().is_empty());
    assert!(!saw.load(Ordering::SeqCst));
    let rec = h
        .actions()
        .into_iter()
        .find(|a| a.capability_id == "aa.pipeline.harvest.probe")
        .unwrap();
    assert_eq!(rec.outcome(), pb::Outcome::NotDispatched);
    assert!(
        !h.entries()
            .iter()
            .any(|e| matches!(e.body, Some(pb::audit_entry::Body::Intent(_))))
    );
}

#[tokio::test]
async fn discover_verify_invoke_flow() {
    let (h, _saw, exe, _d) = probe_harness("status\n");
    h.install("tool", &exe, vec![]);
    let ch = h.channel(pb::Role::Agent).await;
    harvest(&ch, "tool").await.unwrap();
    let mut distill = DistillServiceClient::new(ch.clone());
    let c = distill
        .compile(pb::CompileRequest {
            target_id: "tool".into(),
            check: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(c.added, vec!["tool.status".to_string()]);
    let evs = invoke(&ch, "tool.status", "{}").await.unwrap();
    assert_eq!(terminal_error(&evs).unwrap().code(), pb::ErrorCode::CapabilityWithheld);

    let mut s = distill
        .verify(pb::VerifyRequest {
            target_id: "tool".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let mut records = Vec::new();
    let mut summary = None;
    while let Some(m) = s.message().await.unwrap() {
        match m.event {
            Some(pb::verify_response::Event::Record(r)) => records.push(r),
            Some(pb::verify_response::Event::Summary(x)) => summary = Some(x),
            _ => {}
        }
    }
    assert_eq!(records.len(), 3, "L0, L1, L3");
    assert!(
        records
            .iter()
            .all(|r| r.record_id.starts_with("vr_") && r.action_ids.len() == 1)
    );
    let summary = summary.unwrap();
    assert_eq!(summary.servable, 1);
    assert_eq!(summary.target_status(), pb::TargetStatus::LiveVerified);
    // L1/L3 are PROBE pipeline actions with intents.
    let intents = h
        .entries()
        .iter()
        .filter(|e| matches!(&e.body, Some(pb::audit_entry::Body::Intent(i)) if i.capability_id.starts_with("aa.pipeline.verify.")))
        .count();
    assert_eq!(intents, 2);
    let evs = invoke(&ch, "tool.status", "{}").await.unwrap();
    assert!(terminal_result(&evs).is_some(), "{:?}", terminal_error(&evs));
    let admission_events = h
        .entries()
        .iter()
        .filter(|e| matches!(&e.body, Some(pb::audit_entry::Body::Admin(a)) if a.kind == pb::AdminEventKind::AdmissionChanged as i32))
        .count();
    assert_eq!(admission_events, 1);
}

// ---------------------------------------------------------------------------------------------------------
// 5. Boot halted
// ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn tampered_audit_boots_halted() {
    let b = HarnessBuilder {
        tampered: true,
        ..Default::default()
    };
    let h = b.build();
    tool(&h, vec![capability("tool", "status", read_effects(), true)]);
    let ch = h.channel(pb::Role::Agent).await;
    let s = EngineServiceClient::new(ch.clone())
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    let halt = s.halt.unwrap();
    assert!(halt.halted);
    assert_eq!(halt.source(), pb::HaltSource::AuditVerifyFailed);
    let st = invoke(&ch, "tool.status", "{}").await.unwrap_err();
    assert_eq!(st.code(), tonic::Code::Unavailable);
    assert_eq!(decode_status(&st).code(), pb::ErrorCode::EngineHalted);
    assert!(h.entries().iter().any(|e| matches!(&e.body,
        Some(pb::audit_entry::Body::Admin(a)) if a.kind == pb::AdminEventKind::ChainVerifyFailed as i32)));
    assert!(h.actions().is_empty());
}

// ---------------------------------------------------------------------------------------------------------
// 6. Identity drift
// ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn identity_drift_is_recorded_and_not_dispatched() {
    let h = Harness::new();
    let exe = tool(&h, vec![capability("tool", "status", read_effects(), true)]);
    std::fs::write(&exe, "#!/bin/sh\necho replaced\n").unwrap();
    let ch = h.channel(pb::Role::Agent).await;
    let before = h.actions().len();
    let evs = invoke(&ch, "tool.status", "{}").await.unwrap();
    let rec = one_new_record(&h, before, &evs);
    let err = rec.error.clone().unwrap();
    assert_eq!(err.code(), pb::ErrorCode::TargetIdentityChanged);
    assert_eq!(err.recovery.unwrap().kind(), pb::RecoveryKind::Redistill);
    assert_eq!(rec.outcome(), pb::Outcome::NotDispatched);
    assert!(h.runner.calls().is_empty());
}

// ---------------------------------------------------------------------------------------------------------
// 7. PathScope
// ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn path_scope_read_roots_and_symlink_escape() {
    let roots = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(roots.path()).unwrap();
    let allowed = base.join("allowed");
    let outside = base.join("outside");
    let write_root = base.join("w");
    for d in [&allowed, &outside, &write_root] {
        std::fs::create_dir(d).unwrap();
    }
    std::os::unix::fs::symlink(&outside, write_root.join("link")).unwrap();
    let exe = base.join("tool");
    std::fs::write(&exe, "#!/bin/sh\n").unwrap();
    let b = HarnessBuilder {
        policy_toml: Some(format!(
            "format = \"aa-policy/1\"\n[paths]\nread_roots = [\"{}\"]\nwrite_roots = [\"{}\"]\n",
            allowed.display(),
            write_root.display()
        )),
        runner: aa_core::api::testing::FakeRunner::new().on(&[exe_str(&exe)], ok_output("ok")),
        ..Default::default()
    };
    let h = b.build();
    let mut export = capability("tool", "export", read_effects(), true);
    export.scopes.push(pb::ScopeParam {
        pointer: "/text".into(),
        kind: pb::ScopeKind::WritePath as i32,
    });
    h.install(
        "tool",
        &exe,
        vec![capability("tool", "status", read_effects(), true), export],
    );
    let ch = h.channel(pb::Role::Agent).await;

    let input = |p: &std::path::Path| serde_json::json!({ "repo": p }).to_string();
    let evs = invoke(&ch, "tool.status", &input(&outside)).await.unwrap();
    let err = terminal_error(&evs).unwrap();
    assert_eq!(err.code(), pb::ErrorCode::PolicyDenied);
    assert_eq!(err.denial_reason(), pb::DenialReason::OutsidePathScope);
    let evs = invoke(&ch, "tool.status", &input(std::path::Path::new("/etc")))
        .await
        .unwrap();
    assert_eq!(
        terminal_error(&evs).unwrap().denial_reason(),
        pb::DenialReason::OutsidePathScope
    );
    // A path that does not resolve is an input error, still recorded and not dispatched.
    let evs = invoke(&ch, "tool.status", &input(&allowed.join("missing")))
        .await
        .unwrap();
    assert_eq!(terminal_error(&evs).unwrap().code(), pb::ErrorCode::InvalidRequest);
    assert!(h.runner.calls().is_empty());

    let evs = invoke(&ch, "tool.status", &input(&allowed)).await.unwrap();
    assert!(terminal_result(&evs).is_some(), "{:?}", terminal_error(&evs));
    assert_eq!(h.runner.calls().len(), 1);

    // A write path whose parent is a symlink out of the write root is denied.
    let escape = serde_json::json!({ "text": write_root.join("link").join("f.txt") }).to_string();
    let evs = invoke(&ch, "tool.export", &escape).await.unwrap();
    assert_eq!(
        terminal_error(&evs).unwrap().denial_reason(),
        pb::DenialReason::OutsidePathScope
    );
    let inside = serde_json::json!({ "text": write_root.join("f.txt") }).to_string();
    let evs = invoke(&ch, "tool.export", &inside).await.unwrap();
    assert!(terminal_result(&evs).is_some(), "{:?}", terminal_error(&evs));
    assert_eq!(h.runner.calls().len(), 2);
}

// ---------------------------------------------------------------------------------------------------------
// 8. Kill switch
// ---------------------------------------------------------------------------------------------------------

async fn wait_for_calls(h: &Harness, n: usize) {
    for _ in 0..500 {
        if h.runner.calls().len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("runner never reached {n} calls");
}

fn hanging_harness() -> (Harness, tempfile::TempDir) {
    let (_exe_dir, exe) = exe_in_tmp("tool");
    let b = HarnessBuilder {
        runner: aa_core::api::testing::FakeRunner::new().hang(&[exe_str(&exe), "slow"]),
        ..Default::default()
    };
    let h = b.build();
    h.install("tool", &exe, vec![capability("tool", "slow", read_effects(), true)]);
    (h, _exe_dir)
}

#[tokio::test]
async fn halt_drops_in_flight_dispatch_and_refuses_new_invokes() {
    let (h, _exe) = hanging_harness();
    let ch = h.channel(pb::Role::Agent).await;
    let call = tokio::spawn({
        let ch = ch.clone();
        async move { invoke(&ch, "tool.slow", "{}").await }
    });
    wait_for_calls(&h, 1).await;
    let r = EngineServiceClient::new(ch.clone())
        .halt(pb::HaltRequest {
            reason: "kill switch test".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(r.cancelled_actions, 1);
    let evs = tokio::time::timeout(Duration::from_secs(5), call)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(terminal_error(&evs).unwrap().code(), pb::ErrorCode::EngineHalted);
    assert_eq!(h.runner.dropped(), 1, "the runner observed its future being dropped");
    let rec = h
        .actions()
        .into_iter()
        .find(|a| a.capability_id == "tool.slow")
        .unwrap();
    assert!(matches!(rec.outcome(), pb::Outcome::Cancelled | pb::Outcome::Unknown));
    let st = invoke(&ch, "tool.slow", "{}").await.unwrap_err();
    assert_eq!(st.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn cancel_rpc_and_client_drop_stop_the_dispatch() {
    let (h, _exe) = hanging_harness();
    let ch = h.channel(pb::Role::Agent).await;
    let mut rt = RuntimeServiceClient::new(ch.clone());
    let mut stream = rt
        .invoke(pb::InvokeRequest {
            capability_id: "tool.slow".into(),
            input_json: "{}".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let first = stream.message().await.unwrap().unwrap();
    wait_for_calls(&h, 1).await;
    let r = rt
        .cancel(pb::CancelRequest {
            action_id: first.action_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(r.outcome(), pb::Outcome::Cancelled);
    let mut last = None;
    while let Some(m) = stream.message().await.unwrap() {
        last = Some(m);
    }
    assert!(
        matches!(last.unwrap().event, Some(pb::invoke_response::Event::Error(e)) if e.code() == pb::ErrorCode::Cancelled)
    );
    let status = rt
        .get_action_status(pb::GetActionStatusRequest {
            key: Some(pb::get_action_status_request::Key::ActionId(first.action_id)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(status.record.unwrap().outcome(), pb::Outcome::Cancelled);

    // Client goes away mid-flight: the gate drops the dispatch and records CANCELLED.
    let mut stream = rt
        .invoke(pb::InvokeRequest {
            capability_id: "tool.slow".into(),
            input_json: "{}".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    stream.message().await.unwrap().unwrap();
    wait_for_calls(&h, 2).await;
    drop(stream);
    for _ in 0..500 {
        if h.runner.dropped() == 2 && h.actions().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(h.runner.dropped(), 2);
    assert_eq!(h.actions()[1].outcome(), pb::Outcome::Cancelled);
}

// ---------------------------------------------------------------------------------------------------------
// Targets, sessions, UDS
// ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn create_target_pins_identity_and_audits_authorization() {
    let h = Harness::new();
    let mut t = TargetServiceClient::new(h.channel(pb::Role::Agent).await);
    let locator = Some(pb::TargetLocator {
        locator: Some(pb::target_locator::Locator::Executable("sh".into())),
    });
    let st = t
        .create_target(pb::CreateTargetRequest {
            target_id: "sh".into(),
            locator: locator.clone(),
            authorization_statement: "  ".into(),
            harvesters: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::InvalidArgument);
    // CreateTarget is audited: the refusal is an AdminEvent too.
    assert!(h.entries().iter().any(|e| matches!(&e.body,
        Some(pb::audit_entry::Body::Admin(a)) if a.details_json.contains("ERROR_CODE_INVALID_REQUEST"))));
    let st = t
        .create_target(pb::CreateTargetRequest {
            target_id: "Bad_Id".into(),
            locator: locator.clone(),
            authorization_statement: "ok".into(),
            harvesters: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::InvalidArgument);
    let r = t
        .create_target(pb::CreateTargetRequest {
            target_id: "sh".into(),
            locator,
            authorization_statement: "owner workstation".into(),
            harvesters: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    let target = r.target.unwrap();
    let identity = target.identity.unwrap();
    assert_eq!(identity.files[0].role, "executable");
    assert!(!identity.identity_digest.is_empty());
    assert_eq!(target.authorization_statement, "owner workstation");
    assert!(r.overlay_path.ends_with("targets/sh/target.aa.yaml"));
    assert!(crate::ports::Store::read_overlay(&*h.store, "sh").unwrap().is_some());
    let registered = h
        .entries()
        .into_iter()
        .find_map(|e| match e.body {
            Some(pb::audit_entry::Body::Admin(a)) if a.kind == pb::AdminEventKind::TargetRegistered as i32 => Some(a),
            _ => None,
        })
        .unwrap();
    assert_eq!(registered.subject, "sh");
    assert!(registered.details_json.contains("owner workstation"));
    let list = t
        .list_targets(pb::ListTargetsRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(list.targets.len(), 1);
    let st = t
        .get_target(pb::GetTargetRequest {
            target_id: "nope".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn import_cli_anything_stores_install_cmd_and_never_runs_it() {
    let h = Harness::new();
    let python = h.fake_exe("python3");
    let mut t = TargetServiceClient::new(h.channel(pb::Role::Agent).await);
    let r = t
        .import_cli_anything(pb::ImportCliAnythingRequest {
            repo_path: h.dir.path().display().to_string(),
            harnesses: vec!["notes".into(), "absent".into()],
            python: python.display().to_string(),
            authorization_statement: "owner".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(r.available, vec!["notes".to_string()]);
    let ok = &r.harnesses[0];
    assert!(ok.error.is_none(), "{:?}", ok.error);
    assert_eq!(ok.target_id, "cli-anything.notes");
    assert_eq!(ok.source_tree_sha, "tree123");
    assert_eq!(
        ok.install_cmd_artifact_sha256,
        aa_core::digest::sha256_hex(b"pip install x; rm -rf /")
    );
    assert!(r.harnesses[1].error.is_some());
    assert!(h.runner.calls().is_empty(), "import is STATIC: nothing runs");
    let index = crate::ports::Store::read_harvest_index(&*h.store, "cli-anything.notes").unwrap();
    assert_eq!(index.len(), 1);
}

#[tokio::test]
async fn sessions_are_private_and_tainted_by_output() {
    let (_exe_dir, exe) = exe_in_tmp("tool");
    let b = HarnessBuilder {
        runner: aa_core::api::testing::FakeRunner::new().on(&[exe_str(&exe)], ok_output("x")),
        ..Default::default()
    };
    let h = b.build();
    h.install("tool", &exe, vec![capability("tool", "status", read_effects(), true)]);
    let ch = h.channel(pb::Role::Agent).await;
    let mut rt = RuntimeServiceClient::new(ch.clone());
    let s = rt
        .open_session(pb::OpenSessionRequest {
            target_ids: vec!["tool".into()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(std::path::Path::new(&s.workspace_path).is_dir());
    let st = rt
        .open_session(pb::OpenSessionRequest {
            read_roots: vec!["/etc".into()],
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::FailedPrecondition);
    let mut stream = rt
        .invoke(pb::InvokeRequest {
            capability_id: "tool.status".into(),
            input_json: "{}".into(),
            session_id: s.session_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    while stream.message().await.unwrap().is_some() {}
    assert!(
        h.engine
            .sessions
            .get(&s.session_id, crate::config::euid())
            .unwrap()
            .tainted()
    );
    assert_eq!(h.actions()[0].session_id, s.session_id);
    let st = invoke_with_session(&ch, "nope").await.unwrap_err();
    assert_eq!(st.code(), tonic::Code::NotFound);
    rt.close_session(pb::CloseSessionRequest {
        session_id: s.session_id.clone(),
        keep_workspace: false,
    })
    .await
    .unwrap();
    assert!(!std::path::Path::new(&s.workspace_path).exists());
}

async fn invoke_with_session(ch: &Channel, session: &str) -> Result<(), tonic::Status> {
    RuntimeServiceClient::new(ch.clone())
        .invoke(pb::InvokeRequest {
            capability_id: "tool.status".into(),
            session_id: session.into(),
            ..Default::default()
        })
        .await
        .map(|_| ())
}

async fn uds(path: &std::path::Path) -> Channel {
    tonic::transport::Endpoint::from_shared(format!("unix://{}", path.display()))
        .unwrap()
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn uds_listeners_roles_peer_uid_and_shutdown() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // A short runtime dir: UDS paths are limited to ~104 bytes on macOS.
    let rt_dir = tempfile::Builder::new().prefix("aa").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(rt_dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    // Agent uids exclude the owner: agent.sock refuses us, operator.sock accepts us.
    let b = HarnessBuilder {
        agent_uids: Some(vec![crate::config::euid().wrapping_add(1)]),
        ..Default::default()
    };
    let mut h = b.build();
    Arc::get_mut(&mut h.engine).unwrap().cfg.runtime_dir = rt_dir.path().join("r");
    let engine = Arc::clone(&h.engine);
    let server = tokio::spawn(engine.serve_uds(std::future::pending()));
    let agent_sock = rt_dir.path().join("r/agent.sock");
    let op_sock = rt_dir.path().join("r/operator.sock");
    for _ in 0..400 {
        if op_sock.exists() && agent_sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(std::fs::metadata(&agent_sock).unwrap().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&op_sock).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        std::fs::metadata(rt_dir.path().join("r")).unwrap().mode() & 0o777,
        0o700
    );

    let st = EngineServiceClient::new(uds(&agent_sock).await)
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::PermissionDenied);
    assert_eq!(decode_status(&st).code(), pb::ErrorCode::AccessDenied);

    let mut op = EngineServiceClient::new(uds(&op_sock).await);
    let s = op.get_status(pb::GetStatusRequest {}).await.unwrap().into_inner();
    assert_eq!(s.caller_role(), pb::Role::Operator);
    assert_eq!(s.operator_listen, format!("unix://{}", op_sock.display()));
    assert_eq!(s.assurance(), pb::AssuranceTier::SeparatedAgent);

    // A second engine on the same runtime dir is refused by the flock.
    let second = HarnessBuilder::default().build();
    let mut second_engine = second.engine;
    Arc::get_mut(&mut second_engine).unwrap().cfg.runtime_dir = rt_dir.path().join("r");
    let err = second_engine.serve_uds(std::future::pending()).await.unwrap_err();
    assert_eq!(err.code, pb::ErrorCode::ConfigurationInvalid);

    op.shutdown(pb::ShutdownRequest { drain: true }).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!agent_sock.exists() && !op_sock.exists(), "sockets removed on shutdown");
    assert!(h.entries().iter().any(|e| matches!(&e.body,
        Some(pb::audit_entry::Body::Admin(a)) if a.kind == pb::AdminEventKind::EngineStop as i32)));
}
