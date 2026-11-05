//! Test doubles for the engine's seams (`crate::ports`) and a harness that assembles an [`Engine`] from them.
//!
//! The other work packages' crates are not exercised here (build-plan §2): `MemoryAudit` and `FakeRunner`
//! come from `aa_core::api::testing`; the store, halt persistence, semantics and binding are in-memory or
//! deliberately simple stand-ins with the frozen contracts' observable behaviour.

use crate::{
    Engine, EngineConfig, EngineDeps, Parts,
    ports::{AuditStore, HaltPersist, Imported, PolicyEval, RawMeta, Semantics, Store},
    status::lock,
};
use aa_core::{
    AaError, AaResult,
    api::{
        AuditSink, Binding, BindingKind, BindingOutput, Harvester, InvokeCtx, ProcessRunner, RunSpec,
        testing::{FakeRunner, MemoryAudit},
    },
    manifest::{CompileInput, CompileOutput, Overlay},
    pb,
    policy::{ActionDescriptor, paths::PathScope},
};
use aa_exec::lanes::LaneCtx;
use aa_store::{AuditQuery, BootReport};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

// ---------------------------------------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemState {
    overlays: HashMap<String, String>,
    targets: BTreeMap<String, pb::Target>,
    locks: HashMap<String, pb::CapabilityLock>,
    index: HashMap<String, Vec<pb::SourceRef>>,
    meta: HashMap<String, Vec<RawMeta>>,
    verifications: Vec<pb::VerificationRecord>,
    cas: HashMap<String, Vec<u8>>,
}

pub(crate) struct MemStore {
    root: PathBuf,
    state: Mutex<MemState>,
}

impl MemStore {
    pub fn new(root: &Path) -> Self {
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::create_dir_all(root.join("scratch")).unwrap();
        Self {
            root: std::fs::canonicalize(root).unwrap(),
            state: Mutex::default(),
        }
    }
}

impl Store for MemStore {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }
    fn list_targets(&self) -> AaResult<Vec<String>> {
        Ok(lock(&self.state).targets.keys().cloned().collect())
    }
    fn read_overlay(&self, target_id: &str) -> AaResult<Option<String>> {
        Ok(lock(&self.state).overlays.get(target_id).cloned())
    }
    fn write_overlay(&self, target_id: &str, yaml: &str) -> AaResult<String> {
        lock(&self.state).overlays.insert(target_id.into(), yaml.into());
        Ok(aa_core::digest::sha256_hex(yaml.as_bytes()))
    }
    fn overlay_path(&self, target_id: &str) -> PathBuf {
        self.root.join("targets").join(target_id).join("target.aa.yaml")
    }
    fn read_target(&self, target_id: &str) -> AaResult<Option<pb::Target>> {
        Ok(lock(&self.state).targets.get(target_id).cloned())
    }
    fn write_target(&self, target: &pb::Target) -> AaResult<()> {
        lock(&self.state).targets.insert(target.id.clone(), target.clone());
        Ok(())
    }
    fn read_lock(&self, target_id: &str) -> AaResult<Option<pb::CapabilityLock>> {
        Ok(lock(&self.state).locks.get(target_id).cloned())
    }
    fn write_lock(&self, l: &pb::CapabilityLock) -> AaResult<String> {
        let id = l.target.as_ref().map(|t| t.id.clone()).unwrap_or_default();
        lock(&self.state).locks.insert(id, l.clone());
        Ok(l.lock_digest.clone())
    }
    fn write_harvest_index(&self, target_id: &str, sources: &[pb::SourceRef]) -> AaResult<()> {
        lock(&self.state).index.insert(target_id.into(), sources.to_vec());
        Ok(())
    }
    fn read_harvest_index(&self, target_id: &str) -> AaResult<Vec<pb::SourceRef>> {
        Ok(lock(&self.state).index.get(target_id).cloned().unwrap_or_default())
    }
    fn write_raw_meta(&self, target_id: &str, meta: &[RawMeta]) -> AaResult<()> {
        lock(&self.state).meta.insert(target_id.into(), meta.to_vec());
        Ok(())
    }
    fn read_raw_meta(&self, target_id: &str) -> AaResult<Vec<RawMeta>> {
        Ok(lock(&self.state).meta.get(target_id).cloned().unwrap_or_default())
    }
    fn write_verification(&self, rec: &pb::VerificationRecord) -> AaResult<()> {
        lock(&self.state).verifications.push(rec.clone());
        Ok(())
    }
    fn read_verifications(&self, target_id: &str) -> AaResult<Vec<pb::VerificationRecord>> {
        let prefix = format!("{target_id}.");
        Ok(lock(&self.state)
            .verifications
            .iter()
            .filter(|r| r.capability_id.starts_with(&prefix))
            .cloned()
            .collect())
    }
    fn cas_put(&self, bytes: &[u8]) -> AaResult<String> {
        let sha = aa_core::digest::sha256_hex(bytes);
        lock(&self.state).cas.insert(sha.clone(), bytes.to_vec());
        Ok(sha)
    }
    fn cas_get(&self, sha256: &str) -> AaResult<Vec<u8>> {
        lock(&self.state)
            .cas
            .get(sha256)
            .cloned()
            .ok_or_else(|| AaError::internal(format!("cas miss {sha256}")))
    }
    fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }
    fn scratch_dir(&self) -> PathBuf {
        self.root.join("scratch")
    }
    fn hmac_key(&self) -> AaResult<[u8; 32]> {
        Ok([7u8; 32])
    }
}

