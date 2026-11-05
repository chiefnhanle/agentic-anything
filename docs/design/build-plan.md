# Build plan: first vertical slice (M1, slice 1)

- **Status:** ready to implement (2026-11-05).
- **Design references:** `architecture.md`, ADRs `adr/0001`–`adr/0013` and protos `proto/agentic_anything/v1/*.proto` (buf lint clean).
- **Toolchain:** `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Versions: rustc 1.99.1, protoc 36.1, buf 1.75.0.

## 1. Slice goal and definition of done

**Goal:** register a real CLI and a Click/CLI-Anything harness, discover them, verify reads by observation, and invoke them through the
`aa` CLI over UDS gRPC. Every attempt lands in a hash-chained JSONL audit log.

**In scope**
- `cli_argv` binding and the supervision kernel.
- Seatbelt `Confiner` (macOS).
- Harvesters:
  - `cli-anything-registry@1`
  - `click-info@1` (Python one-shot shim)
  - `cobra-complete@1`
  - `help-text@1` (git parse-options + GNU style)
- CLI-Anything harness import (STATIC, from a local checkout, never `install_cmd`).
- Overlay parse, deterministic compile to `capabilities.lock.json`, and characterize rule tables.
- Lanes L0 (static), L1 (parser) and L3 (sandbox-diff, confined). Admission by effect level: **READ servable**. WRITE and DESTRUCTIVE stay
  withheld (`NOT_VERIFIED`) until L4 in slice 2.
- Gate: role check, resolve, admission, canonical schema, PathScope, identity re-check, policy evaluator (strictest wins,
  default deny), intent before dispatch (WRITE+ and INTROSPECT), dispatch, ActionRecord.
- JSONL hash-chained audit, intent recovery, boot-halted on chain failure, and the `Halt`/`Unhalt` kill switch.
- `aad` with **two** UDS listeners (`agent.sock`, `operator.sock`) and the generated `RPC_POLICY` table.
- `aa` commands: `daemon`, `status`, `halt`, `unhalt`, `target add`, `import cli-anything`, `discover`, `verify`, `list`, `describe`,
  `invoke` (alias `x`), `audit`.
- Integration tests on this Mac:
  - real `git`, always;
  - real `gh`, when present;
  - a hermetic Click fixture shaped like a CLI-Anything harness;
  - the real LibreOffice harness, opt-in.

**Out of scope for slice 1** (slice 2 or later):
- L2/L4 and WRITE admission;
- confirmations/Resume/grants;
- `aa mcp`;
- renderers (SKILL.md, MCP);
- `ApplyProposal`;
- `aa run`;
- SQLite index;
- TCP;
- adapters over gRPC.

These RPCs return `UNIMPLEMENTED`.

**Done when:**
1. `scripts/check.sh` is green: buf lint, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo nextest run --workspace`.
2. The acceptance demo (§4) runs on this Mac.
3. The cross-package e2e tests in `crates/aa-cli/tests/` pass against a real `aad` process.

## 2. Package map and ownership

WP0 lands first and is small. After WP0 merges, **every file has exactly one owner**, and no package edits another package's files.
Each package compiles against WP0's frozen trait and struct signatures (§3.0.4) and tests against fakes. Integration happens only in
WP6's e2e suite.

| WP | Crate(s) | Owns | Depends on (APIs only) | Size budget |
|---|---|---|---|---|
| **0** skeleton | workspace, `aa-proto`, `aa-core` foundation | root files, `crates/aa-proto/**`, `crates/aa-core/src/{lib.rs,ids.rs,digest.rs,error.rs,api/**}`, `crates/aa-core/tests/{zero_values.rs,dependency_direction.rs}`, `scripts/check.sh`, plus initial stubs for every other file listed below (ownership transfers on merge) | — | ~900 lines |
| **1** core semantics | `aa-core` | `crates/aa-core/src/{effects.rs,characterize/**,manifest/**,admission.rs,policy/**,lanes/**}`, `crates/aa-core/tests/{effects_props.rs,compile_golden.rs,policy_props.rs,l0.rs}`, `crates/aa-core/tests/fixtures/**` | WP0 | ~1.8k |
| **2** exec | `aa-exec` | `crates/aa-exec/**` | WP0 (`aa-core::api`) | ~1.5k |
| **3** harvest | `aa-harvest` | `crates/aa-harvest/**` (incl. `shims/aa_click_info.py`, `tests/fixtures/notes_cli/**`, recorded raw harvests) | WP0 | ~1.2k + 150 py |
| **4** store | `aa-store` | `crates/aa-store/**` | WP0 | ~0.7k |
| **5** engine | `aa-engine` | `crates/aa-engine/**` | WP0 (`aa-core::api` + signatures of WP1/2/3/4 from §3) | ~1.8k |
| **6** cli + e2e | `aa-cli`, `targets/` | `crates/aa-cli/**`, `targets/git/target.aa.yaml`, `targets/fixture-notes/target.aa.yaml`, `examples/policy.toml` | WP0 (`aa-proto[grpc]`) | ~1.0k |

Dependency direction, which WP0's test enforces:
- `aa-proto ← aa-core ← {aa-exec, aa-harvest, aa-store} ← aa-engine`.
- `aa-cli → aa-proto[grpc]`, plus `aa-engine` as a dev-dependency only.
- `aa-core` must not depend on tokio, tonic, hyper, rusqlite or process-wrap.

Parallelism:
- WP1 through WP6 start together once WP0 merges.
- WP5 builds its gate and services against the signatures in §3.1–§3.4, using fakes from `aa-core::api::testing`. The real crates are linked
  as soon as they land, because the signatures are already frozen.

---

## 3. Work packages

### 3.0 WP0: Workspace skeleton (must land first)

**Files owned (permanently):**
- `Cargo.toml`
- `rust-toolchain.toml`
- `rustfmt.toml`
- `.gitignore`
- `scripts/check.sh`
- `crates/aa-proto/{Cargo.toml,build.rs,src/lib.rs}`
- `crates/aa-core/Cargo.toml`
- `crates/aa-core/src/{lib.rs,ids.rs,digest.rs,error.rs}`
- `crates/aa-core/src/api/{mod.rs,exec.rs,binding.rs,harvest.rs,audit.rs,testing.rs}`
- `crates/aa-core/tests/{zero_values.rs,dependency_direction.rs}`

**Files created as stubs (ownership transfers on merge):** each other crate's `Cargo.toml` and `src/lib.rs`, plus the WP1 module files. Each stub
contains the exact public signatures from §3.1–§3.6 with `todo!()` bodies, so the workspace builds from day one.

#### 3.0.1 Root `Cargo.toml`

```toml
[workspace]
resolver = "3"
members = ["crates/aa-proto", "crates/aa-core", "crates/aa-exec", "crates/aa-harvest",
           "crates/aa-store", "crates/aa-engine", "crates/aa-cli"]

[workspace.package]
edition = "2024"
rust-version = "1.88"
license = "Apache-2.0"
version = "0.1.0"

[workspace.dependencies]
aa-proto = { path = "crates/aa-proto" }
aa-core = { path = "crates/aa-core" }
aa-exec = { path = "crates/aa-exec" }
aa-harvest = { path = "crates/aa-harvest" }
aa-store = { path = "crates/aa-store" }
aa-engine = { path = "crates/aa-engine" }
prost = "0.14.4"
prost-types = "0.14.4"
tonic = { version = "0.14.9", default-features = false, features = ["codegen", "transport", "router"] }
tonic-prost = "0.14.9"
tonic-prost-build = "0.14.9"
pbjson = "0.9.0"
pbjson-types = "0.9.0"
pbjson-build = "0.9.0"
prost-reflect = "0.16.5"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
serde_json_canonicalizer = "0.4.1"
serde_yaml_ng = "0.10.0"
toml = "1.2.0"
sha2 = "0.11.1"
hmac = "0.13.0"
blake3 = "1.8.7"
ulid = "3.0.0"
thiserror = "2.0.21"
anyhow = "1.0.104"
async-trait = "0.1.92"
jsonschema = "0.60.2"
regex = "1.13.1"
globset = "0.4.21"
tokio = { version = "1.54.1", features = ["rt-multi-thread", "macros", "net", "process", "time", "signal", "fs", "io-util", "sync"] }
tokio-stream = { version = "0.1.19", features = ["net"] }
tower = { version = "0.5.3", features = ["util"] }
http = "1.5.0"
process-wrap = { version = "10.0.1", features = ["tokio1"] }
rustix = { version = "1.1.6", features = ["process", "fs"] }
nix = { version = "0.31.3", features = ["signal", "process", "user"] }
tempfile = "3.27.0"
walkdir = "2.5.0"
which = "8.0.6"
fd-lock = "4.0.4"
clap = { version = "4.6.7", features = ["derive", "env"] }
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
proptest = "1.12.0"
insta = { version = "1.49.0", features = ["json"] }
assert_cmd = "2.2.2"

[workspace.lints.rust]
unsafe_code = "deny"
```

`rust-toolchain.toml`: `[toolchain] channel = "1.99.1"`. Exact `process-wrap` feature names must be checked against `cargo info
process-wrap@10.0.1` when WP0 lands. Pin whatever enables tokio process groups.

#### 3.0.2 `aa-proto` (verified recipe)

On 2026-11-05 this exact build compiled the repo's protos in a scratch crate and generated 8 servers and 41 policy rows. Without
`--features grpc`, `cargo tree` shows **zero** tonic crates.

