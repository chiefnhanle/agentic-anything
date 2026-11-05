//! The engine composed with the real `aa-store` (JSONL audit, halt persistence) and `aa-core` policy,
//! through the public API only: `Engine::build` + `EngineDeps::production()` + the in-process transport.

use aa_engine::{Engine, EngineConfig, EngineDeps};
use aa_proto::v1 as pb;
use aa_proto::v1::{engine_service_client::EngineServiceClient, runtime_service_client::RuntimeServiceClient};
use std::{os::unix::fs::PermissionsExt, path::Path};

fn config(dir: &Path) -> EngineConfig {
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::set_permissions(&ws, std::fs::Permissions::from_mode(0o700)).unwrap();
    EngineConfig {
        workspace: ws,
        runtime_dir: dir.join("rt"),
        policy_path: dir.join("absent-policy.toml"),
        agent_uids: vec![rustix::process::geteuid().as_raw()],
        engine_version: "0.1.0-test".into(),
    }
}

fn segments(ws: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(ws.join("audit"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    v.sort();
    v
}

async fn status(engine: &std::sync::Arc<Engine>) -> pb::GetStatusResponse {
    let ch = engine.connect_in_process(pb::Role::Agent).await.unwrap();
    EngineServiceClient::new(ch)
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn tampered_jsonl_segment_boots_halted_and_refuses_invoke() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    {
        let engine = Engine::build(cfg.clone(), EngineDeps::production()).unwrap();
        let s = status(&engine).await;
        assert!(!s.halt.unwrap().halted, "a fresh workspace boots unhalted");
        assert!(!s.audit_head_sha256.is_empty());
        engine.request_shutdown();
    }
    // Let the in-process server task release its engine handle (and the audit flock).
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let seg = segments(&cfg.workspace).into_iter().next().expect("an audit segment");
    let text = std::fs::read_to_string(&seg).unwrap();
    let first = text.lines().next().unwrap();
    assert!(first.contains("aa-policy") || first.contains("policy"), "{first}");
    // Flip one byte inside the first entry (still valid JSON, wrong digest).
    let tampered = text.replacen("builtin", "builtiN", 1);
    assert_ne!(tampered, text, "the first entry names the builtin policy");
    std::fs::write(&seg, tampered).unwrap();

    let engine = Engine::build(cfg.clone(), EngineDeps::production()).unwrap();
    let s = status(&engine).await;
    let halt = s.halt.unwrap();
    assert!(halt.halted);
    assert_eq!(halt.source(), pb::HaltSource::AuditVerifyFailed);
    let ch = engine.connect_in_process(pb::Role::Agent).await.unwrap();
    let st = RuntimeServiceClient::new(ch)
        .invoke(pb::InvokeRequest {
            capability_id: "git.status".into(),
            input_json: "{}".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(st.code(), tonic::Code::Unavailable);
    let err: pb::Error = prost::Message::decode(st.details()).unwrap();
    assert_eq!(err.code(), pb::ErrorCode::EngineHalted);
    engine.request_shutdown();
}

#[tokio::test]
async fn halt_persists_across_restart_and_unhalt_needs_operator() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let epoch = {
        let engine = Engine::build(cfg.clone(), EngineDeps::production()).unwrap();
        let ch = engine.connect_in_process(pb::Role::Agent).await.unwrap();
        let r = EngineServiceClient::new(ch)
            .halt(pb::HaltRequest {
                reason: "persist me".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        engine.request_shutdown();
        r.halt.unwrap().halt_epoch
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let engine = Engine::build(cfg, EngineDeps::production()).unwrap();
    let halt = status(&engine).await.halt.unwrap();
    assert!(halt.halted, "the engine boots halted if it stopped halted");
    assert_eq!(halt.source(), pb::HaltSource::Persisted);
    assert_eq!(halt.halt_epoch, epoch);
    let op = engine.connect_in_process(pb::Role::Operator).await.unwrap();
    let r = EngineServiceClient::new(op)
        .unhalt(pb::UnhaltRequest {
            halt_epoch: epoch,
            reason: "ok".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!r.halt.unwrap().halted);
    engine.request_shutdown();
}