// ---------------------------------------------------------------------------------------------------------
// Audit (MemoryAudit) and halt persistence
// ---------------------------------------------------------------------------------------------------------

/// `MemoryAudit` plus the read side, and a switch that makes `verify()` report a broken chain.
pub(crate) struct TestAudit {
    pub mem: MemoryAudit,
    pub tampered: AtomicBool,
}

fn body_action_id(e: &pb::AuditEntry) -> Option<&str> {
    match &e.body {
        Some(pb::audit_entry::Body::Action(a)) => Some(&a.action_id),
        Some(pb::audit_entry::Body::Intent(i)) => Some(&i.action_id),
        _ => None,
    }
}

fn body_capability(e: &pb::AuditEntry) -> Option<&str> {
    match &e.body {
        Some(pb::audit_entry::Body::Action(a)) => Some(&a.capability_id),
        Some(pb::audit_entry::Body::Intent(i)) => Some(&i.capability_id),
        _ => None,
    }
}

impl AuditSink for TestAudit {
    fn append_intent(&self, p: &pb::Principal, epoch: u64, intent: pb::ActionIntent) -> AaResult<u64> {
        self.mem.append_intent(p, epoch, intent)
    }
    fn append_action(&self, p: &pb::Principal, epoch: u64, record: pb::ActionRecord) -> AaResult<u64> {
        self.mem.append_action(p, epoch, record)
    }
    fn append_admin(&self, p: &pb::Principal, epoch: u64, event: pb::AdminEvent) -> AaResult<u64> {
        self.mem.append_admin(p, epoch, event)
    }
    fn head(&self) -> (u64, String) {
        self.mem.head()
    }
    fn entry(&self, seq: u64) -> AaResult<Option<pb::AuditEntry>> {
        self.mem.entry(seq)
    }
}

impl AuditStore for TestAudit {
    fn query(&self, q: &AuditQuery) -> AaResult<Vec<pb::AuditEntry>> {
        let limit = if q.limit == 0 { usize::MAX } else { q.limit as usize };
        Ok(self
            .mem
            .entries()
            .into_iter()
            .filter(|e| e.seq > q.after_seq)
            .filter(|e| !q.actions_only || matches!(e.body, Some(pb::audit_entry::Body::Action(_))))
            .filter(|e| q.action_id.as_deref().is_none_or(|a| body_action_id(e) == Some(a)))
            .filter(|e| q.capability_id.as_deref().is_none_or(|c| body_capability(e) == Some(c)))
            .filter(|e| {
                q.target_id
                    .as_deref()
                    .is_none_or(|t| body_capability(e).is_some_and(|c| c.starts_with(&format!("{t}."))))
            })
            .take(limit)
            .collect())
    }
    fn verify(&self) -> BootReport {
        let ok = self.mem.verify_chain() && !self.tampered.load(Ordering::SeqCst);
        BootReport {
            chain_ok: ok,
            entries: self.mem.entries().len() as u64,
            first_broken_seq: (!ok).then_some(1),
            head_matches: true,
            recovered_action_ids: vec![],
        }
    }
    fn find_action(&self, action_id: &str) -> AaResult<Option<pb::ActionRecord>> {
        Ok(self.mem.actions().into_iter().rev().find(|a| a.action_id == action_id))
    }
    fn find_by_idempotency_key(&self, key: &str) -> AaResult<Option<pb::ActionRecord>> {
        Ok(self.mem.actions().into_iter().rev().find(|a| a.idempotency_key == key))
    }
}