```toml
# crates/aa-proto/Cargo.toml
[package]
name = "aa-proto"
edition.workspace = true
rust-version.workspace = true
version.workspace = true

[features]
grpc = ["dep:tonic", "dep:tonic-prost"]

[dependencies]
prost.workspace = true
prost-types.workspace = true
pbjson.workspace = true
pbjson-types.workspace = true
serde.workspace = true
tonic = { workspace = true, optional = true }
tonic-prost = { workspace = true, optional = true }

[build-dependencies]
tonic-prost-build.workspace = true
pbjson-build.workspace = true
prost-reflect.workspace = true
```

```rust
// crates/aa-proto/build.rs
use prost_reflect::{DescriptorPool, Value};
use std::{env, fmt::Write as _, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let out = PathBuf::from(env::var("OUT_DIR")?);
    let desc = out.join("agentic_anything_v1.bin");
    let mut files: Vec<PathBuf> = fs::read_dir(root.join("agentic_anything/v1"))?
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "proto"))
        .collect();
    files.sort();
    let grpc = env::var_os("CARGO_FEATURE_GRPC").is_some();
    tonic_prost_build::configure()
        .build_server(grpc)
        .build_client(grpc)
        .file_descriptor_set_path(&desc)
        .compile_well_known_types(true)
        .extern_path(".google.protobuf", "::pbjson_types")
        .compile_protos(&files, &[root.clone()])?;
    let bytes = fs::read(&desc)?;
    pbjson_build::Builder::new().register_descriptors(&bytes)?.build(&[".agentic_anything.v1"])?;

    // RPC_POLICY: every method MUST carry (agentic_anything.v1.rpc_policy) or the build fails.
    let pool = DescriptorPool::decode(bytes.as_slice())?;
    let ext = pool.get_extension_by_name("agentic_anything.v1.rpc_policy").expect("rpc_policy extension");
    let mut t = String::from("pub static RPC_POLICY: &[RpcPolicyRow] = &[\n");
    for svc in pool.services() {
        for m in svc.methods() {
            let o = m.options();
            assert!(o.has_extension(&ext), "method {} lacks rpc_policy", m.full_name());
            let v = o.get_extension(&ext);
            let msg = v.as_message().unwrap();
            let roles: Vec<i32> = msg.get_field_by_name("roles").unwrap().as_list().unwrap().iter()
                .map(|r| match r { Value::EnumNumber(n) => *n, _ => unreachable!() }).collect();
            let halted = msg.get_field_by_name("allowed_when_halted").unwrap().as_bool().unwrap();
            let audited = msg.get_field_by_name("audited").unwrap().as_bool().unwrap();
            writeln!(t, "  RpcPolicyRow {{ path: \"/{}/{}\", roles: &{:?}, allowed_when_halted: {}, audited: {} }},",
                     svc.full_name(), m.name(), roles, halted, audited)?;
        }
    }
    t.push_str("];\n");
    fs::write(out.join("rpc_policy.rs"), t)?;
    println!("cargo:rerun-if-changed={}", root.display());
    Ok(())
}
```

```rust
// crates/aa-proto/src/lib.rs  (public API, frozen)
pub mod v1 {
    #![allow(clippy::all, clippy::pedantic)]
    include!(concat!(env!("OUT_DIR"), "/agentic_anything.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/agentic_anything.v1.serde.rs"));
}
#[derive(Debug, Clone, Copy)]
pub struct RpcPolicyRow { pub path: &'static str, pub roles: &'static [i32], pub allowed_when_halted: bool, pub audited: bool }
include!(concat!(env!("OUT_DIR"), "/rpc_policy.rs"));
pub fn rpc_policy(path: &str) -> Option<&'static RpcPolicyRow> { RPC_POLICY.iter().find(|r| r.path == path) }
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agentic_anything_v1.bin"));
```

#### 3.0.3 `aa-core` foundation (WP0 implements fully)

`crates/aa-core/Cargo.toml` deps:
- `aa-proto` (no `grpc` feature)
- serde, serde_json, serde_json_canonicalizer, serde_yaml_ng, toml
- sha2, hmac, ulid, thiserror, async-trait, jsonschema, regex, globset

Dev-dependencies: proptest, insta.

```rust
// crates/aa-core/src/lib.rs
pub use aa_proto::v1 as pb;
pub mod ids; pub mod digest; pub mod error; pub mod api;              // WP0
pub mod effects; pub mod characterize; pub mod manifest;              // WP1
pub mod admission; pub mod policy; pub mod lanes;                     // WP1
pub use error::{AaError, AaResult};

// crates/aa-core/src/error.rs
#[derive(Debug, Clone, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AaError { pub code: pb::ErrorCode, pub message: String, pub remediation: String,
                     pub recovery: Option<pb::Recovery>, pub denial_reason: pb::DenialReason,
                     pub upstream: Option<pb::TargetText>, pub details_json: String }
pub type AaResult<T> = Result<T, AaError>;
impl AaError {
    pub fn new(code: pb::ErrorCode, message: impl Into<String>, remediation: impl Into<String>) -> Self;
    pub fn invalid(message: impl Into<String>) -> Self;          // INVALID_REQUEST, remediation "inspect the input schema: aa describe <id>"
    pub fn internal(message: impl Into<String>) -> Self;
    pub fn with_recovery(self, kind: pb::RecoveryKind, operation: impl Into<String>) -> Self;
    pub fn with_denial(self, reason: pb::DenialReason) -> Self;
    pub fn category(&self) -> pb::ErrorCategory;                  // total mapping code -> category
    pub fn retryable(&self) -> bool;
    pub fn to_proto(&self) -> pb::Error;
}
impl From<std::io::Error> for AaError { /* INTERNAL */ }
impl From<serde_json::Error> for AaError { /* INVALID_REQUEST */ }

// crates/aa-core/src/ids.rs
pub fn validate_target_id(id: &str) -> AaResult<()>;           // [a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*
pub fn capability_id(target_id: &str, native_path: &[&str]) -> String; // lowercases, '_'->'-', joins with '.'
pub fn mcp_name(prefix: &str, capability_id: &str, target_id: &str, taken: &std::collections::BTreeSet<String>) -> String; // <=64, 6-hex suffix
pub fn new_action_id() -> String;    // "act_" + ULID
pub fn new_session_id() -> String;   // "ses_" + ULID
pub fn new_confirmation_id() -> String; // "cnf_" + ULID

// crates/aa-core/src/digest.rs
pub fn jcs<T: serde::Serialize + ?Sized>(v: &T) -> AaResult<Vec<u8>>;     // RFC 8785 via serde_json_canonicalizer
pub fn sha256_hex(bytes: &[u8]) -> String;
pub fn digest<T: serde::Serialize + ?Sized>(v: &T) -> AaResult<String>;   // sha256_hex(jcs(v))
/// Digest of a message with one string field blanked first (lock_digest, entry_sha256, ...).
pub fn self_digest<T: serde::Serialize + Clone>(v: &T, blank: impl FnOnce(&mut T)) -> AaResult<String>;
pub fn args_commitment(key: &[u8; 32], input: &serde_json::Value) -> AaResult<String>; // "hmac-sha256:<hex>"
pub fn action_digest(input: &pb::ActionDigestInput) -> AaResult<String>;
```

