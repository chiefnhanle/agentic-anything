//! `aa-engine` (`aad`): two UDS listeners, the role layer, the gate and the gRPC services.
//!
//! Composition root (architecture §2, ADR 0013). Every effect on a target goes through [`Gate`]; every RPC
//! goes through a [`RoleLayer`] that derives the caller's role from the listener. The engine reaches the
//! other crates only through the seams in [`ports`].

pub mod config;
pub mod gate;
mod listen;
mod pipeline;
pub mod policy_file;
pub mod ports;
pub mod role_layer;
mod services;
mod sessions;
mod status;
#[cfg(test)]
mod testing;

#[cfg(test)]
mod acceptance;

pub use config::{AadArgs, EngineConfig};
pub use gate::{Gate, GateEvent};
pub use role_layer::{RoleLayer, RoleService};

use aa_core::{AaError, AaResult, pb};
use ports::{AuditStore, HaltPersist, Semantics, Store};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub struct EngineDeps {
    pub runner: std::sync::Arc<dyn aa_core::api::ProcessRunner>,
    pub harvesters: Vec<std::sync::Arc<dyn aa_core::api::Harvester>>,
    pub bindings: Vec<std::sync::Arc<dyn aa_core::api::Binding>>,
}

impl EngineDeps {
    /// Supervisor(default_confiner), builtin_harvesters, CliArgvBinding.
    pub fn production() -> Self {
        let runner: Arc<dyn aa_core::api::ProcessRunner> =
            Arc::new(aa_exec::Supervisor::new(aa_exec::SupervisorConfig {
                confiner: aa_exec::default_confiner(),
                term_grace: std::time::Duration::from_millis(250),
            }));
        Self {
            bindings: vec![Arc::new(aa_exec::CliArgvBinding::new(Arc::clone(&runner)))],
            harvesters: aa_harvest::builtin_harvesters(),
            runner,
        }
    }
}

/// The seams an engine is assembled from (production: `aa-store` + [`ports::RealSemantics`]).
pub(crate) struct Parts {
    pub store: Arc<dyn Store>,
    pub audit: Arc<dyn AuditStore>,
    pub boot: aa_store::BootReport,
    pub halt: Arc<dyn HaltPersist>,
    pub sem: Arc<dyn Semantics>,
}

/// Workspace, audit, halt, policy, sessions, identity cache, deps.
pub struct Engine {
    pub(crate) cfg: EngineConfig,
    pub(crate) deps: EngineDeps,
    pub(crate) store: Arc<dyn Store>,
    pub(crate) audit: Arc<dyn AuditStore>,
    pub(crate) sem: Arc<dyn Semantics>,
    pub(crate) halt: status::HaltCtl,
    pub(crate) policy: policy_file::LoadedPolicy,
    pub(crate) sessions: sessions::Sessions,
    pub(crate) inflight: status::Inflight,
    pub(crate) hmac_key: [u8; 32],
    pub(crate) boot: aa_store::BootReport,
    pub(crate) owner_uid: u32,
    /// `true` once shutdown was requested (Shutdown RPC or the `serve_uds` future).
    pub(crate) shutdown: watch::Sender<bool>,
    /// `(agent, operator)` listen URIs once bound.
    pub(crate) listen: Mutex<(String, String)>,
}

/// A capability located in its target's lock.
pub(crate) struct Resolved {
    pub lock: pb::CapabilityLock,
    pub idx: usize,
}

impl Resolved {
    pub fn cap(&self) -> &pb::Capability {
        &self.lock.capabilities[self.idx]
    }
    pub fn target(&self) -> &pb::Target {
        static EMPTY: std::sync::OnceLock<pb::Target> = std::sync::OnceLock::new();
        self.lock
            .target
            .as_ref()
            .unwrap_or_else(|| EMPTY.get_or_init(pb::Target::default))
    }
}

