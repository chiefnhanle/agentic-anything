//! Engine-local seams over the crates the engine composes (WP1 `aa-core` semantics, WP2 `aa-exec`, WP3
//! `aa-harvest`, WP4 `aa-store`).
//!
//! The engine never calls those crates directly outside this module: every call goes through one of the
//! traits below. [`Engine::build`](crate::Engine::build) wires the production implementations (the `Real*`
//! types / impls at the bottom of this file); the crate's own tests wire in-memory fakes
//! (`crate::testing`), so the gate, the role layer and the services are tested in isolation from the other
//! work packages (build-plan §2: "each package tests against fakes").

use aa_core::{
    AaError, AaResult,
    api::AuditSink,
    manifest::{CompileInput, CompileOutput, Overlay},
    pb,
    policy::{ActionDescriptor, paths::PathScope},
};
use aa_exec::lanes::LaneCtx;
use aa_store::{AuditQuery, BootReport};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

// ---------------------------------------------------------------------------------------------------------
// Store (workspace + CAS)
// ---------------------------------------------------------------------------------------------------------

/// Metadata of one raw harvest artifact. `SourceRef` (the harvest index) does not carry `name` and
/// `media_type`, which `Harvester::candidates` may need, so the engine keeps this sidecar next to the index
/// (`targets/<id>/harvest/raw-meta.json`) to rebuild exact `RawHarvest`s from CAS bytes at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMeta {
    pub harvester: String,
    pub name: String,
    pub media_type: String,
    pub uri: String,
    pub authority: i32,
    pub sha256: String,
}

/// The on-disk workspace (`aa_store::Workspace` in production).
pub trait Store: Send + Sync {
    fn root(&self) -> PathBuf;
    fn list_targets(&self) -> AaResult<Vec<String>>;
    fn read_overlay(&self, target_id: &str) -> AaResult<Option<String>>;
    fn write_overlay(&self, target_id: &str, yaml: &str) -> AaResult<String>;
    fn overlay_path(&self, target_id: &str) -> PathBuf;
    fn read_target(&self, target_id: &str) -> AaResult<Option<pb::Target>>;
    fn write_target(&self, target: &pb::Target) -> AaResult<()>;
    fn read_lock(&self, target_id: &str) -> AaResult<Option<pb::CapabilityLock>>;
    fn write_lock(&self, lock: &pb::CapabilityLock) -> AaResult<String>;
    fn write_harvest_index(&self, target_id: &str, sources: &[pb::SourceRef]) -> AaResult<()>;
    fn read_harvest_index(&self, target_id: &str) -> AaResult<Vec<pb::SourceRef>>;
    fn write_raw_meta(&self, target_id: &str, meta: &[RawMeta]) -> AaResult<()>;
    fn read_raw_meta(&self, target_id: &str) -> AaResult<Vec<RawMeta>>;
    fn write_verification(&self, rec: &pb::VerificationRecord) -> AaResult<()>;
    fn read_verifications(&self, target_id: &str) -> AaResult<Vec<pb::VerificationRecord>>;
    fn cas_put(&self, bytes: &[u8]) -> AaResult<String>;
    fn cas_get(&self, sha256: &str) -> AaResult<Vec<u8>>;
    fn sessions_dir(&self) -> PathBuf;
    fn scratch_dir(&self) -> PathBuf;
    fn hmac_key(&self) -> AaResult<[u8; 32]>;
}

/// The audit log: the frozen [`AuditSink`] plus the read side (`aa_store::AuditLog` in production).
pub trait AuditStore: AuditSink {
    fn query(&self, q: &AuditQuery) -> AaResult<Vec<pb::AuditEntry>>;
    fn verify(&self) -> BootReport;
    fn find_action(&self, action_id: &str) -> AaResult<Option<pb::ActionRecord>>;
    fn find_by_idempotency_key(&self, key: &str) -> AaResult<Option<pb::ActionRecord>>;
}

/// Kill-switch persistence (`aa_store::HaltStore` in production).
pub trait HaltPersist: Send + Sync {
    fn load(&self) -> pb::HaltState;
    fn save(&self, s: &pb::HaltState) -> AaResult<()>;
    fn sentinel_present(&self) -> bool;
}

// ---------------------------------------------------------------------------------------------------------
// Semantics (pure core + identity + lanes + CLI-Anything import)
// ---------------------------------------------------------------------------------------------------------