```rust
// crates/aa-core/src/api/exec.rs   (frozen; implemented by aa-exec)
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
#[derive(Debug, Clone, Default)]
pub struct ConfinementSpec { pub deny_network: bool, pub writable: Vec<PathBuf>, pub exec_allow: Vec<PathBuf> }
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub argv: Vec<String>,                 // argv[0] must be an absolute realpath; never a shell
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,     // the COMPLETE child env (runner calls env_clear first)
    pub stdin: Option<Vec<u8>>,            // None => /dev/null
    pub deadline: Duration,
    pub max_stdout: u64,
    pub max_stderr_excerpt: usize,
    pub confinement: Option<ConfinementSpec>,
    pub run_id: String,                    // exported last as AA_RUN_ID
}
#[derive(Debug, Clone)]
pub struct RunOutput {
    pub exit_code: Option<i32>, pub signal: Option<i32>,
    pub stdout: Vec<u8>, pub stdout_total: u64, pub stdout_truncated: bool,
    pub stderr_excerpt: String, pub stderr_total: u64,
    pub wall: Duration, pub timed_out: bool,
    pub cleanup: pb::CleanupStatus, pub confinement_denials: Vec<String>,
    pub isolation: pb::Isolation,          // what was actually in force
}
/// Dropping the returned future MUST kill the owned process group (drop guard).
#[async_trait::async_trait]
pub trait ProcessRunner: Send + Sync {
    async fn run(&self, spec: RunSpec) -> crate::AaResult<RunOutput>;
    /// Best isolation this runner can provide on this host (CONFINED if a Confiner is available).
    fn max_isolation(&self) -> pb::Isolation;
}

// crates/aa-core/src/api/binding.rs  (frozen; cli_argv implemented by aa-exec)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind { CliArgv, Mcp, Http, Grpc, Adapter }
pub fn binding_kind(b: &pb::Binding) -> Option<BindingKind>;
#[derive(Debug, Clone)]
pub struct InvokeCtx {
    pub action_id: String,
    pub identity: pb::TargetIdentity,      // verified just before dispatch
    pub state_handle: Option<PathBuf>,     // engine-supplied (session workspace) or canonical scoped path
    pub scratch_parent: PathBuf,           // where the binding creates its private root
    pub deadline: std::time::Duration,
    pub dry_run: bool,
    pub confinement: Option<ConfinementSpec>,
}
#[derive(Debug, Clone)]
pub struct BindingOutput {
    pub outcome: pb::Outcome,
    pub output_json: Option<serde_json::Value>, // JSON_STDOUT parsed, TEXT_STDOUT => {"text","lines"}
    pub error: Option<crate::AaError>,          // UPSTREAM_ERROR / UNREADABLE_OUTPUT / TIMEOUT / ...
    pub process: pb::ProcessReport,
    pub observed_files: Vec<pb::ObservedFileEffect>,
}
#[async_trait::async_trait]
pub trait Binding: Send + Sync {
    fn kind(&self) -> BindingKind;
    /// Pure: the exact argv/request that WOULD be dispatched (used by tests, L1, and the audit record).
    fn plan(&self, cap: &pb::Capability, input: &serde_json::Value, ctx: &InvokeCtx) -> crate::AaResult<RunSpec>;
    async fn invoke(&self, cap: &pb::Capability, input: &serde_json::Value, ctx: &InvokeCtx) -> crate::AaResult<BindingOutput>;
}

// crates/aa-core/src/api/harvest.rs  (frozen; implemented by aa-harvest)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RawHarvest { pub harvester: String /* "click-info@1" */, pub name: String, pub media_type: String,
                        pub uri: String, pub authority: i32 /* pb::DiscoveryAuthority */, pub bytes: Vec<u8> }
impl RawHarvest { pub fn sha256(&self) -> String; }
#[derive(Debug, Clone)]
pub struct Candidate {
    pub native_path: Vec<String>,          // ["writer","add-paragraph"]
    pub capability: pb::Capability,        // id, title/description (TargetText), input_schema_json, binding, provenance; NO effects/admission
    pub native_signals: Vec<pb::EffectSignal>, // raise-only native hints (e.g. enum names), confidence <= DECLARED
    pub interactive: bool,                 // repl / bare group
}
pub struct HarvestCtx<'a> {
    pub target: &'a pb::Target,
    pub options: &'a serde_json::Value,    // overlay harvest[].options
    pub runner: &'a dyn ProcessRunner,
    pub scratch_parent: &'a std::path::Path,
    pub allow_network: bool,
}
#[async_trait::async_trait]
pub trait Harvester: Send + Sync {
    fn id(&self) -> &'static str;          // "click-info"
    fn version(&self) -> u32;              // 1
    fn authority(&self) -> pb::DiscoveryAuthority;
    fn applies(&self, target: &pb::Target, options: &serde_json::Value) -> bool;
    async fn harvest(&self, ctx: HarvestCtx<'_>) -> crate::AaResult<Vec<RawHarvest>>;   // effectful
    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> crate::AaResult<Vec<Candidate>>; // PURE
}

// crates/aa-core/src/api/audit.rs  (frozen; implemented by aa-store)
pub trait AuditSink: Send + Sync {
    /// Append + fsync. Err => caller MUST NOT dispatch (AUDIT_UNAVAILABLE).
    fn append_intent(&self, principal: &pb::Principal, halt_epoch: u64, intent: pb::ActionIntent) -> crate::AaResult<u64>;
    fn append_action(&self, principal: &pb::Principal, halt_epoch: u64, record: pb::ActionRecord) -> crate::AaResult<u64>;
    fn append_admin(&self, principal: &pb::Principal, halt_epoch: u64, event: pb::AdminEvent) -> crate::AaResult<u64>;
    fn head(&self) -> (u64, String);
    fn entry(&self, seq: u64) -> crate::AaResult<Option<pb::AuditEntry>>; // used by compile to verify `assumed[].audit_seq`
}

// crates/aa-core/src/api/testing.rs  (cfg(any(test, feature = "testing")))
pub struct FakeRunner { /* scripted RunOutput per argv prefix */ }
impl FakeRunner { pub fn new() -> Self; pub fn on(self, argv_prefix: &[&str], out: RunOutput) -> Self; pub fn calls(&self) -> Vec<RunSpec>; }
pub struct MemoryAudit { /* Vec<pb::AuditEntry> with a real hash chain */ }
impl MemoryAudit { pub fn new() -> Self; pub fn entries(&self) -> Vec<pb::AuditEntry>; pub fn fail_next_intent(&self); }
pub fn ok_output(stdout: &str) -> RunOutput;
```

**WP0 acceptance tests**
- `tests/zero_values.rs` checks that every safety enum's zero value is the most restrictive one: `Tri::Unknown`, `AdmissionState::Withheld`, `Verdict::Deny`, `Role::None`,
  `EffectLevel::Unknown`, `AuthorityTier::Unknown`, `ContentOrigin::Target`, `HaltScope::Engine`, `Isolation::NotProven`,
  `Confidence::Proposed`, `Outcome::Unknown`, `LaneStatus::NotRun`, `ApprovalSource::None`, `AssuranceTier::SamePrincipal`,
  `RequestState::UnknownId`. It also proptests that pbjson round-trips `CapabilityLock` and `AuditEntry` byte-identically after JCS.
- `aa-proto` unit test: `RPC_POLICY.len() == 41`. It checks that `DecideConfirmation`, `IssueGrant`, `Unhalt` and `AssumeEffects` list only `ROLE_OPERATOR` (2),
  and that every `AdapterService` method lists only `ROLE_ENGINE_INTERNAL` (3).
- `digest` checks against the RFC 8785 Appendix test vectors (at least the number-serialization and key-ordering vectors), embedded as fixtures.
- `tests/dependency_direction.rs` runs `cargo metadata --format-version 1`. It fails if `aa-core` (normal deps, transitively) contains
  `tokio`, `tonic`, `hyper`, `rusqlite` or `process-wrap`, or if any crate depends on a crate to its right in the rule (§2).