pub(crate) fn now_ts() -> pbjson_types::Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    pbjson_types::Timestamp {
        seconds: i64::try_from(now.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(now.subsec_nanos()).unwrap_or(0),
    }
}

impl Engine {
    /// Opens the workspace + audit (BootReport: chain failure => boots halted with HALT_SOURCE_AUDIT_VERIFY_FAILED),
    /// loads halt state and policy, writes ENGINE_START.
    pub fn build(cfg: EngineConfig, deps: EngineDeps) -> aa_core::AaResult<std::sync::Arc<Self>> {
        let (store, audit, boot, halt) = ports::open_production(&cfg.workspace)?;
        let sem: Arc<dyn Semantics> = Arc::new(ports::RealSemantics::new());
        Self::assemble(
            cfg,
            deps,
            Parts {
                store,
                audit,
                boot,
                halt,
                sem,
            },
        )
    }

    pub(crate) fn assemble(cfg: EngineConfig, deps: EngineDeps, parts: Parts) -> AaResult<Arc<Self>> {
        let root = parts.store.root();
        let extra_deny = [root.join("state"), root.join("audit"), cfg.runtime_dir.clone()];
        let policy = policy_file::load(&*parts.sem, &cfg.policy_path, &extra_deny)?;
        let hmac_key = parts.store.hmac_key()?;
        let mut initial = parts.halt.load();
        if initial.halted && initial.source() != pb::HaltSource::AuditVerifyFailed {
            initial.source = pb::HaltSource::Persisted as i32;
        }
        let engine = Arc::new(Self {
            owner_uid: config::euid(),
            halt: status::HaltCtl::new(initial, parts.halt),
            store: parts.store,
            audit: parts.audit,
            sem: parts.sem,
            policy,
            sessions: sessions::Sessions::default(),
            inflight: status::Inflight::default(),
            hmac_key,
            boot: parts.boot,
            shutdown: watch::channel(false).0,
            listen: Mutex::new((String::new(), String::new())),
            cfg,
            deps,
        });
        let sys = engine.system_principal();
        let boot = &engine.boot;
        let boot_json = serde_json::json!({
            "chain_ok": boot.chain_ok, "entries": boot.entries, "first_broken_seq": boot.first_broken_seq,
            "head_matches": boot.head_matches, "recovered_action_ids": boot.recovered_action_ids,
        });
        let chain_ok = boot.chain_ok && boot.head_matches;
        if !chain_ok {
            let reason = match boot.first_broken_seq {
                Some(seq) => format!("audit chain failed to verify at seq {seq}"),
                None => "audit HEAD does not match the last entry (truncation?)".to_string(),
            };
            engine
                .halt
                .halt(pb::HaltSource::AuditVerifyFailed, pb::HaltScope::Engine, "", &reason);
            if let Err(e) = engine.admin(&sys, pb::AdminEventKind::ChainVerifyFailed, "audit", boot_json.clone()) {
                tracing::error!(error = %e, "could not record CHAIN_VERIFY_FAILED");
            }
        } else if engine.halt.sentinel_present() && !engine.halt.is_halted() {
            engine.halt.halt(
                pb::HaltSource::SentinelFile,
                pb::HaltScope::Engine,
                "",
                "HALT sentinel present at boot",
            );
        }
        let policy_json = serde_json::json!({
            "policy_digest": engine.policy.digest,
            "source": engine.policy.source.as_ref().map_or("builtin".to_string(), |p| p.display().to_string()),
        });
        let start_json = serde_json::json!({
            "engine_version": engine.cfg.engine_version, "boot": boot_json,
            "halted": engine.halt.is_halted(),
        });
        for (kind, subject, details) in [
            (pb::AdminEventKind::PolicyLoaded, "policy", policy_json),
            (pb::AdminEventKind::EngineStart, "aad", start_json),
        ] {
            if let Err(e) = engine.admin(&sys, kind, subject, details) {
                if chain_ok {
                    return Err(e);
                }
                tracing::error!(error = %e, "could not append to a broken audit chain");
            }
        }
        Ok(engine)
    }