/// A loaded policy evaluator (`aa_core::policy::Policy` in production).
pub trait PolicyEval: Send + Sync {
    fn evaluate(&self, d: &ActionDescriptor<'_>) -> pb::PolicyDecision;
    fn remediation_for(&self, reason: pb::DenialReason, d: &ActionDescriptor<'_>) -> String;
}

/// A CLI-Anything harness import (mirrors `aa_harvest::cli_anything::ImportedHarness`).
pub struct Imported {
    pub target: pb::Target,
    pub overlay_yaml: String,
    pub raw: Vec<aa_core::api::RawHarvest>,
    pub install_cmd_flags: Vec<String>,
}

#[async_trait::async_trait]
pub trait Semantics: Send + Sync {
    // effects (aa_core::effects)
    fn level(&self, e: &pb::Effects) -> pb::EffectLevel;
    fn tier(&self, e: &pb::Effects, binding: &pb::Binding) -> pb::AuthorityTier;
    fn apply_branches(
        &self,
        base: &pb::Effects,
        branches: &[pb::EffectBranch],
        input: &serde_json::Value,
    ) -> pb::Effects;
    fn tier_rank(&self, t: pb::AuthorityTier) -> u8;

    // policy (aa_core::policy)
    fn load_policy(&self, toml_text: &str) -> AaResult<Arc<dyn PolicyEval>>;
    fn policy_digest(&self, toml_text: &str) -> String;
    fn canonicalize_for_scope(&self, p: &Path, write: bool) -> AaResult<PathBuf>;
    fn check_scope(&self, scope: &PathScope, canonical: &Path, write: bool) -> Result<(), pb::DenialReason>;

    // identity (aa_exec::identity)
    fn file_identity(&self, role: &str, path: &Path) -> AaResult<pb::FileIdentity>;
    /// Like `file_identity`, but keeps the invoked path (a venv's `python` symlink selects the venv).
    fn file_identity_as_invoked(&self, role: &str, path: &Path) -> AaResult<pb::FileIdentity>;
    fn identity_digest(&self, id: &pb::TargetIdentity) -> AaResult<String>;
    fn verify_identity(&self, id: &pb::TargetIdentity) -> AaResult<()>;

    // manifest (aa_core::manifest)
    fn parse_overlay(&self, yaml: &str) -> AaResult<Overlay>;
    fn starter_overlay(&self, target: &pb::Target, harvesters: &[String]) -> String;
    fn compile(&self, input: CompileInput<'_>, previous: Option<&pb::CapabilityLock>) -> AaResult<CompileOutput>;
    fn finalize_digest(&self, lock: &mut pb::CapabilityLock) -> AaResult<()>;

    // lanes (aa_core::lanes, aa_core::admission, aa_exec::lanes)
    fn required_lanes(&self, level: pb::EffectLevel, cap: &pb::Capability) -> Vec<pb::Lane>;
    fn run_l0(&self, lock: &pb::CapabilityLock, cap: &pb::Capability, engine_version: &str) -> pb::VerificationRecord;
    async fn run_l1(&self, ctx: &LaneCtx<'_>, cap: &pb::Capability) -> pb::VerificationRecord;
    async fn run_l3(&self, ctx: &LaneCtx<'_>, cap: &pb::Capability) -> pb::VerificationRecord;

    // CLI-Anything (aa_harvest::cli_anything)
    fn registry_names(&self, repo: &Path) -> AaResult<Vec<String>>;
    fn import_harness(&self, repo: &Path, name: &str, python: &Path, authorization: &str) -> AaResult<Imported>;
}

// ---------------------------------------------------------------------------------------------------------
// Production implementations
// ---------------------------------------------------------------------------------------------------------

impl Store for aa_store::Workspace {
    fn root(&self) -> PathBuf {
        aa_store::Workspace::root(self).to_path_buf()
    }
    fn list_targets(&self) -> AaResult<Vec<String>> {
        aa_store::Workspace::list_targets(self)
    }
    fn read_overlay(&self, target_id: &str) -> AaResult<Option<String>> {
        aa_store::Workspace::read_overlay(self, target_id)
    }
    fn write_overlay(&self, target_id: &str, yaml: &str) -> AaResult<String> {
        aa_store::Workspace::write_overlay(self, target_id, yaml)
    }
    fn overlay_path(&self, target_id: &str) -> PathBuf {
        self.target_dir(target_id).join("target.aa.yaml")
    }
    fn read_target(&self, target_id: &str) -> AaResult<Option<pb::Target>> {
        aa_store::Workspace::read_target(self, target_id)
    }
    fn write_target(&self, target: &pb::Target) -> AaResult<()> {
        aa_store::Workspace::write_target(self, target)
    }
    fn read_lock(&self, target_id: &str) -> AaResult<Option<pb::CapabilityLock>> {
        aa_store::Workspace::read_lock(self, target_id)
    }
    fn write_lock(&self, lock: &pb::CapabilityLock) -> AaResult<String> {
        aa_store::Workspace::write_lock(self, lock)
    }
    fn write_harvest_index(&self, target_id: &str, sources: &[pb::SourceRef]) -> AaResult<()> {
        aa_store::Workspace::write_harvest_index(self, target_id, sources)
    }
    fn read_harvest_index(&self, target_id: &str) -> AaResult<Vec<pb::SourceRef>> {
        aa_store::Workspace::read_harvest_index(self, target_id)
    }
    fn write_raw_meta(&self, target_id: &str, meta: &[RawMeta]) -> AaResult<()> {
        let dir = self.target_dir(target_id).join("harvest");
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join(".raw-meta.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(meta)?)?;
        std::fs::rename(&tmp, dir.join("raw-meta.json"))?;
        Ok(())
    }
    fn read_raw_meta(&self, target_id: &str) -> AaResult<Vec<RawMeta>> {
        let path = self.target_dir(target_id).join("harvest").join("raw-meta.json");
        match std::fs::read(&path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }
    fn write_verification(&self, rec: &pb::VerificationRecord) -> AaResult<()> {
        aa_store::Workspace::write_verification(self, rec)
    }
    fn read_verifications(&self, target_id: &str) -> AaResult<Vec<pb::VerificationRecord>> {
        aa_store::Workspace::read_verifications(self, target_id)
    }
    fn cas_put(&self, bytes: &[u8]) -> AaResult<String> {
        self.cas().put(bytes)
    }
    fn cas_get(&self, sha256: &str) -> AaResult<Vec<u8>> {
        self.cas().get(sha256)
    }
    fn sessions_dir(&self) -> PathBuf {
        aa_store::Workspace::sessions_dir(self)
    }
    fn scratch_dir(&self) -> PathBuf {
        aa_store::Workspace::scratch_dir(self)
    }
    fn hmac_key(&self) -> AaResult<[u8; 32]> {
        aa_store::Workspace::hmac_key(self)
    }
}

impl AuditStore for aa_store::AuditLog {
    fn query(&self, q: &AuditQuery) -> AaResult<Vec<pb::AuditEntry>> {
        aa_store::AuditLog::query(self, q)
    }
    fn verify(&self) -> BootReport {
        self.verify_chain()
    }
    fn find_action(&self, action_id: &str) -> AaResult<Option<pb::ActionRecord>> {
        aa_store::AuditLog::find_action(self, action_id)
    }
    fn find_by_idempotency_key(&self, key: &str) -> AaResult<Option<pb::ActionRecord>> {
        aa_store::AuditLog::find_by_idempotency_key(self, key)
    }
}

impl HaltPersist for aa_store::HaltStore {
    fn load(&self) -> pb::HaltState {
        aa_store::HaltStore::load(self)
    }
    fn save(&self, s: &pb::HaltState) -> AaResult<()> {
        aa_store::HaltStore::save(self, s)
    }
    fn sentinel_present(&self) -> bool {
        aa_store::HaltStore::sentinel_present(self)
    }
}

struct RealPolicy(aa_core::policy::Policy);

impl PolicyEval for RealPolicy {
    fn evaluate(&self, d: &ActionDescriptor<'_>) -> pb::PolicyDecision {
        aa_core::policy::evaluate(&self.0, d)
    }
    fn remediation_for(&self, reason: pb::DenialReason, d: &ActionDescriptor<'_>) -> String {
        aa_core::policy::remediation_for(reason, d)
    }
}

/// The production [`Semantics`]: delegates to `aa-core`, `aa-exec` and `aa-harvest`.
pub struct RealSemantics {
    identity: aa_exec::IdentityCache,
}

impl RealSemantics {
    pub fn new() -> Self {
        Self {
            identity: aa_exec::IdentityCache::new(),
        }
    }
}

impl Default for RealSemantics {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Semantics for RealSemantics {
    fn level(&self, e: &pb::Effects) -> pb::EffectLevel {
        aa_core::effects::level(e)
    }
    fn tier(&self, e: &pb::Effects, binding: &pb::Binding) -> pb::AuthorityTier {
        aa_core::effects::tier(e, binding)
    }
    fn apply_branches(
        &self,
        base: &pb::Effects,
        branches: &[pb::EffectBranch],
        input: &serde_json::Value,
    ) -> pb::Effects {
        aa_core::effects::apply_branches(base, branches, input)
    }
    fn tier_rank(&self, t: pb::AuthorityTier) -> u8 {
        aa_core::effects::tier_rank(t)
    }
    fn load_policy(&self, toml_text: &str) -> AaResult<Arc<dyn PolicyEval>> {
        Ok(Arc::new(RealPolicy(aa_core::policy::parse_policy(toml_text)?)))
    }
    fn policy_digest(&self, toml_text: &str) -> String {
        aa_core::policy::policy_digest(toml_text)
    }
    fn canonicalize_for_scope(&self, p: &Path, write: bool) -> AaResult<PathBuf> {
        aa_core::policy::paths::canonicalize_for_scope(p, write)
    }
    fn check_scope(&self, scope: &PathScope, canonical: &Path, write: bool) -> Result<(), pb::DenialReason> {
        aa_core::policy::paths::check(scope, canonical, write)
    }
    fn file_identity(&self, role: &str, path: &Path) -> AaResult<pb::FileIdentity> {
        // STATIC registration: never run the target (`version_argv = None`).
        aa_exec::file_identity(role, path, None)
    }
    fn file_identity_as_invoked(&self, role: &str, path: &Path) -> AaResult<pb::FileIdentity> {
        aa_exec::identity::file_identity_as_invoked(role, path, None)
    }
    fn identity_digest(&self, id: &pb::TargetIdentity) -> AaResult<String> {
        aa_exec::identity_digest(id)
    }
    fn verify_identity(&self, id: &pb::TargetIdentity) -> AaResult<()> {
        self.identity.verify(id)
    }
    fn parse_overlay(&self, yaml: &str) -> AaResult<Overlay> {
        aa_core::manifest::parse_overlay(yaml)
    }
    fn starter_overlay(&self, target: &pb::Target, harvesters: &[String]) -> String {
        aa_core::manifest::starter_overlay(target, harvesters)
    }
    fn compile(&self, input: CompileInput<'_>, previous: Option<&pb::CapabilityLock>) -> AaResult<CompileOutput> {
        aa_core::manifest::compile(input, previous)
    }
    fn finalize_digest(&self, lock: &mut pb::CapabilityLock) -> AaResult<()> {
        aa_core::manifest::finalize_digest(lock)
    }
    fn required_lanes(&self, level: pb::EffectLevel, cap: &pb::Capability) -> Vec<pb::Lane> {
        aa_core::admission::required_lanes(level, cap)
    }
    fn run_l0(&self, lock: &pb::CapabilityLock, cap: &pb::Capability, engine_version: &str) -> pb::VerificationRecord {
        aa_core::lanes::run_l0(lock, cap, engine_version)
    }
    async fn run_l1(&self, ctx: &LaneCtx<'_>, cap: &pb::Capability) -> pb::VerificationRecord {
        aa_exec::lanes::run_parser_lane(ctx, cap).await
    }
    async fn run_l3(&self, ctx: &LaneCtx<'_>, cap: &pb::Capability) -> pb::VerificationRecord {
        aa_exec::lanes::run_sandbox_diff_lane(ctx, cap).await
    }
    fn registry_names(&self, repo: &Path) -> AaResult<Vec<String>> {
        Ok(aa_harvest::cli_anything::read_registry(repo)?
            .into_iter()
            .map(|e| e.name)
            .collect())
    }
    fn import_harness(&self, repo: &Path, name: &str, python: &Path, authorization: &str) -> AaResult<Imported> {
        let h = aa_harvest::cli_anything::import_harness(repo, name, python, authorization)?;
        Ok(Imported {
            target: h.target,
            overlay_yaml: h.overlay_yaml,
            raw: h.raw,
            install_cmd_flags: h.install_cmd_flags,
        })
    }
}

/// The production store, audit log (with its boot report) and halt store.
pub(crate) type ProductionParts = (Arc<dyn Store>, Arc<dyn AuditStore>, BootReport, Arc<dyn HaltPersist>);

/// Opens the production store, audit log and halt store under `workspace`.
pub(crate) fn open_production(workspace: &Path) -> AaResult<ProductionParts> {
    let ws = aa_store::Workspace::open(workspace)?;
    let root = aa_store::Workspace::root(&ws).to_path_buf();
    let (audit, report) = aa_store::AuditLog::open(&root.join("audit"))?;
    let halt = aa_store::HaltStore::open(&root.join("state"))?;
    Ok((Arc::new(ws), Arc::new(audit), report, Arc::new(halt)))
}

/// `AaError` for a missing object, mapped to `NOT_FOUND` at the transport.
pub(crate) fn not_found(what: impl Into<String>, remediation: impl Into<String>) -> AaError {
    AaError::new(pb::ErrorCode::UnresolvedSelector, what, remediation)
}