**Commands**

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
buf lint && buf format -d --exit-code proto
cargo build --workspace && cargo test -p aa-proto -p aa-core
cargo tree -p aa-core -e normal | grep -c tonic    # must print 0
```

`scripts/check.sh` runs: `buf lint`, `buf format -d --exit-code proto`, `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace` (or `cargo test --workspace` if nextest is missing).

---

### 3.1 WP1: `aa-core` semantics (pure)

**Files owned:**
- `src/effects.rs`
- `src/characterize/{mod.rs,rules.rs,click.rs,cobra.rs,help_text.rs}`
- `src/manifest/{mod.rs,overlay.rs,compile.rs,lock.rs}`
- `src/admission.rs`
- `src/policy/{mod.rs,paths.rs,exclusions.rs}`
- `src/lanes/{mod.rs,static_lane.rs}`
- `tests/{effects_props.rs,compile_golden.rs,policy_props.rs,l0.rs}`
- `tests/fixtures/**`

**Public API (exact):**

```rust
// effects.rs
pub const TARGET_FIELDS: &[&str];                       // the 16 target-facing Effects field names
pub fn get(e: &pb::Effects, field: &str) -> Option<pb::Tri>;
pub fn set(e: &mut pb::Effects, field: &str, v: pb::Tri) -> crate::AaResult<()>;
/// Lowering table (ADR 0011). None => lanes can never lower this field.
pub fn min_isolation_to_lower(field: &str, has_write_path_param: bool) -> Option<pb::Isolation>;
/// Whether a FALSE signal may count (source/confidence/isolation rules, architecture §6.2).
pub fn admissible(sig: &pb::EffectSignal, has_write_path_param: bool) -> bool;
/// Max-combine over admissible signals; idempotent combines by AND (UNKNOWN => FALSE).
pub fn combine(signals: &[pb::EffectSignal], has_write_path_param: bool) -> pb::Effects;
pub fn apply_branches(base: &pb::Effects, branches: &[pb::EffectBranch], input: &serde_json::Value) -> pb::Effects;
pub fn level(e: &pb::Effects) -> pb::EffectLevel;
pub fn tier(e: &pb::Effects, binding: &pb::Binding) -> pb::AuthorityTier;
pub fn hints(e: &pb::Effects) -> pb::McpHints;
/// Evaluation-time conservatism: UNKNOWN tier => highest, UNKNOWN level => never servable.
pub fn tier_rank(t: pb::AuthorityTier) -> u8;

// characterize/mod.rs
pub fn rule_versions() -> std::collections::BTreeMap<String, String>;   // {"cli-verbs":"1","cli-options":"1","cli-anything":"1","exclusions":"1","compile":"1"}
pub struct Characterized { pub signals: Vec<pb::EffectSignal>, pub branches: Vec<pb::EffectBranch>,
                           pub scopes: Vec<pb::ScopeParam>, pub withheld: Vec<pb::Withheld> }
pub fn characterize(target: &pb::Target, cand: &crate::api::Candidate, overlay: &crate::manifest::Overlay) -> Characterized;

// manifest/overlay.rs  (serde types mirroring architecture §7.2; unknown keys rejected)
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)] #[serde(deny_unknown_fields)]
pub struct Overlay { pub schema: String, pub target: String, pub display_name: Option<String>, pub mcp_prefix: Option<String>,
                     pub locator: Option<Locator>, pub authorization: Option<String>, #[serde(default)] pub harvest: Vec<HarvestSpec>,
                     #[serde(default)] pub defaults: Defaults, pub state_handle: Option<StateHandle>,
                     #[serde(default)] pub exclude: Vec<Exclude>, #[serde(default)] pub fixtures: Vec<FixtureSpec>,
                     #[serde(default)] pub capabilities: std::collections::BTreeMap<String, CapabilityOverlay>,
                     #[serde(default)] pub assumed: Vec<Assumed> }
// Locator, HarvestSpec{harvester, commands, help_argv, options}, Defaults{binding: BindingDefaults},
// BindingDefaults{global_args, env, deadline_ms, max_output_bytes, usage_exit_code, usage_stderr_marker},
// StateHandle{flag, param, engine_supplied}, Exclude{capability, reason},
// FixtureSpec{id, steps: Vec<FixtureStepSpec>, ready}, FixtureStepSpec{argv|file|capability|include_fixture},
// CapabilityOverlay{binding: Option<BindingOverride>, fixed_args, cwd_pointer, effects: BTreeMap<String,bool>,
//                   branches, scopes, oracle, examples, core, use_when},
// Assumed{field, value: bool, glob, reason, audit_seq}
pub fn parse_overlay(yaml: &str) -> crate::AaResult<Overlay>;
pub fn starter_overlay(target: &pb::Target, harvesters: &[String]) -> String;   // YAML text

// manifest/compile.rs
pub struct CompileInput<'a> {
    pub target: pb::Target,                          // identity already pinned by the engine
    pub overlay: &'a Overlay,
    pub overlay_sha256: String,
    pub sources: Vec<pb::SourceRef>,
    pub candidates: Vec<crate::api::Candidate>,      // from Harvester::candidates over CAS bytes
    pub verification: &'a [pb::VerificationRecord],  // existing records; admission only counts matching digests
    pub audit: &'a dyn crate::api::AuditSink,        // to verify assumed[].audit_seq (ADMIN_EVENT_KIND_EFFECT_ASSUMED)
}
pub struct CompileOutput { pub lock: pb::CapabilityLock, pub added: Vec<String>, pub removed: Vec<String>, pub changed: Vec<String> }
/// PURE and deterministic: same inputs => byte-identical canonical lock.
pub fn compile(input: CompileInput<'_>, previous: Option<&pb::CapabilityLock>) -> crate::AaResult<CompileOutput>;
pub fn capability_digest(cap: &pb::Capability) -> crate::AaResult<String>;

// manifest/lock.rs
pub fn lock_to_json(lock: &pb::CapabilityLock) -> crate::AaResult<Vec<u8>>;     // canonical JCS bytes + trailing \n
pub fn lock_from_json(bytes: &[u8]) -> crate::AaResult<pb::CapabilityLock>;    // validates schema=="aa.lock/v1" and lock_digest
pub fn finalize_digest(lock: &mut pb::CapabilityLock) -> crate::AaResult<()>;

// admission.rs
pub fn required_lanes(level: pb::EffectLevel, cap: &pb::Capability) -> Vec<pb::Lane>;
pub fn compute(cap: &pb::Capability, records: &[pb::VerificationRecord], lock_digest: &str, identity_digest: &str) -> pb::Admission;

// policy/mod.rs
#[derive(Debug, Clone)] pub struct Policy { /* parsed aa-policy/1 */ }
pub fn parse_policy(toml_text: &str) -> crate::AaResult<Policy>;
pub fn policy_digest(toml_text: &str) -> String;
#[derive(Debug, Clone)]
pub struct ActionDescriptor<'a> {
    pub target_id: &'a str, pub capability_id: &'a str, pub level: pb::EffectLevel, pub tier: pb::AuthorityTier,
    pub effects: &'a pb::Effects, pub read_paths: &'a [std::path::PathBuf], pub write_paths: &'a [std::path::PathBuf],
    pub target_registered: bool, pub session_tainted: bool, pub assurance: pb::AssuranceTier,
}
/// Hard exclusions first; then ALL matching rules; strictest wins regardless of order; no match => DENY.
pub fn evaluate(policy: &Policy, d: &ActionDescriptor<'_>) -> pb::PolicyDecision;
/// One remediation command per denial reason (a test asserts each command exists in aa-cli's command list).
pub fn remediation_for(reason: pb::DenialReason, d: &ActionDescriptor<'_>) -> String;

// policy/paths.rs
pub struct PathScope { pub write_roots: Vec<std::path::PathBuf>, pub read_roots: Vec<std::path::PathBuf>, pub deny: Vec<std::path::PathBuf> }
/// realpath (write: realpath of parent + file name); rejects `..` after canonicalization, symlink escapes.
pub fn canonicalize_for_scope(p: &std::path::Path, write: bool) -> crate::AaResult<std::path::PathBuf>;
/// Deny wins; returns the denial reason on failure.
pub fn check(scope: &PathScope, canonical: &std::path::Path, write: bool) -> Result<(), pb::DenialReason>;

// policy/exclusions.rs
pub fn excluded(target: &pb::Target, capability_id: &str, text: &str) -> Option<&'static str>;  // category name

// lanes/static_lane.rs
pub fn run_l0(lock: &pb::CapabilityLock, cap: &pb::Capability, engine_version: &str) -> pb::VerificationRecord;
```

**Rules to implement:**
- architecture §6.2–§6.4: lattice, lowering table, branches, projections and rule tables `cli-verbs@1`, `cli-options@1`,
  `cli-anything@1` (autosave, repl, `{error,type}`), and `exclusions@1`;
- `compile` behaviour:
  - capability ids come from native paths;
  - overlay `exclude` sets `EXCLUDED`;
  - `assumed[]` entries count only if `audit.entry(seq)` is an `EFFECT_ASSUMED` admin event with a matching field and value;
  - JSON Schemas are closed;
  - MCP names are unique;
  - `capability_digest` is computed;
  - admission is reset when the digest changes vs `previous`;
- `TEXT_STDOUT` output schema = `{text, lines}`.
- **Mode-split capabilities:** an overlay may declare a new capability id (for example `git.branch.delete`) whose
  `binding.command_path` equals a harvested native path, plus `fixed_args`. The native candidate it splits (`git.branch`) is then withheld
  `MIXED_AUTHORITY` unless it is excluded. L0 fails if `command_path` matches no harvested native path.
- **`cwd_pointer`:** if the input schema has no property at that pointer, compile adds a required `{type: string, format: path}` property
  (git's `repo`).

**Acceptance tests**
1. `effects_props.rs` (proptest):
   - combine is commutative, associative and idempotent;
   - adding a signal never lowers any field's rank;
   - PROPOSED and INFERRED FALSE never produce FALSE;
   - an OBSERVED FALSE at `ISOLATION_PROCESS` never lowers `accesses_network`, `launches_process` or out-of-root `writes_filesystem`;
   - `has_write_path_param` blocks lowering `writes_filesystem`;
   - level, tier and hints are monotone;
   - any UNKNOWN target field gives `EFFECT_LEVEL_UNKNOWN`.
2. `policy_props.rs`:
   - the decision is invariant under rule shuffles;
   - deny beats ask beats allow;
   - an empty policy gives DENY with `NO_MATCHING_ALLOW`;
   - exclusions beat any allow;
   - every `DenialReason` has a non-empty remediation.
3. `compile_golden.rs`: from the recorded fixtures in `tests/fixtures/` (WP1 commits copies: `click-info.notes.json`, `help-text.git.json`,
   `cobra.gh-alias.json`, plus their overlays) produces locks snapshotted with `insta`. Compiling twice is byte-identical. Changing one help line changes exactly
   one `capability_digest` and resets only that capability's admission.
4. `l0.rs` checks that L0 fails on:
   - an open schema;
   - a duplicate MCP name;
   - an oracle that references a non-servable read;
   - an unresolved `MIXED_AUTHORITY` branch;
   - a fixture include cycle;
   - a UNKNOWN field with no hint.

**Commands:** `cargo test -p aa-core`, `cargo test -p aa-core --test effects_props -- --nocapture`.

---

### 3.2 WP2: `aa-exec` (supervision, confinement, `cli_argv`, L1/L3)

**Files owned:** `crates/aa-exec/**`: `src/{lib.rs,supervisor.rs,root.rs,confine.rs,identity.rs,cli_argv.rs,snapshot.rs,lanes/{mod.rs,parser.rs,sandbox_diff.rs,fixture.rs}}`,
`tests/**`, `tests/bin/torture.rs` (a test helper binary).

Deps: aa-core, tokio, process-wrap, rustix, nix, tempfile, walkdir, blake3, which, async-trait, serde_json, tracing.

**Public API (exact):**

```rust
// supervisor.rs
pub struct SupervisorConfig { pub confiner: std::sync::Arc<dyn Confiner>, pub term_grace: std::time::Duration /* 250ms */ }
pub struct Supervisor { /* … */ }
impl Supervisor { pub fn new(cfg: SupervisorConfig) -> Self; }
impl aa_core::api::ProcessRunner for Supervisor { /* own process group, env_clear + spec.env, AA_RUN_ID last,
   stdin /dev/null, deadline -> SIGTERM(group) -> grace -> SIGKILL(group), bounded stdout (drain overflow),
   head+tail stderr excerpt with token redaction, survivor check => CleanupStatus, Drop guard kills group */ }

// root.rs
pub struct PrivateRoot { pub base: std::path::PathBuf, pub home: std::path::PathBuf, pub cwd: std::path::PathBuf, pub tmp: std::path::PathBuf }
impl PrivateRoot {
    pub fn create(parent: &std::path::Path) -> aa_core::AaResult<Self>;          // 0700, realpath'd, owner-checked
    pub fn base_env(&self, path: &str) -> std::collections::BTreeMap<String, String>; // PATH, HOME, TMPDIR, LANG=C.UTF-8
}
impl Drop for PrivateRoot { /* best-effort remove; failures logged */ }

// snapshot.rs
pub struct TreeSnapshot(pub std::collections::BTreeMap<std::path::PathBuf, String>); // rel path -> blake3
pub fn snapshot(root: &std::path::Path, ignore: &[&str]) -> aa_core::AaResult<TreeSnapshot>;
pub fn diff(before: &TreeSnapshot, after: &TreeSnapshot, root_label: &str) -> Vec<aa_core::pb::ObservedFileEffect>;

// confine.rs
pub trait Confiner: Send + Sync {
    fn available(&self) -> bool;
    /// Returns the wrapped argv (e.g. ["/usr/bin/sandbox-exec","-f",<profile>,…argv]) and writes the profile into `scratch`.
    fn wrap(&self, argv: Vec<String>, spec: &aa_core::api::ConfinementSpec, scratch: &std::path::Path) -> aa_core::AaResult<Vec<String>>;
}
pub struct SeatbeltConfiner;   // macOS; available() = /usr/bin/sandbox-exec exists AND a self-test denies a write
pub struct NoConfiner;
pub fn seatbelt_profile(spec: &aa_core::api::ConfinementSpec) -> String;          // architecture §9 profile; paths escaped
pub fn default_confiner() -> std::sync::Arc<dyn Confiner>;

// identity.rs
pub fn file_identity(role: &str, path: &std::path::Path, version_argv: Option<&[&str]>) -> aa_core::AaResult<aa_core::pb::FileIdentity>;
pub fn identity_digest(id: &aa_core::pb::TargetIdentity) -> aa_core::AaResult<String>;
pub struct IdentityCache { /* (dev, ino, size, mtime) -> sha256 */ }
impl IdentityCache { pub fn new() -> Self; pub fn verify(&self, id: &aa_core::pb::TargetIdentity) -> aa_core::AaResult<()>; } // TARGET_IDENTITY_CHANGED + REDISTILL

// cli_argv.rs
/// PURE. argv = [exe] + interpreter_args + global_args + [state_handle_flag=path] + dry_run.argv + command_path
///   + fixed_args + options("--flag=value", "--flag", repeated) + ["--"] + positionals.
/// Refuses: unknown input keys (schema already closed), values starting with '-' when no_equals_form, NUL bytes.
pub fn build_argv(cap: &aa_core::pb::Capability, input: &serde_json::Value, ctx: &aa_core::api::InvokeCtx) -> aa_core::AaResult<Vec<String>>;
pub fn normalize_output(binding: &aa_core::pb::CliArgvBinding, out: &aa_core::api::RunOutput) -> aa_core::api::BindingOutput;
pub struct CliArgvBinding { runner: std::sync::Arc<dyn aa_core::api::ProcessRunner> }
impl CliArgvBinding { pub fn new(runner: std::sync::Arc<dyn aa_core::api::ProcessRunner>) -> Self; }
impl aa_core::api::Binding for CliArgvBinding { /* plan() = build_argv + RunSpec; invoke() runs in a fresh PrivateRoot
   (cwd = cwd_pointer path if set, else root.cwd), HOME/TMPDIR private, confinement from ctx */ }

// lanes/mod.rs
pub struct LaneCtx<'a> {
    pub lock: &'a aa_core::pb::CapabilityLock,
    pub binding: &'a dyn aa_core::api::Binding,
    pub runner: &'a dyn aa_core::api::ProcessRunner,
    pub scratch_parent: &'a std::path::Path,
    pub engine_version: &'a str,
}
pub async fn run_parser_lane(ctx: &LaneCtx<'_>, cap: &aa_core::pb::Capability) -> aa_core::pb::VerificationRecord;      // L1
pub async fn run_sandbox_diff_lane(ctx: &LaneCtx<'_>, cap: &aa_core::pb::Capability) -> aa_core::pb::VerificationRecord; // L3
pub async fn build_fixture(ctx: &LaneCtx<'_>, fixture_id: &str, root: &PrivateRoot) -> aa_core::AaResult<()>;
```

**L3 semantics:**
- For each example (at least 2, else `SKIPPED` with reason "needs ≥2 examples"):
  1. Fresh root and fixture.
  2. Snapshot `home/`+`cwd/` and the state handle.
  3. Run under confinement if available: `writable = [root]`, `exec_allow = identity files`, `deny_network`.
  4. Snapshot again and diff.
- Emit OBSERVED signals:
  - `mutates_target`/`writes_filesystem` FALSE if every diff is empty, at the run's isolation;
  - `accesses_network`/`launches_process` FALSE only if the run was CONFINED and succeeded;
  - TRUE for each observed change;
  - finding `SIDE_EFFECT` if observed ⊄ declared.
- `non_claims` always includes "cloud/remote state not observed" and, at `ISOLATION_PROCESS`, "network and out-of-root writes not observed".

**L1 semantics:** for each scalar param, a wrong-typed value; plus `--aa-probe-unknown`. Expect `usage_exit_code` (and
`usage_stderr_marker` if set) and no file change. Accepting the probe → `PARSER_MISMATCH`. If `--opt=value` is rejected but `--opt value` is accepted,
the record carries `no_equals_form` for that arg.

**Acceptance tests (`cargo test -p aa-exec`):**
1. **Torture**, via the `torture` helper binary built from `tests/bin/torture.rs`:
   - ignores SIGTERM → killed within deadline + grace, `timed_out`, cleanup COMPLETE;
   - forks a grandchild that sleeps → reaped (same group), no survivor;
   - floods 100 MiB stdout with `max_stdout = 1 MiB` → `stdout_truncated`, memory bounded;
   - dropping the `run()` future mid-flight kills the group within 1 s.
2. **Confinement** (macOS, skipped with reason when `sandbox-exec` is absent; a skip never counts):
   - a child writing outside the root → write denied, the file is absent, stderr reports EPERM, and `confinement_denials` is non-empty;
   - `curl` to a non-loopback address under `deny_network` fails;
   - an exec of `/bin/sh` that is not on `exec_allow` is denied (sandbox-exec exit 71);
   - `git --no-optional-locks status` in a root repo works (all as verified 2026-11-05).
3. **`build_argv` table tests:**
   - a value `-rf` for an option → `--text=-rf`;
   - positionals after `--`;
   - a repeated option;
   - a negatable flag;
   - the state handle before the command path;
   - `dry_run` argv placed before the command path (CLI-Anything requires it);
   - NUL rejected.
4. **`normalize_output`:**
   - CLI-Anything `{"error":"Index 99 out of range (0-0)","type":"IndexError"}` with exit 1 → `UPSTREAM_ERROR` and upstream `TargetText`;
   - non-JSON stdout for `JSON_STDOUT` → `UNREADABLE_OUTPUT`;
   - text → `{text, lines}`.
5. **L3 on real git** (`tests/l3_git.rs`):
   - plain `git status` after an mtime bump → finding `SIDE_EFFECT`, with `.git/index` MODIFIED;
   - with `--no-optional-locks` → no diff, and FALSE signals at CONFINED.
6. **IdentityCache:** a rewritten fake binary → `TARGET_IDENTITY_CHANGED`.

---

### 3.3 WP3: `aa-harvest` (harvesters + CLI-Anything import + Click fixture)

**Files owned:**
- `crates/aa-harvest/**`: `src/{lib.rs,cli_anything.rs,click_info.rs,cobra.rs,help_text.rs,schema.rs}`;
- `shims/aa_click_info.py`;
- `tests/fixtures/notes_cli/{pyproject.toml,notes_cli/__init__.py,notes_cli/cli.py}`;
- `tests/fixtures/raw/{click-info.notes.json,click-info.libreoffice.json,cobra.gh.jsonl,help-text.git.json,registry.min.json}`;
- `tests/**`.

**Public API (exact):**

```rust
pub fn builtin_harvesters() -> Vec<std::sync::Arc<dyn aa_core::api::Harvester>>;   // the four below
pub struct ClickInfoHarvester;    // id "click-info", v1, INTROSPECT
pub struct CobraCompleteHarvester;// id "cobra-complete", v1, INTROSPECT
pub struct HelpTextHarvester;     // id "help-text", v1, INTROSPECT
pub struct CliAnythingRegistryHarvester; // id "cli-anything-registry", v1, STATIC
pub const CLICK_INFO_SHIM: &str = include_str!("../shims/aa_click_info.py");

pub mod cli_anything {
    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct RegistryEntry { pub name: String, pub display_name: String, pub version: String, pub description: String,
        pub requires: Option<String>, pub homepage: Option<String>, pub source_url: Option<String>,
        pub install_cmd: Option<String>, pub entry_point: String, pub skill_md: Option<String>, pub category: Option<String> }
    pub fn read_registry(repo: &std::path::Path) -> aa_core::AaResult<Vec<RegistryEntry>>;
    pub struct ImportedHarness {
        pub target: aa_core::pb::Target,          // id "cli-anything.<name>", kind CLI_ANYTHING_HARNESS, identity files
                                                  // {interpreter, entry_module, backend:<requires>?}, source_tree_sha, license
        pub overlay_yaml: String,                 // starter: state_handle --project (engine_supplied), exclude repl, harvest [click-info]
        pub raw: Vec<aa_core::api::RawHarvest>,   // registry entry JSON, install_cmd (verbatim, never run), SKILL.md (untrusted)
        pub install_cmd_flags: Vec<String>,       // e.g. "shell-metachar", "unpinned-git-url"
    }
    /// STATIC only: reads files; hashes the harness tree (git tree sha if .git present, else sorted blake3 manifest).
    pub fn import_harness(repo: &std::path::Path, name: &str, python: &std::path::Path, authorization: &str) -> aa_core::AaResult<ImportedHarness>;
}

pub mod schema {
    /// Click param list (to_info_dict) -> closed JSON Schema + ArgMappings; global --json/--project/--dry-run handled per §11.
    pub fn click_params_to_schema(params: &serde_json::Value, globals: &CliAnythingGlobals) -> aa_core::AaResult<(serde_json::Value, Vec<aa_core::pb::ArgMapping>)>;
    #[derive(Debug, Clone, Default)] pub struct CliAnythingGlobals { pub json_flag: Option<String>, pub project_flag: Option<String>, pub dry_run_flag: Option<String> }
}

pub mod help_text {
    #[derive(Debug, Clone, PartialEq)] pub struct OptionSpec { pub long: String, pub short: Option<char>, pub value: Option<String>, pub negatable: bool, pub help: String }
    pub fn parse_git_parse_options(text: &str) -> Vec<OptionSpec>;   // "-s, --[no-]short    show status concisely", "--untracked-files[=<mode>]"
    pub fn parse_gnu(text: &str) -> Vec<OptionSpec>;
}
```

**Shim** `aa_click_info.py`, protocol `aa-click-info/1`:
- It uses the stdlib plus `click` only.
- It resolves the entry point by name via `importlib.metadata`. It loads it; if the object is not a `click.Command`, it takes the **unique `click.Group` in the
  entry module that is not a child of another group**, and otherwise emits `{"error":"ambiguous_root"}` with exit 65.
- It prints `{"protocol","entry_point","distribution":{name,version},"click_version","root": root.to_info_dict(ctx)}`.
- It never invokes the group.
- It is run as `<python> -I <root>/aa_click_info.py <entry_point>` with confinement (network denied, writes only in the root), a 20 s deadline
  and a 4 MiB cap.

**Cobra:**
- Crawl recursively with `X __complete <path…> ""`, which lists subcommands with `\t` descriptions and a trailing `:<directive>` line, and
  `X __complete <path…> --` for flags.
- Respect `max_depth` 4 and `max_calls` 600.
- Record each call's argv and output in one JSONL raw artifact.
- `candidates()` turns leaves into capabilities. Flags with descriptions become options. Flags named in `cli-options@1` become branches (WP1 rules). The
  usage-error marker is `"unknown flag"` with exit code 1 (gh, observed).

**help-text:**
- Run `X <cmd> -h` per overlay `commands`. git prints parse-options usage and exits 129, which is the expected exit for this harvester.
- `--[no-]x` becomes a negatable flag. `--opt[=<v>]` and `--opt <v>` become string options. `<pathspec>...` in the usage line becomes a positional array.

**Click fixture** `notes_cli`: about 120 lines, shaped like a CLI-Anything harness:
- `@click.group(invoke_without_command=True)` with `--json/--project/--dry-run`;
- a `main()` wrapper;
- `{"error","type"}` errors;
- leaves `notes list`, `notes add --text`, `notes remove <index>`, `export --output <path>`, `repl`;
- **`notes stats`**, which claims to be a read but writes `~/.cache/notes/stats`, so L3 must flag `SIDE_EFFECT`;
- **`notes lenient`**, which accepts unknown options (`ignore_unknown_options`), so L1 must flag `PARSER_MISMATCH`.

**Acceptance tests (`cargo test -p aa-harvest`):**
1. Pure replay with no Python, git or gh:
   - `candidates()` over `click-info.notes.json` → 7 leaves (with `repl` marked interactive) → `insta` snapshot;
   - over `cobra.gh.jsonl` → includes `gh.alias.list`/`gh.alias.set`;
   - over `help-text.git.json` → `git.status` has `untracked_files` (enum `no|normal|all`) and `short` (negatable).
2. `click_params_to_schema`: Choice→enum, IntRange→bounds, `multiple`→array, flag→boolean, `nargs=-1`→positional array; `--json` never exposed;
   `--project` mapped to the state handle.
3. `import_harness` against `tests/fixtures/registry.min.json` plus a temp harness tree:
   - `install_cmd` is stored and never executed (a `FakeRunner` records zero calls);
   - `requires: libreoffice` → precondition `binary:soffice`;
   - `target.id == "cli-anything.libreoffice"`.
4. Live, gated on `AA_TEST_PYTHON` (an interpreter with click). The CI job sets it; locally a missing value prints a skip reason and never counts as pass:
   - real `harvest()` of `notes_cli` through `aa-core::api::testing::FakeRunner` → the bytes are re-recorded to a temp file and must equal the
     committed fixture modulo `click_version`.
5. Live LibreOffice, opt-in via `AA_TEST_CLI_ANYTHING=/Users/nhanclie/CLI-Anything` and `AA_TEST_LO_PYTHON=<venv>/bin/python`:
   - `import_harness(repo, "libreoffice", …)` → 40 leaf candidates (generator-first observation);
   - root selection succeeds despite `main()` (`libreoffice_cli.py:831`).

---

### 3.4 WP4: `aa-store` (workspace, CAS, JSONL audit, halt state)

**Files owned:** `crates/aa-store/**`: `src/{lib.rs,workspace.rs,cas.rs,audit.rs,halt.rs}` and `tests/**`.
Deps: aa-core, fd-lock, serde_json, tempfile (dev).

**Public API (exact):**

```rust
// workspace.rs
pub struct Workspace { /* root */ }
impl Workspace {
    pub fn open(root: &std::path::Path) -> aa_core::AaResult<Self>;       // creates layout 0700; refuses foreign owner/loose mode
    pub fn root(&self) -> &std::path::Path;
    pub fn target_dir(&self, target_id: &str) -> std::path::PathBuf;
    pub fn list_targets(&self) -> aa_core::AaResult<Vec<String>>;
    pub fn read_overlay(&self, target_id: &str) -> aa_core::AaResult<Option<String>>;
    pub fn write_overlay(&self, target_id: &str, yaml: &str) -> aa_core::AaResult<String>;          // atomic; returns sha256
    pub fn read_target(&self, target_id: &str) -> aa_core::AaResult<Option<aa_core::pb::Target>>;   // target.json (pinned identity)
    pub fn write_target(&self, target: &aa_core::pb::Target) -> aa_core::AaResult<()>;
    pub fn read_lock(&self, target_id: &str) -> aa_core::AaResult<Option<aa_core::pb::CapabilityLock>>;
    pub fn write_lock(&self, lock: &aa_core::pb::CapabilityLock) -> aa_core::AaResult<String>;      // canonical bytes via aa_core::manifest::lock_to_json
    pub fn write_harvest_index(&self, target_id: &str, sources: &[aa_core::pb::SourceRef]) -> aa_core::AaResult<()>;
    pub fn read_harvest_index(&self, target_id: &str) -> aa_core::AaResult<Vec<aa_core::pb::SourceRef>>;
    pub fn write_verification(&self, rec: &aa_core::pb::VerificationRecord) -> aa_core::AaResult<()>;
    pub fn read_verifications(&self, target_id: &str) -> aa_core::AaResult<Vec<aa_core::pb::VerificationRecord>>;
    pub fn cas(&self) -> &Cas;
    pub fn sessions_dir(&self) -> std::path::PathBuf;
    pub fn scratch_dir(&self) -> std::path::PathBuf;                       // parent for private roots
    pub fn hmac_key(&self) -> aa_core::AaResult<[u8; 32]>;                 // state/hmac.key, created 0600 on first use
}
// cas.rs
pub struct Cas { /* root */ }
impl Cas { pub fn put(&self, bytes: &[u8]) -> aa_core::AaResult<String>; pub fn get(&self, sha256: &str) -> aa_core::AaResult<Vec<u8>>;
           pub fn contains(&self, sha256: &str) -> bool; }
// audit.rs
pub struct AuditLog { /* single writer behind a Mutex; segment rollover at 64 MiB */ }
#[derive(Debug, Clone)] pub struct BootReport { pub chain_ok: bool, pub entries: u64, pub first_broken_seq: Option<u64>,
                                                pub head_matches: bool, pub recovered_action_ids: Vec<String> }
#[derive(Debug, Clone, Default)] pub struct AuditQuery { pub target_id: Option<String>, pub capability_id: Option<String>,
                                                         pub action_id: Option<String>, pub after_seq: u64, pub limit: u32, pub actions_only: bool }