#[derive(Default)]
pub(crate) struct MemHalt {
    state: Mutex<pb::HaltState>,
    sentinel: AtomicBool,
}

impl MemHalt {
    pub fn set_sentinel(&self, on: bool) {
        self.sentinel.store(on, Ordering::SeqCst);
    }
}

impl HaltPersist for MemHalt {
    fn load(&self) -> pb::HaltState {
        lock(&self.state).clone()
    }
    fn save(&self, s: &pb::HaltState) -> AaResult<()> {
        *lock(&self.state) = s.clone();
        Ok(())
    }
    fn sentinel_present(&self) -> bool {
        self.sentinel.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Semantics
// ---------------------------------------------------------------------------------------------------------

const T: i32 = pb::Tri::True as i32;
const F: i32 = pb::Tri::False as i32;

/// Every target-facing field FALSE (a decided READ); `idempotent` TRUE.
pub(crate) fn read_effects() -> pb::Effects {
    pb::Effects {
        mutates_target: F,
        writes_filesystem: F,
        launches_process: F,
        accesses_network: F,
        changes_ui_state: F,
        sends_input: F,
        may_discard_data: F,
        executes_foreign_code: F,
        mutates_device: F,
        external_side_effect: F,
        attaches_process: F,
        reads_screen: F,
        reads_clipboard: F,
        writes_clipboard: F,
        reads_credentials: F,
        irreversible: F,
        mutates_session: F,
        idempotent: T,
    }
}

pub(crate) fn write_effects() -> pb::Effects {
    pb::Effects {
        mutates_target: T,
        writes_filesystem: T,
        idempotent: F,
        ..read_effects()
    }
}

fn target_fields(e: &pb::Effects) -> [i32; 16] {
    [
        e.mutates_target,
        e.writes_filesystem,
        e.launches_process,
        e.accesses_network,
        e.changes_ui_state,
        e.sends_input,
        e.may_discard_data,
        e.executes_foreign_code,
        e.mutates_device,
        e.external_side_effect,
        e.attaches_process,
        e.reads_screen,
        e.reads_clipboard,
        e.writes_clipboard,
        e.reads_credentials,
        e.irreversible,
    ]
}

/// A minimal policy: per-capability deny rules, then allow by level for registered targets, else default deny.
#[derive(Clone, Default)]
pub(crate) struct FakePolicy {
    pub allow_levels: Vec<pb::EffectLevel>,
    pub deny_caps: Vec<String>,
}

impl PolicyEval for FakePolicy {
    fn evaluate(&self, d: &ActionDescriptor<'_>) -> pb::PolicyDecision {
        if self.deny_caps.iter().any(|c| c == d.capability_id) {
            let rule = format!("deny-{}", d.capability_id);
            return pb::PolicyDecision {
                verdict: pb::Verdict::Deny as i32,
                matched_rule_ids: vec![rule.clone()],
                deciding_rule_id: rule,
                denial_reason: pb::DenialReason::DenyRule as i32,
                ..Default::default()
            };
        }
        if d.target_registered && self.allow_levels.contains(&d.level) {
            let rule = format!("allow-{}", d.level.as_str_name());
            return pb::PolicyDecision {
                verdict: pb::Verdict::Allow as i32,
                matched_rule_ids: vec![rule.clone()],
                deciding_rule_id: rule,
                approval_source: pb::ApprovalSource::PolicyAllow as i32,
                ..Default::default()
            };
        }
        pb::PolicyDecision {
            verdict: pb::Verdict::Deny as i32,
            denial_reason: pb::DenialReason::NoMatchingAllow as i32,
            ..Default::default()
        }
    }
    fn remediation_for(&self, reason: pb::DenialReason, d: &ActionDescriptor<'_>) -> String {
        format!("aa policy explain {} ({})", d.capability_id, reason.as_str_name())
    }
}

pub(crate) struct FakeSemantics {
    pub policy: Mutex<FakePolicy>,
    pub overlay: Mutex<Overlay>,
}

impl Default for FakeSemantics {
    fn default() -> Self {
        Self {
            policy: Mutex::new(FakePolicy {
                allow_levels: vec![pb::EffectLevel::Read],
                deny_caps: vec![],
            }),
            overlay: Mutex::new(Overlay {
                schema: "aa.overlay/v1".into(),
                ..Default::default()
            }),
        }
    }
}

fn sha256_file(p: &Path) -> AaResult<String> {
    Ok(aa_core::digest::sha256_hex(&std::fs::read(p)?))
}

fn record(cap: &pb::Capability, lane: pb::Lane, status: pb::LaneStatus) -> pb::VerificationRecord {
    pb::VerificationRecord {
        capability_id: cap.id.clone(),
        capability_digest: cap.capability_digest.clone(),
        lane: lane as i32,
        status: status as i32,
        isolation: pb::Isolation::Process as i32,
        non_claims: vec!["fake lane".into()],
        ..Default::default()
    }
}

#[async_trait::async_trait]
impl Semantics for FakeSemantics {
    fn level(&self, e: &pb::Effects) -> pb::EffectLevel {
        let f = target_fields(e);
        if f.contains(&(pb::Tri::Unknown as i32)) {
            return pb::EffectLevel::Unknown;
        }
        let on = |v: i32| v == T;
        if on(e.may_discard_data) || on(e.irreversible) || on(e.external_side_effect) || on(e.executes_foreign_code) {
            pb::EffectLevel::Destructive
        } else if on(e.mutates_target) || on(e.writes_filesystem) || on(e.launches_process) || on(e.mutates_device) {
            pb::EffectLevel::Write
        } else if on(e.changes_ui_state) || on(e.sends_input) {
            pb::EffectLevel::Ui
        } else {
            pb::EffectLevel::Read
        }
    }
    fn tier(&self, e: &pb::Effects, _binding: &pb::Binding) -> pb::AuthorityTier {
        match self.level(e) {
            pb::EffectLevel::Unknown => pb::AuthorityTier::Unknown,
            pb::EffectLevel::Read => pb::AuthorityTier::Observe,
            pb::EffectLevel::Ui => pb::AuthorityTier::Drive,
            pb::EffectLevel::Write => pb::AuthorityTier::Write,
            pb::EffectLevel::Destructive => pb::AuthorityTier::Destructive,
        }
    }
    fn apply_branches(&self, base: &pb::Effects, _b: &[pb::EffectBranch], _i: &serde_json::Value) -> pb::Effects {
        *base
    }
    fn tier_rank(&self, t: pb::AuthorityTier) -> u8 {
        match t {
            pb::AuthorityTier::Unknown => u8::MAX,
            other => other as u8,
        }
    }
    fn load_policy(&self, _toml_text: &str) -> AaResult<Arc<dyn PolicyEval>> {
        Ok(Arc::new(lock(&self.policy).clone()))
    }
    fn policy_digest(&self, toml_text: &str) -> String {
        aa_core::digest::sha256_hex(toml_text.as_bytes())
    }
    fn canonicalize_for_scope(&self, p: &Path, write: bool) -> AaResult<PathBuf> {
        let bad = || AaError::invalid(format!("{} does not canonicalize", p.display()));
        if write {
            let parent = std::fs::canonicalize(p.parent().ok_or_else(bad)?).map_err(|_| bad())?;
            Ok(parent.join(p.file_name().ok_or_else(bad)?))
        } else {
            std::fs::canonicalize(p).map_err(|_| bad())
        }
    }
    fn check_scope(&self, scope: &PathScope, canonical: &Path, write: bool) -> Result<(), pb::DenialReason> {
        if scope.deny.iter().any(|d| canonical.starts_with(d)) {
            return Err(pb::DenialReason::OutsidePathScope);
        }
        let roots = if write { &scope.write_roots } else { &scope.read_roots };
        if roots.iter().any(|r| canonical.starts_with(r)) {
            Ok(())
        } else {
            Err(pb::DenialReason::OutsidePathScope)
        }
    }
    fn file_identity(&self, role: &str, path: &Path) -> AaResult<pb::FileIdentity> {
        let real = std::fs::canonicalize(path)?;
        Ok(pb::FileIdentity {
            role: role.into(),
            sha256: sha256_file(&real)?,
            realpath: real.display().to_string(),
            version_banner: String::new(),
        })
    }
    fn file_identity_as_invoked(&self, role: &str, path: &Path) -> AaResult<pb::FileIdentity> {
        let mut f = self.file_identity(role, path)?;
        f.realpath = path.display().to_string();
        Ok(f)
    }
    fn identity_digest(&self, id: &pb::TargetIdentity) -> AaResult<String> {
        aa_core::digest::self_digest(id, |i| i.identity_digest.clear())
    }
    fn verify_identity(&self, id: &pb::TargetIdentity) -> AaResult<()> {
        for f in &id.files {
            if sha256_file(Path::new(&f.realpath)).ok().as_deref() != Some(f.sha256.as_str()) {
                return Err(AaError::new(
                    pb::ErrorCode::TargetIdentityChanged,
                    format!("{} changed since it was pinned", f.realpath),
                    "aa discover <target>",
                )
                .with_recovery(pb::RecoveryKind::Redistill, "aa discover <target>"));
            }
        }
        Ok(())
    }
    fn parse_overlay(&self, _yaml: &str) -> AaResult<Overlay> {
        Ok(lock(&self.overlay).clone())
    }
    fn starter_overlay(&self, target: &pb::Target, harvesters: &[String]) -> String {
        format!(
            "schema: aa.overlay/v1\ntarget: {}\n# harvesters: {}\n",
            target.id,
            harvesters.join(",")
        )
    }
    /// Candidates start with undecided effects (level UNKNOWN), as real harvests do; a PASSED L3 record
    /// decides them READ; SERVABLE once L0, L1 and L3 have all PASSED.
    fn compile(&self, input: CompileInput<'_>, previous: Option<&pb::CapabilityLock>) -> AaResult<CompileOutput> {
        let mut caps = Vec::new();
        for c in input.candidates {
            let mut cap = c.capability.clone();
            cap.target_id = input.target.id.clone();
            if cap.input_schema_json.is_empty() {
                cap.input_schema_json = r#"{"additionalProperties":false,"type":"object"}"#.into();
            }
            cap.capability_digest = aa_core::digest::sha256_hex(cap.id.as_bytes());
            let passed = |lane: pb::Lane| {
                input.verification.iter().any(|r| {
                    r.capability_id == cap.id && r.lane == lane as i32 && r.status == pb::LaneStatus::Passed as i32
                })
            };
            let observed = passed(pb::Lane::SandboxDiff);
            let effects = if observed {
                read_effects()
            } else {
                pb::Effects::default()
            };
            let binding = cap.binding.unwrap_or_default();
            cap.level = self.level(&effects) as i32;
            cap.tier = self.tier(&effects, &binding) as i32;
            cap.effects = Some(effects);
            cap.binding = Some(binding);
            let servable = passed(pb::Lane::Static) && passed(pb::Lane::Parser) && observed;
            cap.admission = Some(admission(servable));
            caps.push(cap);
        }
        let mut lock = pb::CapabilityLock {
            schema: "aa.lock/v1".into(),
            target: Some(input.target),
            capabilities: caps,
            overlay_sha256: input.overlay_sha256,
            ..Default::default()
        };
        self.finalize_digest(&mut lock)?;
        let prev_ids: Vec<String> = previous
            .map(|p| p.capabilities.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default();
        let added = lock
            .capabilities
            .iter()
            .filter(|c| !prev_ids.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        Ok(CompileOutput {
            lock,
            added,
            removed: vec![],
            changed: vec![],
        })
    }
    fn finalize_digest(&self, lock: &mut pb::CapabilityLock) -> AaResult<()> {
        lock.lock_digest = aa_core::digest::self_digest(lock, |l| l.lock_digest.clear())?;
        Ok(())
    }
    fn required_lanes(&self, level: pb::EffectLevel, _cap: &pb::Capability) -> Vec<pb::Lane> {
        match level {
            pb::EffectLevel::Read => vec![pb::Lane::Static, pb::Lane::Parser, pb::Lane::SandboxDiff],
            _ => vec![pb::Lane::Static],
        }
    }
    fn run_l0(&self, _lock: &pb::CapabilityLock, cap: &pb::Capability, _v: &str) -> pb::VerificationRecord {
        record(cap, pb::Lane::Static, pb::LaneStatus::Passed)
    }
    async fn run_l1(&self, _ctx: &LaneCtx<'_>, cap: &pb::Capability) -> pb::VerificationRecord {
        record(cap, pb::Lane::Parser, pb::LaneStatus::Passed)
    }
    /// Runs the capability once through the lane's binding; PASSED iff it exits 0.
    async fn run_l3(&self, ctx: &LaneCtx<'_>, cap: &pb::Capability) -> pb::VerificationRecord {
        let ictx = InvokeCtx {
            action_id: "lane".into(),
            identity: ctx
                .lock
                .target
                .as_ref()
                .and_then(|t| t.identity.clone())
                .unwrap_or_default(),
            state_handle: None,
            scratch_parent: ctx.scratch_parent.to_path_buf(),
            deadline: Duration::from_secs(5),
            dry_run: false,
            confinement: None,
        };
        let ok = matches!(
            ctx.binding.invoke(cap, &serde_json::json!({}), &ictx).await,
            Ok(o) if o.outcome == pb::Outcome::Completed
        );
        record(
            cap,
            pb::Lane::SandboxDiff,
            if ok {
                pb::LaneStatus::Passed
            } else {
                pb::LaneStatus::Failed
            },
        )
    }
    fn registry_names(&self, _repo: &Path) -> AaResult<Vec<String>> {
        Ok(vec!["notes".into()])
    }
    fn import_harness(&self, _repo: &Path, name: &str, python: &Path, authorization: &str) -> AaResult<Imported> {
        Ok(Imported {
            target: pb::Target {
                id: format!("cli-anything.{name}"),
                kind: pb::TargetKind::CliAnythingHarness as i32,
                identity: Some(pb::TargetIdentity {
                    files: vec![self.file_identity("interpreter", python)?],
                    source_tree_sha: "tree123".into(),
                    ..Default::default()
                }),
                authorization_statement: authorization.into(),
                ..Default::default()
            },
            overlay_yaml: "schema: aa.overlay/v1\n".into(),
            raw: vec![aa_core::api::RawHarvest {
                harvester: "cli-anything-registry@1".into(),
                name: "install_cmd".into(),
                media_type: "text/plain".into(),
                uri: "registry.json#/install_cmd".into(),
                authority: pb::DiscoveryAuthority::Static as i32,
                bytes: b"pip install x; rm -rf /".to_vec(),
            }],
            install_cmd_flags: vec!["shell-metachar".into()],
        })
    }
}

pub(crate) fn admission(servable: bool) -> pb::Admission {
    if servable {
        pb::Admission {
            state: pb::AdmissionState::Servable as i32,
            required_runtime_isolation: pb::Isolation::Process as i32,
            ..Default::default()
        }
    } else {
        pb::Admission {
            state: pb::AdmissionState::Withheld as i32,
            withheld: vec![pb::Withheld {
                reason: pb::WithheldReason::NotVerified as i32,
                hint: "aa verify".into(),
                evidence_ids: vec![],
            }],
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Binding
// ---------------------------------------------------------------------------------------------------------

/// A `cli_argv`-kind binding that runs `[executable, command_path..., --k=v...]` through the runner and
/// returns `{text, lines}`.
pub(crate) struct FakeBinding {
    pub runner: Arc<dyn ProcessRunner>,
}

#[async_trait::async_trait]
impl Binding for FakeBinding {
    fn kind(&self) -> BindingKind {
        BindingKind::CliArgv
    }
    fn plan(&self, cap: &pb::Capability, input: &serde_json::Value, ctx: &InvokeCtx) -> AaResult<RunSpec> {
        let Some(pb::binding::Kind::Cli(cli)) = cap.binding.as_ref().and_then(|b| b.kind.clone()) else {
            return Err(AaError::internal("not a cli binding"));
        };
        let exe = ctx
            .identity
            .files
            .iter()
            .find(|f| f.role == cli.executable_role)
            .map(|f| f.realpath.clone())
            .ok_or_else(|| AaError::internal("no executable in identity"))?;
        let mut argv = vec![exe];
        argv.extend(cli.command_path.iter().cloned());
        if let Some(obj) = input.as_object() {
            for (k, v) in obj {
                argv.push(format!(
                    "--{k}={}",
                    v.as_str().map_or_else(|| v.to_string(), str::to_string)
                ));
            }
        }
        Ok(RunSpec {
            argv,
            cwd: ctx.scratch_parent.clone(),
            env: BTreeMap::new(),
            stdin: None,
            deadline: ctx.deadline,
            max_stdout: 1 << 20,
            max_stderr_excerpt: 1024,
            confinement: ctx.confinement.clone(),
            run_id: ctx.action_id.clone(),
        })
    }
    async fn invoke(
        &self,
        cap: &pb::Capability,
        input: &serde_json::Value,
        ctx: &InvokeCtx,
    ) -> AaResult<BindingOutput> {
        let spec = self.plan(cap, input, ctx)?;
        let out = self.runner.run(spec).await?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let ok = out.exit_code == Some(0);
        Ok(BindingOutput {
            outcome: if ok {
                pb::Outcome::Completed
            } else {
                pb::Outcome::Failed
            },
            output_json: Some(serde_json::json!({"text": text, "lines": text.lines().collect::<Vec<_>>()})),
            error: (!ok).then(|| AaError::new(pb::ErrorCode::UpstreamError, "target failed", "aa describe")),
            process: pb::ProcessReport {
                exit_code: out.exit_code.unwrap_or(-1),
                stdout_bytes: out.stdout_total,
                cleanup: out.cleanup as i32,
                ..Default::default()
            },
            observed_files: vec![],
        })
    }
}

// ---------------------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------------------

pub(crate) struct Harness {
    pub engine: Arc<Engine>,
    pub audit: Arc<TestAudit>,
    pub runner: Arc<FakeRunner>,
    pub store: Arc<MemStore>,
    pub halt: Arc<MemHalt>,
    pub dir: tempfile::TempDir,
}

pub(crate) struct HarnessBuilder {
    pub runner: FakeRunner,
    pub tampered: bool,
    pub harvesters: Vec<Arc<dyn Harvester>>,
    pub policy: FakePolicy,
    pub policy_toml: Option<String>,
    pub agent_uids: Option<Vec<u32>>,
    pub audit: Option<Arc<TestAudit>>,
}

impl Default for HarnessBuilder {
    fn default() -> Self {
        Self {
            runner: FakeRunner::new(),
            tampered: false,
            harvesters: vec![],
            policy: FakePolicy {
                allow_levels: vec![pb::EffectLevel::Read],
                deny_caps: vec![],
            },
            policy_toml: None,
            agent_uids: None,
            audit: None,
        }
    }
}

impl HarnessBuilder {
    pub fn build(self) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let store = Arc::new(MemStore::new(&ws));
        let audit = self.audit.unwrap_or_else(|| {
            Arc::new(TestAudit {
                mem: MemoryAudit::new(),
                tampered: AtomicBool::new(false),
            })
        });
        audit.tampered.store(self.tampered, Ordering::SeqCst);
        let halt = Arc::new(MemHalt::default());
        let runner = Arc::new(self.runner);
        let sem = Arc::new(FakeSemantics::default());
        *lock(&sem.policy) = self.policy;
        let policy_path = dir.path().join("policy.toml");
        if let Some(text) = &self.policy_toml {
            std::fs::write(&policy_path, text).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&policy_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let cfg = EngineConfig {
            workspace: ws,
            runtime_dir: dir.path().join("rt"),
            policy_path,
            agent_uids: self.agent_uids.unwrap_or_else(|| vec![crate::config::euid()]),
            engine_version: "0.1.0-test".into(),
        };
        let deps = EngineDeps {
            runner: runner.clone(),
            harvesters: self.harvesters,
            bindings: vec![Arc::new(FakeBinding { runner: runner.clone() })],
        };
        let boot = audit.verify();
        let engine = Engine::assemble(
            cfg,
            deps,
            Parts {
                store: store.clone(),
                audit: audit.clone(),
                boot,
                halt: halt.clone(),
                sem,
            },
        )
        .unwrap();
        Harness {
            engine,
            audit,
            runner,
            store,
            halt,
            dir,
        }
    }
}

impl Harness {
    pub fn new() -> Self {
        HarnessBuilder::default().build()
    }

    /// A fake executable file whose content is pinned in the target's identity.
    pub fn fake_exe(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, format!("#!/bin/sh\n# fake {name}\n")).unwrap();
        std::fs::canonicalize(p).unwrap()
    }

    /// Registers `target_id` (authorization "test") pinned to `exe`, with a lock holding `caps`.
    pub fn install(&self, target_id: &str, exe: &Path, caps: Vec<pb::Capability>) -> pb::CapabilityLock {
        let sem = FakeSemantics::default();
        let mut identity = pb::TargetIdentity {
            files: vec![sem.file_identity("executable", exe).unwrap()],
            ..Default::default()
        };
        identity.identity_digest = sem.identity_digest(&identity).unwrap();
        let target = pb::Target {
            id: target_id.into(),
            kind: pb::TargetKind::CliTool as i32,
            identity: Some(identity),
            authorization_statement: "test".into(),
            ..Default::default()
        };
        self.store.write_target(&target).unwrap();
        self.store.write_overlay(target_id, "schema: aa.overlay/v1\n").unwrap();
        let mut lock = pb::CapabilityLock {
            schema: "aa.lock/v1".into(),
            target: Some(target),
            capabilities: caps,
            ..Default::default()
        };
        sem.finalize_digest(&mut lock).unwrap();
        self.store.write_lock(&lock).unwrap();
        lock
    }

    pub async fn channel(&self, role: pb::Role) -> tonic::transport::Channel {
        self.engine.connect_in_process(role).await.unwrap()
    }

    pub fn actions(&self) -> Vec<pb::ActionRecord> {
        self.audit.mem.actions()
    }

    pub fn entries(&self) -> Vec<pb::AuditEntry> {
        self.audit.mem.entries()
    }
}

/// A capability `<target>.<name>` with a closed schema `{repo?: string, text?: string}`, a cli binding running
/// the target's executable with `command_path = [name]`, and `/repo` as a READ_PATH scope.
pub(crate) fn capability(target: &str, name: &str, effects: pb::Effects, servable: bool) -> pb::Capability {
    let sem = FakeSemantics::default();
    let binding = pb::Binding {
        kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
            executable_role: "executable".into(),
            command_path: vec![name.into()],
            output: pb::OutputMode::TextStdout as i32,
            kill_scope: pb::KillScope::OwnedGroup as i32,
            ..Default::default()
        })),
    };
    pb::Capability {
        id: format!("{target}.{name}"),
        target_id: target.into(),
        level: sem.level(&effects) as i32,
        tier: sem.tier(&effects, &binding) as i32,
        effects: Some(effects),
        input_schema_json: r#"{"additionalProperties":false,"properties":{"repo":{"type":"string"},"text":{"type":"string"}},"type":"object"}"#.into(),
        scopes: vec![pb::ScopeParam {
            pointer: "/repo".into(),
            kind: pb::ScopeKind::ReadPath as i32,
        }],
        binding: Some(binding),
        capability_digest: format!("digest-{name}"),
        admission: Some(admission(servable)),
        returns_target_content: true,
        ..Default::default()
    }
}

/// The aa `Error` encoded in a status's details.
pub(crate) fn decode_status(st: &tonic::Status) -> pb::Error {
    prost::Message::decode(st.details()).expect("status details carry an aa Error")
}