    /// Runtime-dir checks, flock(engine.lock), unlink stale sockets, bind both 0600, serve until `shutdown`.
    pub async fn serve_uds(
        self: std::sync::Arc<Self>,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> aa_core::AaResult<()> {
        listen::serve_uds(self, shutdown).await
    }

    /// For tests: in-process duplex transport with an explicit role (no sockets).
    pub async fn connect_in_process(
        self: &std::sync::Arc<Self>,
        role: aa_core::pb::Role,
    ) -> aa_core::AaResult<tonic::transport::Channel> {
        listen::connect_in_process(self, role).await
    }

    /// The single enforcement point bound to this engine.
    pub fn gate(self: &Arc<Self>) -> Gate {
        Gate::new(Arc::clone(self))
    }

    /// Requests shutdown of every listener (as the `Shutdown` RPC does).
    pub fn request_shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    pub(crate) fn system_principal(&self) -> pb::Principal {
        pb::Principal {
            role: pb::Role::EngineInternal as i32,
            channel: pb::Channel::EngineInternal as i32,
            uid: self.owner_uid,
            pid: i32::try_from(std::process::id()).unwrap_or(0),
            client_name: "aad".into(),
            client_version: self.cfg.engine_version.clone(),
            ..Default::default()
        }
    }

    /// The principal a pipeline action runs as: engine-internal, on behalf of `caller`.
    pub(crate) fn pipeline_principal(&self, caller: &pb::Principal) -> pb::Principal {
        pb::Principal {
            role: pb::Role::EngineInternal as i32,
            channel: pb::Channel::EngineInternal as i32,
            uid: caller.uid,
            pid: caller.pid,
            client_name: format!("aad pipeline for {}", caller.role().as_str_name()),
            client_version: self.cfg.engine_version.clone(),
            ..Default::default()
        }
    }

    /// Computed from proofs (architecture §5.2): separated only when no agent uid is the operator's.
    pub(crate) fn assurance(&self) -> pb::AssuranceTier {
        if self.cfg.agent_uids.contains(&self.owner_uid) {
            pb::AssuranceTier::SamePrincipal
        } else {
            pb::AssuranceTier::SeparatedAgent
        }
    }

    pub(crate) fn admin(
        &self,
        principal: &pb::Principal,
        kind: pb::AdminEventKind,
        subject: &str,
        details: serde_json::Value,
    ) -> AaResult<u64> {
        let event = pb::AdminEvent {
            kind: kind as i32,
            subject: subject.to_string(),
            details_json: details.to_string(),
            operator_note: None,
        };
        self.audit
            .append_admin(principal, self.halt.current().halt_epoch, event)
    }

    /// Appends an ActionRecord. If the audit log cannot take it, the engine halts: nothing may be dispatched
    /// that cannot be recorded.
    pub(crate) fn record_action(&self, principal: &pb::Principal, rec: &pb::ActionRecord) {
        if let Err(e) = self
            .audit
            .append_action(principal, self.halt.current().halt_epoch, rec.clone())
        {
            tracing::error!(error = %e, action = %rec.action_id, "audit append failed; halting");
            self.halt.halt(
                pb::HaltSource::Unspecified,
                pb::HaltScope::Engine,
                "",
                &format!("audit append failed: {}", e.message),
            );
        }
    }

    /// Locates a capability by id in the lock of the target whose id is its longest prefix.
    pub(crate) fn resolve(&self, capability_id: &str) -> AaResult<Resolved> {
        let missing = || {
            ports::not_found(
                format!("unknown capability {capability_id:?}"),
                "aa list (or aa discover <target>)",
            )
        };
        let mut targets: Vec<String> = self
            .store
            .list_targets()?
            .into_iter()
            .filter(|t| capability_id.len() > t.len() + 1 && capability_id.starts_with(&format!("{t}.")))
            .collect();
        targets.sort_by_key(|t| std::cmp::Reverse(t.len()));
        for t in targets {
            if let Some(lock) = self.store.read_lock(&t)?
                && let Some(idx) = lock.capabilities.iter().position(|c| c.id == capability_id)
            {
                return Ok(Resolved { lock, idx });
            }
        }
        Err(missing())
    }

    /// Withholds one capability in its lock (`SIDE_EFFECT` observed at runtime) and audits the change.
    pub(crate) fn auto_withhold(
        &self,
        principal: &pb::Principal,
        target_id: &str,
        capability_id: &str,
        hint: &str,
    ) -> AaResult<()> {
        let Some(mut lock) = self.store.read_lock(target_id)? else {
            return Ok(());
        };
        let Some(cap) = lock.capabilities.iter_mut().find(|c| c.id == capability_id) else {
            return Ok(());
        };
        let adm = cap.admission.get_or_insert_with(Default::default);
        adm.state = pb::AdmissionState::Withheld as i32;
        adm.withheld.push(pb::Withheld {
            reason: pb::WithheldReason::SideEffect as i32,
            hint: format!("observed at runtime: {hint}"),
            evidence_ids: vec![],
        });
        self.sem.finalize_digest(&mut lock)?;
        self.store.write_lock(&lock)?;
        self.admin(
            principal,
            pb::AdminEventKind::AdmissionChanged,
            capability_id,
            serde_json::json!({"state": "ADMISSION_STATE_WITHHELD", "reason": "WITHHELD_REASON_SIDE_EFFECT", "hint": hint}),
        )?;
        Ok(())
    }

    /// Engages the kill switch from `source`, cancels in-flight actions (by broadcasting the new state) and
    /// audits it. Returns the new state and how many actions were in flight.
    pub(crate) fn engage_halt(
        &self,
        principal: &pb::Principal,
        source: pb::HaltSource,
        scope: pb::HaltScope,
        subject: &str,
        reason: &str,
    ) -> (pb::HaltState, u32) {
        let inflight = u32::try_from(self.inflight.len()).unwrap_or(u32::MAX);
        let state = self.halt.halt(source, scope, subject, reason);
        let details = serde_json::json!({
            "halt_epoch": state.halt_epoch, "source": source.as_str_name(), "scope": scope.as_str_name(),
            "scope_subject": subject, "reason": reason, "cancelled_actions": inflight,
        });
        if let Err(e) = self.admin(principal, pb::AdminEventKind::Halt, "engine", details) {
            tracing::error!(error = %e, "could not audit HALT");
        }
        (state, inflight)
    }
}

/// `AaError` -> tonic status code for errors raised before an ActionRecord exists.
pub(crate) fn code_for(err: &AaError) -> tonic::Code {
    use pb::ErrorCode as C;
    match err.code {
        C::UnresolvedSelector => tonic::Code::NotFound,
        C::InvalidRequest => tonic::Code::InvalidArgument,
        C::AccessDenied => tonic::Code::PermissionDenied,
        C::PolicyDenied | C::Excluded | C::CapabilityWithheld => tonic::Code::FailedPrecondition,
        C::EngineHalted | C::AuditUnavailable | C::TargetUnavailable | C::CapabilityUnavailable => {
            tonic::Code::Unavailable
        }
        C::ConfigurationInvalid | C::IsolationNotProven | C::TargetIdentityChanged | C::CatalogStale => {
            tonic::Code::FailedPrecondition
        }
        C::Timeout => tonic::Code::DeadlineExceeded,
        C::Cancelled => tonic::Code::Cancelled,
        C::ResourceConstraint => tonic::Code::ResourceExhausted,
        _ => tonic::Code::Internal,
    }
}

/// `AaError` as a tonic status with the encoded aa `Error` in the details.
pub(crate) fn to_status(err: &AaError) -> tonic::Status {
    role_layer::status_for(code_for(err), err)
}