impl AuditLog {
    /// Verifies the chain + HEAD, then closes dangling intents with recovered ActionRecords (OUTCOME_UNKNOWN,
    /// EXECUTION_STATE_UNKNOWN, recovered=true, principal ENGINE_INTERNAL). Never truncates or rewrites.
    pub fn open(dir: &std::path::Path) -> aa_core::AaResult<(Self, BootReport)>;
    pub fn query(&self, q: &AuditQuery) -> aa_core::AaResult<Vec<aa_core::pb::AuditEntry>>;
    pub fn verify_chain(&self) -> BootReport;
    pub fn find_action(&self, action_id: &str) -> aa_core::AaResult<Option<aa_core::pb::ActionRecord>>;
    pub fn find_by_idempotency_key(&self, key: &str) -> aa_core::AaResult<Option<aa_core::pb::ActionRecord>>;
}
impl aa_core::api::AuditSink for AuditLog { /* append_* = assign seq, prev, entry_sha256 = sha256(JCS(entry with "")),
   write line, fsync (always for intent/admin; action: fsync if level >= WRITE), atomic-rename HEAD */ }
// halt.rs
pub struct HaltStore { /* state/halt.json + state/HALT sentinel */ }
impl HaltStore { pub fn open(dir: &std::path::Path) -> aa_core::AaResult<Self>; pub fn load(&self) -> aa_core::pb::HaltState;
                 pub fn save(&self, s: &aa_core::pb::HaltState) -> aa_core::AaResult<()>; pub fn sentinel_present(&self) -> bool; }
```

**Acceptance tests (`cargo test -p aa-store`):**
1. Append 1,000 mixed entries, reopen → `chain_ok`.
2. **Tamper:** flip one byte in a middle line → `first_broken_seq` equals that line. Delete the last line → `head_matches == false`.
3. **Crash recovery:** append an intent with no action, reopen → exactly one recovered ActionRecord with `OUTCOME_UNKNOWN` and `recovered`. Reopening again
   adds nothing (idempotent).
4. Entries round-trip as proto3-JSON lines that validate as `AuditEntry`.
5. `write_lock` writes canonical bytes, so `read_lock` and then `write_lock` are byte-identical.
6. `Workspace::open` refuses a dir with mode 0777 or a symlinked root.
7. `hmac_key` is created with mode 0600.

---

### 3.5 WP5: `aa-engine` (`aad`: listeners, roles, gate, services)

**Files owned:**
- `crates/aa-engine/**`: `src/{lib.rs,config.rs,listen.rs,role_layer.rs,gate.rs,sessions.rs,pipeline.rs,status.rs}`;
- `src/services/{engine.rs,target.rs,distill.rs,catalog.rs,runtime.rs,policy.rs,audit.rs}`;
- `src/bin/aad.rs`;
- `tests/**`.

Deps: aa-core, aa-exec, aa-harvest, aa-store, aa-proto[grpc], tonic, tonic-prost, tokio, tokio-stream, tower, http, fd-lock, clap,
tracing, tracing-subscriber.

**Public API (exact):**

```rust
// config.rs
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub workspace: std::path::PathBuf,         // AA_HOME
    pub runtime_dir: std::path::PathBuf,       // contains agent.sock, operator.sock, engine.lock
    pub policy_path: std::path::PathBuf,       // missing => built-in default (deny all but observe of registered targets)
    pub agent_uids: Vec<u32>,                  // default [geteuid()]
    pub engine_version: String,
}
impl EngineConfig { pub fn from_env_and_args(args: &AadArgs) -> aa_core::AaResult<Self>; } // AA_HOME, AA_RUNTIME_DIR, --policy
#[derive(Debug, clap::Parser)] pub struct AadArgs { /* --workspace --runtime-dir --policy --log */ }

// lib.rs
pub struct EngineDeps {
    pub runner: std::sync::Arc<dyn aa_core::api::ProcessRunner>,
    pub harvesters: Vec<std::sync::Arc<dyn aa_core::api::Harvester>>,
    pub bindings: Vec<std::sync::Arc<dyn aa_core::api::Binding>>,
}
impl EngineDeps { pub fn production() -> Self; }   // Supervisor(default_confiner), builtin_harvesters, CliArgvBinding
pub struct Engine { /* workspace, audit, halt, policy, sessions, identity cache, deps */ }
impl Engine {
    /// Opens the workspace + audit (BootReport: chain failure => boots halted with HALT_SOURCE_AUDIT_VERIFY_FAILED),
    /// loads halt state and policy, writes ENGINE_START.
    pub fn build(cfg: EngineConfig, deps: EngineDeps) -> aa_core::AaResult<std::sync::Arc<Self>>;
    /// Runtime-dir checks, flock(engine.lock), unlink stale sockets, bind both 0600, serve until `shutdown`.
    pub async fn serve_uds(self: std::sync::Arc<Self>, shutdown: impl std::future::Future<Output = ()> + Send + 'static) -> aa_core::AaResult<()>;
    /// For tests: in-process duplex transport with an explicit role (no sockets).
    pub async fn connect_in_process(self: &std::sync::Arc<Self>, role: aa_core::pb::Role) -> aa_core::AaResult<tonic::transport::Channel>;
}

// role_layer.rs
/// Per-listener tower layer: looks up req.uri().path() in aa_proto::RPC_POLICY (unknown path => PERMISSION_DENIED),
/// checks role ∈ roles and halt state (allowed_when_halted), checks peer uid from UdsConnectInfo, and inserts
/// `aa_core::pb::Principal` into request extensions. Errors carry an encoded aa Error in Status details.
#[derive(Clone)] pub struct RoleLayer { pub role: aa_core::pb::Role, /* halt handle, allowed uids */ }
impl<S> tower::Layer<S> for RoleLayer { type Service = RoleService<S>; fn layer(&self, inner: S) -> Self::Service; }
pub struct RoleService<S> { /* … */ }

// gate.rs  (the single enforcement point; architecture §5.3 steps 2-13; slice 1 has no "ask" path:
//           ask => PendingConfirmation is NOT minted; Error CONFIRMATION_REQUIRED, recovery AWAIT_CONFIRMATION "aa approve (M3)")
pub struct Gate { /* Arc<Engine> internals */ }
pub enum GateEvent { Progress(String), Result(aa_core::pb::InvokeResult), Error(aa_core::pb::Error) }
impl Gate {
    pub async fn invoke(&self, principal: &aa_core::pb::Principal, req: aa_core::pb::InvokeRequest,
                        events: tokio::sync::mpsc::Sender<GateEvent>) -> aa_core::pb::ActionRecord;   // exactly one record written
    /// Pipeline actions (aa.pipeline.harvest.<id>, aa.pipeline.verify.<lane>) through the same audit discipline.
    pub async fn pipeline<T>(&self, principal: &aa_core::pb::Principal, capability_id: &str, target: &aa_core::pb::Target,
                             authority: aa_core::pb::DiscoveryAuthority,
                             f: impl std::future::Future<Output = aa_core::AaResult<T>> + Send) -> aa_core::AaResult<(T, String /*action_id*/)>;
}
```

**Services implemented in slice 1.** Every other RPC returns `UNIMPLEMENTED`.
- `EngineService`: `GetStatus`, `Halt`, `Unhalt`, `Shutdown`.
- `TargetService`: `ListTargets`, `GetTarget`, `CreateTarget`, `ImportCliAnything`. `CreateTarget`:
  - requires a non-empty authorization statement;
  - pins identity via `aa_exec::identity`;
  - writes `target.json` and the starter overlay (or keeps an existing overlay);
  - writes a `TARGET_REGISTERED` admin event.
- `DistillService`:
  - `Harvest` runs each applicable harvester through `Gate::pipeline`. INTROSPECT is refused without an authorization statement, writes intent first,
    and stores raw bytes in the CAS and in `harvest/index.json`.
  - `Compile`: reads the CAS → `candidates()` → `aa_core::manifest::compile` → `write_lock`.
  - `Verify` runs L0 (`aa_core::lanes`), then L1 and L3 (`aa_exec::lanes`). It writes records and recompiles admission.
- `CatalogService`: `ListCapabilities`, `DescribeCapability`, `SearchCapabilities` (substring search over id, title and description).
- `RuntimeService`: `OpenSession` (0700 workspace under `sessions/`), `CloseSession`, `Invoke`, `Cancel`, `GetActionStatus`.
- `AuditService`: `QueryAudit`, `VerifyChain`.

**Acceptance tests (`cargo test -p aa-engine`, all in-process over duplex or a temp-dir UDS, using `FakeRunner` and `MemoryAudit` where noted):**
1. **Role split:**
   - every RPC whose `RPC_POLICY` row is operator-only (`Unhalt`, `Shutdown`, `AssumeEffects`, `DecideConfirmation`, `IssueGrant`) returns
     `PERMISSION_DENIED` on the agent role, generated by iterating the table;
   - `Halt` works on both roles;
   - `Unhalt` with a stale epoch is refused.
2. **Peer uid:** a connection whose `UdsConnectInfo` uid is not allowed → `PERMISSION_DENIED` (unit-test the layer with a synthetic extension).
3. **Gate order and records:**
   - unknown capability → `NOT_FOUND` with **no** record;
   - schema-invalid input → record `NOT_DISPATCHED` + `INVALID_REQUEST`;
   - withheld → `CAPABILITY_WITHHELD` record;
   - policy deny → record with `deciding_rule_id`;
   - READ allowed → dispatch → `COMPLETED`.

   Each case has exactly one record (asserted on `MemoryAudit`).
4. **Intent discipline:**
   - a WRITE capability (servable in a synthetic lock) with `MemoryAudit::fail_next_intent()` → `AUDIT_UNAVAILABLE`, and `FakeRunner.calls()` is empty;
   - an INTROSPECT harvest writes an intent before the runner is called.
5. **Boot halted:** a workspace whose audit segment is tampered → `GetStatus.halt.halted == true` with source `AUDIT_VERIFY_FAILED`. Invoke →
   `UNAVAILABLE`.
6. **Identity drift:** change the pinned file between compile and invoke → `TARGET_IDENTITY_CHANGED` record, nothing dispatched.
7. **PathScope:** `repo` outside `read_roots` → `OUTSIDE_PATH_SCOPE`. A symlink escaping a write root → denied.
8. **Kill switch:** `Halt` while a long FakeRunner call is in flight → the call future is dropped (the runner observes the drop). The record is
   `CANCELLED` or `UNKNOWN`, and a new Invoke → `UNAVAILABLE`.

**Commands:**
- `cargo test -p aa-engine`
- `cargo run -p aa-engine --bin aad -- --workspace /tmp/aa-ws --runtime-dir /tmp/aa-rt`

---

### 3.6 WP6: `aa-cli` + end-to-end integration

**Files owned:**
- `crates/aa-cli/**`: `src/{lib.rs,main.rs,connect.rs,exit.rs,output.rs,commands/*.rs}` and `tests/{exit_codes.rs,e2e_git.rs,e2e_click.rs,e2e_gh.rs,e2e_libreoffice.rs,common/mod.rs}`;
- `targets/git/target.aa.yaml` (architecture §7.3);
- `targets/fixture-notes/target.aa.yaml`;
- `examples/policy.toml`.

Deps: aa-proto[grpc], tonic, tokio, clap, serde_json, anyhow. Dev-dependencies: aa-engine, assert_cmd, tempfile.

**Public API (exact):**

```rust
// connect.rs
pub struct ClientConfig { pub runtime_dir: std::path::PathBuf }      // AA_RUNTIME_DIR or platform default
impl ClientConfig { pub fn from_env() -> Self; pub fn agent_socket(&self) -> std::path::PathBuf; pub fn operator_socket(&self) -> std::path::PathBuf; }
/// tonic native unix endpoint: Endpoint::from_shared(format!("unix://{}", path))?.connect().await
pub async fn connect(cfg: &ClientConfig, operator: bool) -> Result<tonic::transport::Channel, CliError>;
// exit.rs
#[derive(Debug)] pub enum CliError { Unreachable(String), Status(tonic::Status), Domain(aa_proto::v1::Error), Usage(String) }
/// 0 ok | 1 failed | 2 usage/schema | 3 needs human/authority | 4 engine unreachable or halted | 130/143 signal | 141 EPIPE
pub fn exit_code(err: &CliError) -> i32;
pub fn exit_code_for_error(err: &aa_proto::v1::Error) -> i32;     // the single classifier
// lib.rs
pub async fn run(argv: Vec<std::ffi::OsString>) -> i32;
```

**Commands** (clap derive):

| Command | RPC |
|---|---|
| `aa daemon [--foreground]` | execs sibling `aad` |
| `aa status` | `GetStatus` |
| `aa halt [--reason]` | `Halt` |
| `aa unhalt --epoch N` | `Unhalt`, on **operator.sock** |
| `aa target add <id> --exec <path> \| --python-script <path> [--harvester h]… [--overlay file] --authorization "…"` | `CreateTarget` |
| `aa import cli-anything <repo> --harness <name> --python <path> --authorization "…"` | `ImportCliAnything` |
| `aa discover <target>` | `Harvest`, then `Compile`; prints counts: candidates, unknown bits, withheld by reason |
| `aa verify <target> [--require-tools t]…` | `Verify` |
| `aa list [<target>] [--servable]` | `ListCapabilities` |
| `aa describe <cap>` | `DescribeCapability` |
| `aa invoke <cap> --input-json <inline\|@file\|-> [--session s] [--dry-run]` (alias `x`) | `Invoke` |
| `aa audit [--last N] [--action id] [--verify]` | `QueryAudit` / `VerifyChain` |

Global flags: `--json` (compact) and `--full-output` (envelope `{ok, data|error, meta:{action_id}}`). stdin is capped at 8 MiB.

**Acceptance tests:**
1. `exit_codes.rs` (unit + `assert_cmd`):
   - `exit_code_for_error` covers every `ErrorCode`, and the match is exhaustive: no `_` arm, plus a test over all enum values;
   - with no engine running, `aa status` → 4;
   - a usage error → 2.
2. **`e2e_git.rs`** (real git, always on macOS; this is the slice's headline test). It:
   - spawns `aad` as a real process on a temp runtime dir and workspace, with `examples/policy.toml` and `read_roots = [<tmp>]`;
   - creates a fixture repo in the temp dir;
   - runs `aa target add git --exec git --overlay targets/git/target.aa.yaml --authorization "test"` → 0;
   - runs `aa discover git` → candidates include `git.status`, `git.log`, `git.add`, and the overlay mode-splits `git.branch.list`/`git.branch.delete`.
     The native `git.branch` is withheld as `MIXED_AUTHORITY`;
   - runs `aa verify git` → `git.status`, `git.log` and `git.branch.list` become `SERVABLE` (READ, `requiredRuntimeIsolation = CONFINED` when Seatbelt is available);
     `git.add` is `WITHHELD NOT_VERIFIED`; `git.branch.delete` is `WITHHELD`;
   - runs `aa invoke git.status --input-json '{"repo":"<tmp>/repo"}'` → 0, with `.data.lines` containing `# branch.head`;
   - runs `aa invoke git.add …` → 3, `CAPABILITY_WITHHELD`, nothing dispatched (the repo index hash is unchanged);
   - runs `aa invoke git.status --input-json '{"repo":"/etc"}'` → 3, `OUTSIDE_PATH_SCOPE`;
   - runs `aa invoke git.status --input-json '{"repo":1}'` → 2;
   - runs `aa audit --last 10 --verify` → chain intact; one ActionRecord per invoke attempt (4); harvest and verify pipeline actions present with intents;
   - asserts the `git_status_side_effect_caught` variant: an overlay **without** `--no-optional-locks`, plus a fixture that bumps an mtime → after
     `aa verify`, `git.status` is WITHHELD with a `SIDE_EFFECT` finding naming `.git/index`.
3. **`e2e_click.rs`** (needs `AA_TEST_PYTHON`, set in CI; locally prints a skip reason):
   - `aa target add fixture-notes --python-script <venv>/bin/notes-cli …` → discover → verify;
   - `notes.list` SERVABLE;
   - `notes.stats` WITHHELD `SIDE_EFFECT`;
   - `notes.lenient` WITHHELD `PARSER_MISMATCH`;
   - `repl` INTERACTIVE;
   - `notes.add` WITHHELD `NOT_VERIFIED` (WRITE; slice 2);
   - invoking `notes.list` uses an engine-supplied `--project` in the session workspace.
4. **`e2e_gh.rs`** (runs when `gh` is on PATH, else prints a skip reason):
   - `aa target add gh --exec gh --harvester cobra-complete` with overlay env `GH_CONFIG_DIR` pointing into the private root;
   - `aa discover gh` → `gh.alias.list` present;
   - `aa verify gh --only gh.alias.list` → SERVABLE under confinement (no network).
5. **`e2e_libreoffice.rs`** (opt-in: `AA_TEST_CLI_ANYTHING`, `AA_TEST_LO_PYTHON`):
   - `aa import cli-anything $AA_TEST_CLI_ANYTHING --harness libreoffice --python $AA_TEST_LO_PYTHON --authorization …`;
   - `aa discover cli-anything.libreoffice` → 40 candidates;
   - `aa verify` → `writer.list` and `document.info` SERVABLE;
   - the audit shows the `install_cmd` artifact stored and **no** process with `pip` in its argv.
6. **Operator split from the CLI:** `aa unhalt --epoch N` while `AA_RUNTIME_DIR`'s `operator.sock` is made unreadable (chmod 000 in the test) → exit 4. The
   same request over the agent socket (hidden test-only flag `--via-agent-socket`) → exit 3 `ACCESS_DENIED`.

**Commands:**
- `cargo test -p aa-cli`
- `cargo test -p aa-cli --test e2e_git -- --nocapture`
- `AA_TEST_PYTHON=$PWD/.venv/bin/python cargo test -p aa-cli --test e2e_click`

---

## 4. Acceptance demo (run on this Mac after integration)

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cargo build --release
export AA_HOME=$(mktemp -d) AA_RUNTIME_DIR=$(mktemp -d)/rt PATH="$PWD/target/release:$PATH"
cp examples/policy.toml $AA_HOME/policy.toml
aad --policy $AA_HOME/policy.toml & sleep 1
aa status                                   # caller_role AGENT, halted=false, confinement_available=true, assurance SAME_PRINCIPAL

R=$(mktemp -d)/repo && git init -q $R && echo hi > $R/README && git -C $R add README \
  && git -C $R -c user.name=me -c user.email=me@localhost commit -qm init
aa target add git --exec git --overlay targets/git/target.aa.yaml --authorization "owner workstation"
aa discover git                             # 5 native + 2 overlay mode-split candidates; unknown bits listed per capability
aa verify git                               # L0/L1/L3; git.status, git.log, git.branch.list -> SERVABLE (READ, CONFINED)
aa list git                                 # git.add WITHHELD NOT_VERIFIED; git.branch.delete WITHHELD; git.rev-parse MIXED_AUTHORITY
aa x git.status --input-json "{\"repo\":\"$R\"}"          # exit 0; porcelain v2 lines
aa x git.add --input-json "{\"repo\":\"$R\",\"pathspec\":[\"README\"]}"; echo $?   # 3: withheld, nothing ran
aa halt --reason demo && aa x git.status --input-json "{\"repo\":\"$R\"}"; echo $?  # 4: halted
aa unhalt --epoch $(aa status --json | jq .halt.haltEpoch)
aa audit --last 12 --verify                 # intents for pipeline actions; one ActionRecord per invoke; chain intact

# CLI-Anything (operator pre-installed the harness venv; the engine never installs):
#   uv venv ~/.local/share/aa-venvs/lo && uv pip install --python ~/.local/share/aa-venvs/lo/bin/python \
#     /Users/nhanclie/CLI-Anything/libreoffice/agent-harness
aa import cli-anything /Users/nhanclie/CLI-Anything --harness libreoffice \
   --python ~/.local/share/aa-venvs/lo/bin/python --authorization "owner workstation; MPL-2.0 app; Apache-2.0 harness"
aa discover cli-anything.libreoffice && aa verify cli-anything.libreoffice
aa list cli-anything.libreoffice --servable  # document.info, writer.list, calc.list-sheets, ... (READ)
```

## 5. Slice 2 preview (so slice-1 APIs do not paint us into a corner)

- **L2 dry-run and L4 read-back/negative lanes** are added in `aa-exec::lanes`. Their `admission::required_lanes` rows already exist in WP1.
  WRITE becomes servable: `git.add` (via `git.status` CONTAINS), `notes.add`, `cli-anything.libreoffice.writer.add-paragraph`.
- **`aa-core::render`:** mcp-typed, mcp-search, skill-md, cli-tree, and the catalog digest. `CatalogService.GetExposure` and `DistillService.Package`.
- **`aa mcp`** as the `mcp` feature of `aa-cli` (rmcp 3.8.0 stdio). `aa run` builds a dynamic clap tree.
- **`aa-core::proposal`:** the ManifestProposal fence, with the hostile-proposal table and a blocking cargo-mutants floor. `GetProposalContext` and `ApplyProposal`.
- **`compile --check` / `package --check`** run in CI.
