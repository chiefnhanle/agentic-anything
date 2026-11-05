# Proposal: MVP-first (the smallest end-to-end loop)

Status: independent architecture proposal, one of three (see `PLAN.md`, "Design phase"). Lens: **get a working loop
with the fewest crates and lines, and leave clean seams for everything else.** Date: 2026-11-05.

The loop this proposal optimizes for:

```
point at a CLI binary or a CLI-Anything harness
  -> discover (harvest the self-description, record it verbatim)
  -> characterize (effects; conservative max-of-signals; missing = unknown)
  -> synthesize (capability manifest IR, content-addressed)
  -> verify (deterministic lanes; anything not READ stays disabled)
  -> invoke over gRPC (policy gate -> owned one-shot child -> ActionRecord in a hash-chained audit log)
  -> `aa` CLI on top
```

Scope rule, carried into the API: targets are software the owner owns or is authorized to automate. Every target
registration records an authority basis. DRM/licence defeat, protected-asset extraction and security-control evasion
are out of scope, and the API has no capability kind that would serve them (`survey-safety-rails-and-authorization-models.md` §10).

---

## 1. Thesis

1. **The capability manifest is the product. Everything else renders it or enforces it.** The manifest is protobuf
   (`aa.v1.Manifest`), and it is stored on disk as its proto3-JSON form. That gives one schema language for both the
   wire API and the IR, one codegen step, and `buf breaking` as a free compatibility gate on the IR. Payload schemas
   (capability input/output) stay JSON Schema strings inside the proto. They are JSON-native, and proto `Struct` would
   lose the int/float distinction.
2. **Four crates, two binaries.** `aa-proto` (generated code), `aa-core` (pipeline, policy, runner, audit; no
   transport), `aa-engine` (binary `aad`, the tonic server), `aa-cli` (binary `aa`, a gRPC client). The MCP server is
   an `aa mcp` subcommand that is also a gRPC client (milestone M2). Skills are rendered by the engine from the same IR.
   Nothing else is a crate until it is needed.
3. **One binding kind on day one: `cli_argv`.** It covers CLI-Anything harnesses, and through the same Click
   introspection it covers *any* Click-based CLI. Other surfaces (generic `--help`/Cobra CLIs, HTTP/OpenAPI,
   MCP-proxy, plugin bridges, a11y) go into a reserved `oneof` slot and an out-of-process adapter protocol drafted
   here but not built.
4. **Fail closed, by construction rather than by review.** Proto3's zero value for every effect is `TRI_UNKNOWN`, so
   a field that is missing *on the wire* means unknown. Unknown counts as hazardous. The only way a capability becomes
   callable in the MVP is an attributable operator review that lowers it to READ. WRITE, DESTRUCTIVE and UNKNOWN
   capabilities are emitted but disabled, with typed reasons, until the read-back-oracle lane exists (M3).
5. **Every invocation attempt after capability resolution yields exactly one ActionRecord**, including denials and
   malformed input ("denial launched nothing" is a recorded fact; REA `docs/reconstruction-readiness.md:67` via
   `rea-contracts-and-evidence-model.md` §7). Records are content-addressed and hash-chained in SQLite, so the audit
   trail exists from the first commit. `PLAN.md` puts this in M3; this proposal pulls it into the first slice.
6. **No LLM in the engine for the MVP.** Discovery, characterization and synthesis are deterministic. Descriptions
   come from the target's own help text, labelled as untrusted target content. LLM proposals come later, as an
   external client that may only *raise* effects (`survey-capability-discovery-and-distillation.md` §5).

---

## 2. Cargo workspace layout

```
agentic-anything/
  Cargo.toml                    # [workspace], edition 2024, resolver 3, shared [workspace.dependencies]
  rust-toolchain.toml           # channel = "1.99.1"
  buf.yaml / buf.gen.yaml       # lint + breaking only; codegen stays in build.rs
  proto/aa/v1/aa.proto          # the whole MVP API (§3)
  proto/aa/adapter/v1/adapter.proto   # DRAFT, not compiled until M4 (§5.3)
  crates/
    aa-proto/                   # build.rs: tonic-prost-build + pbjson-build; re-exports aa::v1
    aa-core/                    # pipeline + policy + runner + audit; no tonic server, no clap
      src/ir.rs                 # helpers over generated types: digests, effect algebra, annotations
      src/discover/{mod,click_introspect,cli_anything,manual}.rs
      src/characterize.rs       # signal table as data, max-combine, review overlay
      src/synthesize.rs         # candidates -> Manifest (naming, closed JSON Schemas, bindings)
      src/verify.rs             # L0 static, L1 address lanes
      src/policy.rs             # admit_target, admit_invocation
      src/runner.rs             # owned one-shot child: argv, env scrub, deadline, bounds, killpg on drop
      src/audit.rs              # SQLite store, JCS digests, hash chain
      src/render.rs             # M2: SKILL.md, MCP tool projections
      assets/click_introspect.py
    aa-engine/                  # bin `aad`: UDS listener, peer-uid interceptor, service impls (thin)
    aa-cli/                     # bin `aa`: clap; `aa mcp` behind feature "mcp" (M2)
  fixtures/click-notes/         # tiny CLI-Anything-convention harness + registry.json (test target)
  docs/design/proposals/mvp-first.md
```

### 2.1 Crates, responsibilities, dependencies

Versions below were checked against crates.io on 2026-11-05. Pin them in `[workspace.dependencies]`.

| Crate | Responsibility | Key dependencies |
|---|---|---|
| `aa-proto` | Compile `proto/aa/v1/aa.proto`. Emit prost messages, tonic client and server stubs, and pbjson serde impls (proto3 JSON) so the IR can be written to disk and digested. No logic. | `prost 0.14.4`, `prost-types 0.14.4`, `tonic 0.14.9`, `tonic-prost 0.14.9`, `pbjson 0.9.0`, `pbjson-types 0.9.0`, `serde 1.0.229`; build: `tonic-prost-build 0.14.9`, `pbjson-build 0.9.0` (pbjson 0.9 depends on prost ^0.14, verified) |
| `aa-core` | All behaviour: discovery sources, characterization, synthesis, verification lanes, policy gate, process runner, audit store, renderers. Async, but transport-free, so it is testable without gRPC and reusable by offline commands (`aa manifest lint FILE`). | `aa-proto`, `tokio 1.54.1` (process, time, fs, rt), `serde_json 1.0.154`, `serde_json_canonicalizer 0.4.1` (RFC 8785 JCS for digests), `sha2 0.11.1`, `hex 0.4.3`, `jsonschema 0.60.2` (validate args against the closed schema), `rusqlite 0.41.1` with feature `bundled` (no system SQLite), `nix 0.31.3` (killpg, geteuid, flock), `uuid 1.29.0` (v7 run ids), `tempfile 3.27.0`, `thiserror 2.0.21`, `tracing 0.1.44` |
| `aa-engine` (`aad`) | Bind the UDS, enforce peer uid, map RPCs onto `aa-core`, and map `aa-core` errors to `tonic::Status` with an `aa.v1.Error` detail. Target: under 600 lines. | `aa-core`, `aa-proto`, `tonic 0.14.9`, `tokio-stream 0.1.19` (`UnixListenerStream`), `directories 6.0.0`, `tracing-subscriber 0.3.23`, `anyhow 1.0.104` |
| `aa-cli` (`aa`) | clap front-end, a pure gRPC client. Resolves relative paths client-side, then calls the engine. Owns exit codes and output formatting. `aa mcp` (M2) is a module here behind cargo feature `mcp`. | `aa-proto`, `tonic 0.14.9` (client; `unix:///abs/path` URIs are supported natively since tonic 0.14.5, verified in `transport/channel/endpoint.rs`), `clap 4.6.7` (derive), `serde_json`, `anyhow`; M2: `rmcp 3.8.0` (features `server`, `transport-io`) |
| dev-only | Workspace-level integration and e2e tests | `assert_cmd 2.2.2`, `insta 1.49.0` (json), `tempfile` |

Deliberately absent: `schemars` (schemas come from the target, not from Rust types), `tonic-reflection`/`tonic-health`
(about 10 lines each, added in M1 when grpcurl debugging is worth it), `hyper-util` (not needed now that the client
supports `unix:` URIs), YAML/TOML parsers (the manifest and overlays are proto3 JSON).

### 2.2 Line budget for the first slice

| Unit | Budget (lines) |
|---|---|
| `aa.proto` | ~380 |
| `aa-core` (discover 350, characterize 200, synthesize 300, verify 120, policy 120, runner 250, audit 250, ir 150) | ~1,750 |
| `aa-engine` | ~500 |
| `aa-cli` | ~500 |
| `click_introspect.py` + `fixtures/click-notes` | ~180 |
| tests | ~700 |
| **Total** | **~4,000** |

### 2.3 Alternatives rejected

- **Three crates (fold `aa-core` into `aa-engine`).** This saves one `Cargo.toml`, but it costs the transport-free
  test seam and the offline commands, and it would make the MCP front-end want to link engine internals.
- **One binary (`aa engine serve`).** This is plausible for distribution, and the seam is the same. It is deferred
  only because two binaries keep "front-ends are clients" honest during the first month. Revisit at packaging time.
- **Separate serde domain types plus proto conversions.** That is the "clean" layering REA uses (zod domain →
  contracts → adapters; `rea-capability-registry-and-composition.md` §1). It roughly doubles the IR code. The MVP
  accepts coupling to generated types and uses `buf breaking` as the governance mechanism instead. Fallback if pbjson
  integration fails on day one: hand-written serde mirrors for `Manifest` only.

---

## 3. The gRPC API (draft `proto/aa/v1/aa.proto`)

Design rules:
- **Proto envelopes, JSON payloads.** Capability input/output schemas and invocation args/results are JSON text
  fields. Their canonical schema is JSON Schema 2020-12 with `additionalProperties: false` at the root ("a misspelled
  parameter never degrades into an omitted one", `rea-desktop-applications-and-agent-environments.md` §8.2).
- **Zero value = unknown or deny.** Every enum's 0 is `UNKNOWN`/`UNSPECIFIED`, and the engine treats it
  conservatively. The draft passes `protoc` and `buf lint` (STANDARD) with one deliberate exception,
  `ENUM_ZERO_VALUE_SUFFIX`, because for `Tri` and `EffectLevel` the zero value *means* unknown rather than
  "not set" (checked 2026-11-05 with protoc 36.1 / buf 1.75.0).
- **Target-originated text is typed.** Any string that came from the target is a `TargetText`, never a bare
  `string`. Front-ends must propagate the untrusted marker. This is REA's `returnsTargetContent`
  (`src/contracts/toolEffects.ts:788-816`) moved into the type system.
- **Transport errors vs recorded outcomes.** Errors before capability resolution are `tonic::Status` with an
  `aa.v1.Error` in `grpc-status-details-bin`. After resolution, the outcome is in-band in an `ActionRecord` (§3.2).

```proto
syntax = "proto3";
package aa.v1;

import "google/protobuf/timestamp.proto";

// ════════════════════════════ shared vocabulary ════════════════════════════

// Tri-state. The proto3 default (0) is UNKNOWN, so "missing = unknown" holds on the wire.
enum Tri { TRI_UNKNOWN = 0; TRI_NO = 1; TRI_YES = 2; }

// Coarse projection for policy/UX (usage-spec semantics: absence = ask; flags only raise; no inheritance).
enum EffectLevel {
  EFFECT_LEVEL_UNKNOWN = 0;
  EFFECT_LEVEL_READ = 1;
  EFFECT_LEVEL_WRITE = 2;
  EFFECT_LEVEL_DESTRUCTIVE = 3;
}

// Ordered tiers. A caller's grant on a target is a ceiling; a capability declares what it needs.
enum AuthorityTier {
  AUTHORITY_TIER_UNSPECIFIED = 0;   // deny
  AUTHORITY_TIER_OBSERVE = 1;       // run read-only operations of the registered target
  AUTHORITY_TIER_LAUNCH = 2;        // reserved (M6): start long-lived instances / sessions
  AUTHORITY_TIER_MUTATE = 3;        // change target-owned state (≈ "drive")
  AUTHORITY_TIER_ATTACH = 4;        // reserved
  AUTHORITY_TIER_INJECT = 5;        // reserved; never for protected third-party components
  AUTHORITY_TIER_MUTATE_SYSTEM = 6; // reserved
}

// Authority used *during discovery* (survey-capability-discovery-and-distillation.md §1.3).
enum DiscoveryAuthority {
  DISCOVERY_AUTHORITY_UNSPECIFIED = 0;
  DISCOVERY_AUTHORITY_STATIC = 1;     // read files only
  DISCOVERY_AUTHORITY_INTROSPECT = 2; // run the target's self-description (may execute target code)
  DISCOVERY_AUTHORITY_PROBE = 3;      // invoke with test args (sandbox lane; M3)
  DISCOVERY_AUTHORITY_EXPLORE = 4;    // drive UI / write state (sandbox only; M6)
}

// Text that originated from the target (help strings, stdout, error messages). Data, never instructions.
message TargetText { string text = 1; string origin = 2; /* e.g. "click.help", "stdout" */ }

message Principal {
  uint32 uid = 1;              // from SO_PEERCRED / getpeereid, set by the engine, not the client
  int32 pid = 2;
  string client = 3;           // declared, e.g. "aa-cli/0.1.0", "aa-mcp/0.1.0"
  PrincipalKind declared_kind = 4;
}
enum PrincipalKind { PRINCIPAL_KIND_UNSPECIFIED = 0; PRINCIPAL_KIND_HUMAN = 1; PRINCIPAL_KIND_AGENT = 2; }

// ════════════════════════════ effects ════════════════════════════

enum SignalSource {
  SIGNAL_SOURCE_UNSPECIFIED = 0;
  SIGNAL_SOURCE_TARGET_DECLARED = 1;   // e.g. Click Path(writable=True), usage `effect`, sdef access
  SIGNAL_SOURCE_CONVENTION = 2;        // e.g. CLI-Anything --project auto-save
  SIGNAL_SOURCE_KEYWORD = 3;           // name/flag keywords: may only RAISE
  SIGNAL_SOURCE_OPERATOR = 4;          // attributable human review
  SIGNAL_SOURCE_OBSERVED = 5;          // sandbox before/after diff (M3); an observed YES can never be overridden
  SIGNAL_SOURCE_PROPOSAL = 6;          // non-binding hint (keyword "read" prior now, LLM later); never applied
}

message EffectSignal {
  string effect = 1;        // field name in Effects, e.g. "writes_filesystem"; "*" = all hazards
  Tri value = 2;
  SignalSource source = 3;
  string detail = 4;        // e.g. "option --overwrite", "command verb 'remove'"
}

// REA's ToolEffects vocabulary (src/contracts/toolEffects.ts:2-22), tri-state, with mutatesSession removed:
// audit recording is not a target effect (rea-tool-design-philosophy-and-verification.md §1.6: 109/234 REA tools
// lost readOnlyHint solely because of it).
message Effects {
  Tri mutates_target = 1;
  Tri writes_filesystem = 2;
  Tri accesses_network = 3;
  Tri launches_process = 4;        // beyond the bound executable itself (e.g. harness spawns soffice)
  Tri changes_ui_state = 5;
  Tri may_discard_data = 6;
  Tri executes_foreign_code = 7;   // runs code beyond the registered target (scripts, plugins, eval)
  Tri mutates_system = 8;          // generalizes REA mutatesDevice
  Tri idempotent = 9;              // UNKNOWN is treated as NO
  EffectLevel level = 10;          // derived; stored for convenience, recomputed on load
  repeated EffectSignal signals = 11;
}

// Derived, never hand-set (REA annotationsFromEffects, toolEffects.ts:765-787), with UNKNOWN treated as YES
// for hazards and as NO for idempotent.
message Annotations {
  bool read_only_hint = 1;
  bool destructive_hint = 2;
  bool idempotent_hint = 3;
  bool open_world_hint = 4;
}

// ════════════════════════════ targets ════════════════════════════

enum TargetKind {
  TARGET_KIND_UNSPECIFIED = 0;
  TARGET_KIND_CLI_EXECUTABLE = 1;
  TARGET_KIND_CLI_ANYTHING_HARNESS = 2;
}

enum IdentityStrength { IDENTITY_STRENGTH_UNSPECIFIED = 0; IDENTITY_STRENGTH_PARTIAL = 1; IDENTITY_STRENGTH_FULL = 2; }

message ExecutableIdentity {
  string path = 1;
  string realpath = 2;
  string sha256 = 3;
  uint64 size_bytes = 4;
  int64 mtime_unix_ns = 5;
  uint64 inode = 6;
  string interpreter_realpath = 7;   // for scripts (shebang)
  string interpreter_sha256 = 8;
  string distribution = 9;           // Python dist name for harnesses
  string distribution_version = 10;
  string record_sha256 = 11;         // sha256 of the dist's installed RECORD file
  IdentityStrength strength = 12;
  repeated string limitations = 13;  // e.g. "editable install: RECORD does not cover sources"
}

// What we learned from a CLI-Anything checkout. install_cmd is recorded, never executed.
message HarnessSource {
  string repo_path = 1;
  string repo_commit = 2;
  string registry_sha256 = 3;
  string registry_name = 4;          // registry.json clis[].name, e.g. "libreoffice"
  string entry_point = 5;            // e.g. "cli-anything-libreoffice"
  string skill_md_path = 6;
  string skill_md_sha256 = 7;
  string install_cmd_recorded = 8;
}

enum AuthorityBasisKind {
  AUTHORITY_BASIS_KIND_UNSPECIFIED = 0;  // registration refused
  AUTHORITY_BASIS_KIND_OWNER = 1;
  AUTHORITY_BASIS_KIND_LICENSED_USER = 2;
  AUTHORITY_BASIS_KIND_EMPLOYER_AUTHORIZED = 3;
}

message AuthorityGrant {
  AuthorityTier max_tier = 1;            // default OBSERVE
  AuthorityBasisKind basis = 2;
  string basis_note = 3;                 // purpose / licence reference, free text
  Principal granted_by = 4;
  google.protobuf.Timestamp granted_at = 5;
}

message Target {
  string id = 1;                         // "tg_" + uuid v7
  string slug = 2;                       // [a-z0-9-]{1,32}; capability id prefix
  TargetKind kind = 3;
  ExecutableIdentity identity = 4;
  HarnessSource harness = 5;             // set iff kind == CLI_ANYTHING_HARNESS
  AuthorityGrant grant = 6;
  string active_manifest_digest = 7;
  repeated string env_allowlist = 8;     // extra env vars passed to the child (default: PATH, HOME, LANG, LC_ALL, TMPDIR)
  google.protobuf.Timestamp registered_at = 9;
}

// ════════════════════════════ capabilities / manifest IR ════════════════════════════

enum CliParamKind {
  CLI_PARAM_KIND_UNSPECIFIED = 0;
  CLI_PARAM_KIND_OPTION = 1;          // --flag VALUE
  CLI_PARAM_KIND_FLAG = 2;            // --flag (bool)
  CLI_PARAM_KIND_POSITIONAL = 3;
  CLI_PARAM_KIND_GLOBAL_OPTION = 4;   // placed before the command path (e.g. --project P)
  CLI_PARAM_KIND_GLOBAL_FLAG = 5;     // e.g. --dry-run
}

message CliParam {
  string field = 1;       // property name in args_json (snake_case, = Click param name)
  CliParamKind kind = 2;
  string flag = 3;        // "--text"; empty for positionals
  uint32 position = 4;
  bool repeated = 5;      // Click multiple=True / nargs=-1
  bool is_path = 6;       // must be absolute; the CLI front-end resolves before sending
  bool sensitive = 7;     // target-declared (Click hide_input) -> redacted in records
  bool required = 8;
}

enum OutputContract {
  OUTPUT_CONTRACT_UNSPECIFIED = 0;
  OUTPUT_CONTRACT_JSON_STDOUT = 1;        // CLI-Anything `--json`
  OUTPUT_CONTRACT_TEXT = 2;
}
enum ErrorConvention {
  ERROR_CONVENTION_UNSPECIFIED = 0;       // exit code only
  ERROR_CONVENTION_CLI_ANYTHING = 1;      // stdout {"error": str, "type": str} + exit 1 (libreoffice_cli.py:86-114)
}

message CliArgvBinding {
  string executable = 1;                  // absolute realpath
  repeated string fixed_global_args = 2;  // e.g. ["--json"]
  repeated string command_path = 3;       // e.g. ["export", "presets"]; only LEAF commands become capabilities
  repeated CliParam params = 4;
  OutputContract output = 5;
  ErrorConvention errors = 6;
  uint32 default_timeout_ms = 7;          // default 60000
  uint64 max_stdout_bytes = 8;            // default 16 MiB
  uint64 max_stderr_bytes = 9;            // default 1 MiB
}

message Binding {
  oneof kind {
    CliArgvBinding cli_argv = 1;
    // reserved for: http_op = 2; mcp_tool = 3; adapter_method = 4 (out-of-process, aa.adapter.v1); a11y_action = 5;
  }
  reserved 2 to 15;
}

enum Lifetime { LIFETIME_UNSPECIFIED = 0; LIFETIME_ONE_SHOT = 1; LIFETIME_SESSION = 2; /* SESSION reserved (M6) */ }

enum CapabilityState { CAPABILITY_STATE_UNSPECIFIED = 0; CAPABILITY_STATE_DISABLED = 1; CAPABILITY_STATE_ENABLED = 2; }

// "Withheld with a reason" (REA docs/agent-environments.md:175-186). ENABLED iff reasons is empty.
enum DisabledReason {
  DISABLED_REASON_UNSPECIFIED = 0;
  DISABLED_REASON_UNREVIEWED = 1;
  DISABLED_REASON_EFFECTS_UNKNOWN = 2;
  DISABLED_REASON_NO_READ_BACK_ORACLE = 3;     // any WRITE/DESTRUCTIVE capability in the MVP
  DISABLED_REASON_VERIFICATION_FAILED = 4;
  DISABLED_REASON_TARGET_CHANGED = 5;
  DISABLED_REASON_OPERATOR_DISABLED = 6;
  DISABLED_REASON_EXCEEDS_GRANT = 7;
}

enum LaneStatus { LANE_STATUS_UNSPECIFIED = 0; LANE_STATUS_PASS = 1; LANE_STATUS_FAIL = 2; LANE_STATUS_SKIPPED = 3; LANE_STATUS_NOT_APPLICABLE = 4; }
message LaneResult {
  string lane = 1;           // "L0_STATIC", "L1_ADDRESS", later "L3_ORACLE", ...
  LaneStatus status = 2;
  string detail = 3;         // SKIPPED requires a reason here; a named skip is never a pass
  string record_id = 4;      // VERIFICATION ActionRecord that produced it
}

message Provenance {
  string source = 1;          // discovery source id, e.g. "click-introspect"
  string source_version = 2;
  DiscoveryAuthority authority_used = 3;
  string raw_digest = 4;      // sha256 of the verbatim harvest stored under manifests/<target>/raw/
  string native_address = 5;  // e.g. "cli-anything-libreoffice export presets"
}

message Example { string title = 1; string args_json = 2; }   // validated against input_schema_json at load

message Capability {
  string id = 1;                     // "<target-slug>.<group...>.<command>", e.g. "libreoffice.export.presets"
  string target_id = 2;
  string title = 3;                  // derived from id
  TargetText summary = 4;            // Click short_help
  TargetText description = 5;        // Click help
  string input_schema_json = 6;      // closed JSON Schema 2020-12
  string output_schema_json = 7;     // "" = unknown (MVP); sampled/observed in M1
  Effects effects = 8;               // effective (manifest signals ⊕ reviews)
  Annotations annotations = 9;       // derived from effects by the engine; front-ends never recompute
  AuthorityTier required_authority = 10;
  Binding binding = 11;
  Lifetime lifetime = 12;
  CapabilityState state = 13;
  repeated DisabledReason disabled_reasons = 14;
  repeated LaneResult verification = 15;
  Provenance provenance = 16;
  bool returns_target_content = 17;  // true for every MVP capability
  repeated Example examples = 18;
  string capability_digest = 19;     // JCS sha256 over semantic fields (excludes state/verification/annotations)
}

message DiscoverySourceRun {
  string source = 1;
  string source_version = 2;
  DiscoveryAuthority authority_used = 3;
  string raw_digest = 4;
  int64 duration_ms = 5;
  repeated TargetText diagnostics = 6;
}

message Manifest {
  uint32 schema_version = 1;          // 1
  string target_id = 2;
  ExecutableIdentity identity = 3;    // snapshot at discovery time
  repeated DiscoverySourceRun sources = 4;
  repeated Capability capabilities = 5;  // sorted by id (code point); order never selects anything
  string manifest_digest = 6;         // "mf_" + sha256(JCS(manifest minus this field, minus timestamps))
  google.protobuf.Timestamp created_at = 7;  // excluded from digest
}

// ════════════════════════════ action records (evidence) ════════════════════════════

enum RecordKind {
  RECORD_KIND_UNSPECIFIED = 0;
  RECORD_KIND_TARGET_REGISTERED = 1;
  RECORD_KIND_DISCOVERY = 2;
  RECORD_KIND_REVIEW = 3;
  RECORD_KIND_VERIFICATION = 4;
  RECORD_KIND_INVOCATION = 5;
}

enum Decision {
  DECISION_UNSPECIFIED = 0;
  DECISION_ALLOW = 1;
  DECISION_DENY = 2;
  DECISION_REJECTED_INPUT = 3;
  DECISION_NEEDS_CONFIRMATION = 4;    // reserved (M3)
}

message PolicyDecision {
  Decision decision = 1;
  string rule = 2;                    // e.g. "capability_disabled", "exceeds_grant", "target_changed", "input_schema"
  repeated DisabledReason reasons = 3;
  AuthorityTier required = 4;
  AuthorityTier granted = 5;
  AuthorityBasisKind basis = 6;
}

enum OutcomeStatus {
  OUTCOME_STATUS_UNSPECIFIED = 0;
  OUTCOME_STATUS_NOT_DISPATCHED = 1;    // policy denial / rejected input: nothing was launched
  OUTCOME_STATUS_COMPLETED = 2;         // exit 0 and output satisfied the output contract
  OUTCOME_STATUS_TARGET_ERROR = 3;      // target reported an error (upstream reason preserved)
  OUTCOME_STATUS_UNREADABLE_OUTPUT = 4; // exit 0 but output violated the contract (e.g. non-JSON)
  OUTCOME_STATUS_EXECUTION_FAILURE = 5; // non-zero exit without a recognizable error object, or signal
  OUTCOME_STATUS_TIMEOUT = 6;
  OUTCOME_STATUS_CANCELLED = 7;
  OUTCOME_STATUS_SPAWN_FAILED = 8;
  OUTCOME_STATUS_RESOURCE_CONSTRAINT = 9; // output exceeded budget; complete stdout not retained
}

message CapturedStream {
  string sha256 = 1;                  // over the full stream as read (up to budget)
  uint64 bytes = 2;
  bool truncated = 3;
  TargetText excerpt = 4;             // head+tail excerpt (REA BoundedOutputExcerpt.ts:1-46), credentials redacted
}

message ActionIssued {
  repeated string argv = 1;           // sensitive values replaced by "<redacted:N bytes>"
  string cwd = 2;                     // private per-run directory
  repeated string env_keys = 3;       // names only, never values
  string run_id = 4;                  // AA_RUN_ID set last in the child env (REA ProviderProcess.ts:277-283)
  uint32 timeout_ms = 5;
}

enum CleanupStatus {
  CLEANUP_STATUS_UNSPECIFIED = 0;
  CLEANUP_STATUS_EXITED = 1;               // child exited on its own; group signalled anyway
  CLEANUP_STATUS_GROUP_KILLED = 2;         // deadline/cancel: SIGTERM -> SIGKILL to the process group
  CLEANUP_STATUS_INCOMPLETE = 3;           // could not confirm; retained for retry (M3)
}

message Outcome {
  OutcomeStatus status = 1;
  int32 exit_code = 2;
  int32 signal = 3;
  int64 duration_ms = 4;
  CapturedStream stdout = 5;
  CapturedStream stderr = 6;
  string result_json = 7;             // parsed stdout JSON (COMPLETED) or the target's error object (TARGET_ERROR)
  Error error = 8;                    // set for every non-COMPLETED status
  CleanupStatus cleanup = 9;
  bool private_root_removed = 10;
}

enum Confidence { CONFIDENCE_UNSPECIFIED = 0; CONFIDENCE_OBSERVED = 1; CONFIDENCE_DERIVED = 2; CONFIDENCE_INFERRED = 3; }
enum Isolation { ISOLATION_UNSPECIFIED = 0; ISOLATION_NONE = 1; ISOLATION_PROCESS = 2; ISOLATION_CONTAINER = 3; ISOLATION_VM = 4; }

message ResidualUnknown { string scope = 1; string reason = 2; }  // e.g. {"post_state", "no read-back oracle"}

message ActionRecord {
  string record_id = 1;               // "ar_" + sha256(JCS(record minus record_id))
  string prev_record_id = 2;          // hash chain; "" only for the genesis record
  uint64 seq = 3;
  RecordKind kind = 4;
  google.protobuf.Timestamp started_at = 5;
  google.protobuf.Timestamp completed_at = 6;
  Principal principal = 7;
  string target_id = 8;
  ExecutableIdentity target_identity = 9;  // as verified at action time
  string capability_id = 10;
  string capability_digest = 11;
  string manifest_digest = 12;
  string args_digest = 13;            // sha256(JCS(args)) before redaction
  string args_redacted_json = 14;     // sensitive pointers replaced by {"sensitive":true,"bytes":N}
  PolicyDecision decision = 15;
  ActionIssued issued = 16;           // absent when NOT_DISPATCHED
  Outcome outcome = 17;
  Confidence confidence = 18;         // OBSERVED for direct invocations
  Isolation isolation = 19;           // NONE in the MVP; said honestly
  repeated string limitations = 20;   // e.g. "exit 0 does not prove the operation's effect"
  repeated ResidualUnknown residual_unknowns = 21;
  bool untrusted_content = 22;        // true whenever outcome carries TargetText
  string payload_json = 23;           // kind-specific body: discovery summary, review decision, lane results
  repeated string links = 24;         // related record ids (e.g. the REVIEW that enabled the capability)
}

// ════════════════════════════ errors ════════════════════════════

// REA's closed envelope (src/contracts/errorSchemas.ts:19-101), trimmed and extended for driving.
enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  ERROR_CODE_INVALID_REQUEST = 1;          // malformed request / args violate input schema
  ERROR_CODE_TARGET_NOT_FOUND = 2;
  ERROR_CODE_CAPABILITY_NOT_FOUND = 3;
  ERROR_CODE_CAPABILITY_DISABLED = 4;
  ERROR_CODE_POLICY_DENIED = 5;            // exceeds grant / out-of-scope target
  ERROR_CODE_TARGET_CHANGED = 6;           // identity differs from the manifest's
  ERROR_CODE_MANIFEST_CHANGED = 7;         // expected_manifest_digest mismatch
  ERROR_CODE_TARGET_UNAVAILABLE = 8;       // executable/interpreter missing
  ERROR_CODE_TARGET_ERROR = 9;             // target reported failure
  ERROR_CODE_UNREADABLE_OUTPUT = 10;
  ERROR_CODE_EXECUTION_FAILURE = 11;
  ERROR_CODE_TIMEOUT = 12;
  ERROR_CODE_CANCELLED = 13;
  ERROR_CODE_RESOURCE_CONSTRAINT = 14;
  ERROR_CODE_CLEANUP_INCOMPLETE = 15;
  ERROR_CODE_CONFIGURATION_INVALID = 16;
  ERROR_CODE_DISCOVERY_FAILED = 17;
  ERROR_CODE_CONFIRMATION_REQUIRED = 18;   // reserved (M3)
  ERROR_CODE_INTERNAL = 19;
}

enum ErrorCategory {
  ERROR_CATEGORY_UNSPECIFIED = 0;
  ERROR_CATEGORY_INVALID_INPUT = 1;
  ERROR_CATEGORY_NOT_FOUND = 2;
  ERROR_CATEGORY_POLICY = 3;
  ERROR_CATEGORY_INTEGRITY = 4;
  ERROR_CATEGORY_UNAVAILABLE = 5;
  ERROR_CATEGORY_TARGET = 6;
  ERROR_CATEGORY_TIMEOUT = 7;
  ERROR_CATEGORY_CANCELLED = 8;
  ERROR_CATEGORY_RESOURCE = 9;
  ERROR_CATEGORY_INTERNAL = 10;
}

message Recovery {
  oneof kind {
    string inspect_input_contract = 1;   // capability id; only for malformed input, never for target misses
    string refresh_catalog = 2;          // target id; re-read capabilities / manifest digest
    string rediscover_target = 3;        // target id; identity changed
    string review_capability = 4;        // capability id; needs operator review
    string retry_cleanup = 5;            // record id (M3)
  }
}

message Error {
  ErrorCode code = 1;
  ErrorCategory category = 2;
  string message = 3;                    // engine-authored; target text goes in target_detail
  bool retryable = 4;
  string remediation_action = 5;         // imperative, names where to act (REA limits-and-recovery rules)
  Recovery recovery = 6;
  TargetText target_detail = 7;          // e.g. the harness's {"error": ...} message, preserved verbatim
  string details_json = 8;
}

// ════════════════════════════ services ════════════════════════════

service EngineService {
  rpc GetInfo(GetInfoRequest) returns (GetInfoResponse);
}
message GetInfoRequest {}
message GetInfoResponse { EngineInfo info = 1; }
message EngineInfo {
  string engine_version = 1;
  string api_version = 2;               // "aa.v1"
  string catalog_digest = 3;            // over all ENABLED capability projections (REA catalogIdentity.ts:43-56)
  string audit_head_record_id = 4;
  string home_dir = 5;
  string socket_path = 6;
  Isolation isolation = 7;              // NONE
  string isolation_proof = 8;           // "not-proven" (REA WindowsAuthority.ts:8-33 pattern)
}

service TargetService {
  rpc RegisterTarget(RegisterTargetRequest) returns (RegisterTargetResponse);
  rpc ListTargets(ListTargetsRequest) returns (ListTargetsResponse);
  rpc GetTarget(GetTargetRequest) returns (GetTargetResponse);
}
message RegisterTargetRequest {
  string slug = 1;
  oneof source {
    string executable_path = 2;                   // generic CLI (Click-detected in MVP; more sources in M1)
    CliAnythingHarnessRef cli_anything = 3;
  }
  AuthorityGrant grant = 4;                       // basis required; max_tier defaults to OBSERVE
  repeated string env_allowlist = 5;
}
message CliAnythingHarnessRef {
  string repo_path = 1;            // CLI-Anything checkout (for registry.json + SKILL.md), read statically
  string registry_name = 2;        // e.g. "libreoffice"
  string python = 3;               // interpreter of the venv where the harness is installed
}
message RegisterTargetResponse { Target target = 1; ActionRecord record = 2; }
message ListTargetsRequest {}
message ListTargetsResponse { repeated Target targets = 1; }
message GetTargetRequest { string target = 1; /* id or slug */ }
message GetTargetResponse { Target target = 1; }

// The pipeline stages that produce or check the IR.
service PipelineService {
  rpc Discover(DiscoverRequest) returns (DiscoverResponse);   // discover + characterize + synthesize + L0/L1 verify
  rpc Verify(VerifyRequest) returns (VerifyResponse);         // re-run lanes on the active manifest
  rpc Render(RenderRequest) returns (RenderResponse);         // M2: SKILL.md, MCP tool list, CLI reference
}
message DiscoverRequest {
  string target = 1;
  repeated string sources = 2;                     // default: auto-detect
  DiscoveryAuthority max_authority = 3;            // default INTROSPECT; PROBE/EXPLORE refused in MVP
  bool activate = 4;                               // make it the active manifest (default true)
}
message ManifestDiff { repeated string added = 1; repeated string removed = 2; repeated string changed = 3; }
message DiscoverResponse { Manifest manifest = 1; ManifestDiff diff = 2; ActionRecord record = 3; }
message VerifyRequest { string target = 1; repeated string lanes = 2; }
message VerifyResponse { repeated Capability capabilities = 1; repeated string non_claims = 2; ActionRecord record = 3; }
enum RenderFormat { RENDER_FORMAT_UNSPECIFIED = 0; RENDER_FORMAT_SKILL_MD = 1; RENDER_FORMAT_MCP_TOOLS_JSON = 2; RENDER_FORMAT_CLI_REFERENCE = 3; }
message RenderRequest { string target = 1; RenderFormat format = 2; bool include_disabled = 3; }
message RenderedFile { string relative_path = 1; bytes content = 2; string sha256 = 3; }
message RenderResponse { repeated RenderedFile files = 1; string manifest_digest = 2; }

service CatalogService {
  rpc GetManifest(GetManifestRequest) returns (GetManifestResponse);
  rpc ListCapabilities(ListCapabilitiesRequest) returns (ListCapabilitiesResponse);
  rpc GetCapability(GetCapabilityRequest) returns (GetCapabilityResponse);
  rpc ReviewCapability(ReviewCapabilityRequest) returns (ReviewCapabilityResponse);
}
message GetManifestRequest { string target = 1; string digest = 2; /* "" = active */ }
message GetManifestResponse { Manifest manifest = 1; }
message ListCapabilitiesRequest {
  string target = 1;                 // "" = all targets
  CapabilityState state = 2;         // UNSPECIFIED = any
  string query = 3;                  // substring over id/summary (backs MCP describe_capabilities in M2)
  uint32 page_size = 4;
  string page_token = 5;
}
message ListCapabilitiesResponse { repeated Capability capabilities = 1; string next_page_token = 2; string catalog_digest = 3; }
message GetCapabilityRequest { string capability_id = 1; }
message GetCapabilityResponse { Capability capability = 1; }
enum ReviewAction { REVIEW_ACTION_UNSPECIFIED = 0; REVIEW_ACTION_SET_EFFECTS = 1; REVIEW_ACTION_DISABLE = 2; REVIEW_ACTION_CLEAR = 3; }
message ReviewCapabilityRequest {
  string capability_id = 1;
  string capability_digest = 2;      // the version the reviewer saw; mismatch -> MANIFEST_CHANGED
  ReviewAction action = 3;
  Effects effects = 4;               // operator decision; MVP accepts only "all hazards NO" (= READ)
  bool override_keyword_signals = 5; // operator may lower KEYWORD raises only with this; never OBSERVED ones
  string note = 6;
}
message ReviewCapabilityResponse { Capability capability = 1; ActionRecord record = 2; }

service InvocationService {
  rpc Invoke(InvokeRequest) returns (InvokeResponse);
}
message InvokeRequest {
  string capability_id = 1;
  string args_json = 2;                     // canonical input object; validated against input_schema_json
  string expected_manifest_digest = 3;      // optional stale-catalog guard
  uint32 timeout_ms = 4;                    // <= binding default_timeout_ms
  repeated string sensitive_pointers = 5;   // RFC 6901 pointers into args to redact (caller-declared)
  string idempotency_key = 6;               // reserved (M3)
}
// Always carries a record once the capability resolved, including denials and rejected input.
message InvokeResponse { ActionRecord record = 1; }

service AuditService {
  rpc ListRecords(ListRecordsRequest) returns (ListRecordsResponse);
  rpc GetRecord(GetRecordRequest) returns (GetRecordResponse);
  rpc VerifyChain(VerifyChainRequest) returns (VerifyChainResponse);
}
message ListRecordsRequest { string target = 1; string capability_id = 2; RecordKind kind = 3; uint64 after_seq = 4; uint32 limit = 5; }
message ListRecordsResponse { repeated ActionRecord records = 1; uint64 last_seq = 2; }
message GetRecordRequest { string record_id = 1; }
message GetRecordResponse { ActionRecord record = 1; }
message VerifyChainRequest { uint64 from_seq = 1; }
message VerifyChainResponse { bool ok = 1; uint64 checked = 2; string head_record_id = 3; uint64 first_bad_seq = 4; string detail = 5; }
```

### 3.1 Derivation rules the engine owns (no front-end recomputes them)

- `hazard(x) = x != TRI_NO` (UNKNOWN counts as YES).
- `level`: DESTRUCTIVE if `hazard(may_discard_data) || hazard(executes_foreign_code) || hazard(mutates_system)`,
  with one exception: if *every* hazard is UNKNOWN, the level is UNKNOWN. Otherwise WRITE if any other hazard holds.
  Otherwise READ.
- `annotations`: `read_only = level == READ`. `destructive = level ∈ {DESTRUCTIVE, UNKNOWN}`.
  `open_world = hazard(accesses_network) || hazard(launches_process)`. `idempotent = idempotent == TRI_YES`.
  This is REA's formula (`toolEffects.ts:765-787`) with the unknown→hazard rule from
  `survey-capability-discovery-and-distillation.md` §4.3.
- `required_authority`: READ maps to OBSERVE. Everything else maps to MUTATE.
- **Signal combination** (survey §4.3, rules 1-5): the effective value per effect is computed in this order.
  1. If any `OBSERVED` YES exists, the value is YES.
  2. Otherwise, if an `OPERATOR` decision exists and every `KEYWORD` YES is either absent or explicitly overridden,
     the operator value applies.
  3. Otherwise the value is the max over `TARGET_DECLARED` / `CONVENTION` / `KEYWORD` signals.
  4. Otherwise it is UNKNOWN.

  `PROPOSAL` signals never apply.

### 3.2 Error mapping

| Situation | Transport result | Record? | `aa` exit |
|---|---|---|---|
| Unknown target/capability, malformed request envelope | `NOT_FOUND` / `INVALID_ARGUMENT` + `aa.v1.Error` detail | no | 2 (usage) |
| Args fail the capability's input schema | OK, `decision=REJECTED_INPUT`, `NOT_DISPATCHED`, `Error{INVALID_REQUEST, recovery.inspect_input_contract}` | yes | 2 |
| Capability disabled / exceeds grant / target changed / manifest changed | OK, `decision=DENY`, `NOT_DISPATCHED` | yes | 3 |
| Target ran: COMPLETED | OK | yes | 0 |
| Target ran: TARGET_ERROR / UNREADABLE_OUTPUT / EXECUTION_FAILURE / TIMEOUT / RESOURCE_CONSTRAINT | OK, `Error` set | yes | 1 |
| Client cancelled (RPC dropped) | n/a (client gone) | yes, `CANCELLED`, group killed | 130 (CLI SIGINT) |
| Engine not reachable | connect error | no | 4 |

The distinction between rows 2 and 3 is REA's malformed-vs-unresolvable rule (`rea-tool-design-philosophy-and-verification.md`
§3.9). The exit-code split 0/1/2/3 is REA's recommended contract (`rea-cli-surface-and-agent-packaging.md` §10.2). Exit
4 is new: an agent should restart the engine, not retry the call.

---

## 4. Transport and local security

**Default and only MVP transport: a Unix domain socket, owner-only, with a peer-credential check. No token, no TCP.**

- Runtime dir: `$AA_RUNTIME_DIR`, else `$XDG_RUNTIME_DIR/aa` (Linux), else `/tmp/aa-<uid>` (macOS). It is short on
  purpose: `sockaddr_un` is 104 bytes on macOS, which REA hit and worked around (`src/ghidra/GhidraTransport.ts:42-60`).
- On start, `aad` creates the directory with mode 0700. If it already exists, the engine `lstat`s it and **refuses to
  start** unless it is a real directory owned by `geteuid()` with no group/other bits. This is REA's private runtime
  root posture (`src/process/PrivateRuntimeRoot.ts:30-97`).
- The engine takes `flock(engine.lock)` (single instance). With the lock held, it unlinks any stale `engine.sock`,
  sets `umask(0o177)`, binds, and verifies the socket mode is 0600.
- A tonic interceptor reads `UdsConnectInfo.peer_cred` from the request extensions and rejects with
  `PERMISSION_DENIED` unless `peer uid == engine euid`. The uid and pid go into `Principal` on every record.
- **Why no token on the UDS:** any process that can read a token file under the same uid can also connect to a 0600
  socket. A token adds nothing against a same-uid attacker, and REA says so directly: "the token protects the IPC
  channel, not the user account" (`SECURITY.md:27-36`, via `rea-engine-bridges-and-process-supervision.md` §1). The
  honest non-claim goes into `EngineInfo.isolation_proof = "not-proven"`.
- **Loopback TCP (deferred to M4, for Windows and containers):** bind `127.0.0.1:0` only, never a non-loopback
  address. Publish the port by atomic rename. Require a 32-byte bearer token from a 0600 file, compared in constant
  time, plus a `:authority`/Host allowlist. This follows the REA HTTP posture (`src/config/mcpHttpEndpoint.ts:13-14`;
  `rea-mcp-server-adapter.md` "HTTP posture"). Windows named pipes with a private DACL are also M4.
- **Child processes** never see the socket path or any engine secret. The env is scrubbed to an allowlist, and
  `AA_RUN_ID` is set last. The run id is an ownership marker, not a credential (REA `ProviderProcess.ts:277-283`).

---

## 5. Adapter and binding model

### 5.1 Two traits in `aa-core`, both in-process in the MVP

```rust
/// Discovery is split so harvesting (effectful) is recorded verbatim and candidates (pure) replay in tests.
#[async_trait] pub trait DiscoverySource: Send + Sync {
    fn id(&self) -> &'static str;                 // "click-introspect"
    fn version(&self) -> &'static str;
    fn authority(&self) -> DiscoveryAuthority;    // STATIC | INTROSPECT
    fn applies(&self, t: &Target) -> bool;
    async fn harvest(&self, t: &Target, run: &RunCtx) -> Result<RawHarvest, CoreError>; // bytes + sha256
    fn candidates(&self, raw: &RawHarvest) -> Result<Vec<Candidate>, CoreError>;       // pure
}

#[async_trait] pub trait Binding: Send + Sync {
    fn kind(&self) -> BindingKind;                // CliArgv in the MVP
    async fn invoke(&self, cap: &Capability, args: &serde_json::Value, ctx: InvokeCtx) -> Outcome;
}
```

Sources and bindings are registered in one `const` table per kind (a complete, typed map in the spirit of REA's
`registerToolFamilies`, `rea-capability-registry-and-composition.md` §10). Every `Capability.binding` variant must
have exactly one `Binding` impl. This is a set-equality test, not a review item.

### 5.2 What runs where

| Thing | Process | Why |
|---|---|---|
| Engine, pipeline, policy, audit | `aad` | single authority point; all writes to state happen here |
| Click introspection shim | child of `aad`, run by the **harness's own Python** (`-I`), one-shot | it must import the harness's code to call `to_info_dict()`. That is target code, so it runs out-of-process under INTROSPECT authority, with a scrubbed env, a private cwd, stdin=/dev/null and a 20 s deadline |
| Target invocation | child of `aad`, one-shot, own process group | REA owned one-shot command pattern (`src/process/OwnedCommand.ts:163-210`). The engine kills the group on deadline, cancellation, or when the RPC future is dropped (Drop guard) |
| Future adapters (HTTP, MCP-proxy, plugin bridges, a11y helpers) | out-of-process, via `aa.adapter.v1` (M4) | crash isolation and any-language adapters. REA's six bridge shapes and bridge-kit spec (`rea-engine-bridges-and-process-supervision.md` §8) is the blueprint |

### 5.3 The out-of-process seam (draft only; not compiled until M4)

```proto
// proto/aa/adapter/v1/adapter.proto  (DRAFT)
// The engine is the gRPC *client*. It spawns the adapter with a 0600 descriptor file (REA bridge-kit §8.2)
// naming a private UDS. The adapter's Hello must echo run_id, exact capability set, and target identity.
service AdapterService {
  rpc Hello(HelloRequest) returns (HelloResponse);           // identity contract; mismatch -> teardown
  rpc Harvest(HarvestRequest) returns (HarvestResponse);     // raw bytes + candidates (same shapes as in-process)
  rpc Invoke(AdapterInvokeRequest) returns (stream AdapterEvent); // progress events, then exactly one outcome
  rpc Shutdown(ShutdownRequest) returns (ShutdownResponse);  // reports what it closed vs retained
}
```

`Binding.adapter_method = 4` is reserved for this. An in-process `Binding` impl forwards to it, so the policy gate and
ActionRecord code stay unchanged.

---

## 6. How the pipeline maps onto crates and RPCs

| Stage | `aa-core` module | RPC | Artifact | MVP depth | Later |
|---|---|---|---|---|---|
| Register | `policy::admit_target`, `discover::cli_anything` (static read of `registry.json`, `SKILL.md`) | `TargetService.RegisterTarget` | `Target` row; TARGET_REGISTERED record with authority basis | CLI-Anything harness or Click-script path; basis required | out-of-scope denylist by signer/publisher (M3) |
| Discover | `discover::{click_introspect, manual}` | `PipelineService.Discover` | `manifests/<t>/raw/<sha>.json` verbatim harvest | Click `to_info_dict` via shim; manual manifest import | `cli-help` source: Cobra `__complete`, Click `_X_COMPLETE`, `--help` parse (M1); OpenAPI/MCP/sdef via adapters (M4) |
| Characterize | `characterize` | (inside Discover) | `Effects` with per-signal provenance | signal table §8; floors only; operator review lowers to READ | OBSERVED signals from sandbox diff (M3); LLM PROPOSAL signals (M5) |
| Synthesize | `synthesize` | (inside Discover) | `Manifest` (`mf_` digest) | naming, closed schemas, `cli_argv` bindings, examples=none | output schema from sampled results (M1); curated core set + families (M2) |
| Verify | `verify` | `Discover` (auto), `PipelineService.Verify` | VERIFICATION record + `LaneResult`s | L0 static, L1 address | L3 read-back oracle + sandbox diff, seed-fails/end-passes (M3); golden replay on upgrade (M5) |
| Review | `characterize::apply_review` | `CatalogService.ReviewCapability` | REVIEW record; effective `Capability` | READ approvals only | WRITE approvals gated on L3 PASS (M3) |
| Package | `render` | `PipelineService.Render` (M2); `CatalogService.ListCapabilities` | SKILL.md, MCP tool projections | `aa` generic `call` only | MCP server, skills, usage KDL export (M2) |
| Invoke | `policy::admit_invocation`, `runner`, `audit` | `InvocationService.Invoke` | INVOCATION record | `cli_argv` one-shot | confirmations, idempotency keys (M3); sessions (M6) |

### 6.1 Verification lanes in the MVP (deterministic, executed by code, never by an LLM)

- **L0 static** (every capability):
  - the input schema compiles under `jsonschema` and is closed at the root;
  - the effects decision exists, and every field is set or explicitly UNKNOWN;
  - ids are unique and match `^[a-z0-9-]+(\.[a-z0-9-]+)+$`;
  - every `CliParam.field` appears in the schema and the reverse holds (set equality);
  - the binding executable exists, and its identity matches the manifest snapshot;
  - `annotations == derive(effects)`.
- **L1 address** (every capability): run `<exe> <command_path...> --help` (INTROSPECT authority, same runner).
  Exit 0 proves the address resolves on the *installed* binary. This follows CLI-Anything's "test the installed
  entry point" doctrine (`HARNESS.md:166-214`, via `survey-agent-interface-generators.md` §2.5).
- The report's `non_claims`: "does not establish effects", "does not establish output schema",
  "host: <os/arch> only", "isolation: none". A named skip is never a pass (REA `docs/testing-lanes.md:18-23`).

---

## 7. Policy gate (`policy::admit_invocation`), in order

1. Resolve the capability by id. If it is absent, return `NOT_FOUND` with no record.
2. Open the record context. Everything below produces exactly one INVOCATION record.
3. If `expected_manifest_digest` is set and differs: DENY with `MANIFEST_CHANGED`, `recovery.refresh_catalog`.
4. If `state != ENABLED`: DENY with `CAPABILITY_DISABLED` and the reasons; `recovery.review_capability` if UNREVIEWED.
5. If `required_authority > target.grant.max_tier`: DENY with `POLICY_DENIED`, `EXCEEDS_GRANT`.
6. **Target identity re-check.** The engine compares the cached `(dev, inode, size, mtime)` of the executable and the
   interpreter. On any change it rehashes. If the SHA-256 differs from the manifest snapshot: DENY with
   `TARGET_CHANGED`, `recovery.rediscover_target`, and the capability gains `DISABLED_REASON_TARGET_CHANGED`. For
   Python harnesses the engine also rehashes the dist `RECORD` (a small file). The record stores
   `identity_state = path_metadata_unchanged | rehashed_same | changed` (REA `executable_identity {state, reason}`,
   `src/domain/process/processCapture.ts:260-263`).
7. Validate `args_json` against the closed schema. On failure: REJECTED_INPUT with `INVALID_REQUEST` and
   `recovery.inspect_input_contract`. Path fields must be absolute.
8. Pass to the runner.

There is no confirmation step in the MVP, because nothing that would need one can be enabled. The
`DECISION_NEEDS_CONFIRMATION` and `ERROR_CODE_CONFIRMATION_REQUIRED` slots are reserved for M3.

### 7.1 Runner (`runner.rs`), as a checklist

- `tokio::process::Command` with an argv vector and no shell. Never run a target's `install_cmd`, even though the
  CLI-Anything hub does with `shell=True` (`cli-hub/cli_hub/installer.py:67-93`).
- argv = `[exe] + fixed_global_args + global params + command_path + options + positionals` (§9 mapping).
- Process setup:
  - `stdin(Stdio::null())`. This matters: CLI-Anything groups enter a REPL when invoked without a subcommand
    (`libreoffice_cli.py:117-139`), and only leaf commands become capabilities.
  - `process_group(0)` and `kill_on_drop`, plus a Drop guard that `killpg`s SIGTERM, then SIGKILL after 1 s
    (REA TERM 250 ms → KILL 1000 ms, `ProviderProcess.ts:710-768`).
  - `env_clear()` plus the allowlist, with `AA_RUN_ID` last.
  - `cwd` = `runs/<run_id>/`, created 0700 and removed afterwards.
- Read stdout/stderr concurrently into byte-counted buffers with the binding's caps. On overflow, keep draining
  without retaining, and mark `RESOURCE_CONSTRAINT`. Build excerpts head+tail.
- Classify the outcome:
  - exit 0 + JSON → COMPLETED.
  - exit 0 + non-JSON under `JSON_STDOUT` → UNREADABLE_OUTPUT.
  - exit ≠ 0 + stdout is a JSON object with `"error"` under `ERROR_CONVENTION_CLI_ANYTHING` → TARGET_ERROR, with the
    upstream message preserved verbatim in `target_detail`. REA's JEB refusal bug came from losing this
    (`rea-adding-a-new-target-walkthrough.md` summary 4).
  - Otherwise → EXECUTION_FAILURE.
- Always add the limitation "exit status and output do not establish the operation's effect on target state", and
  `residual_unknowns: [{post_state, "no read-back oracle in this version"}]` (CLI-Anything "never trust exit 0",
  `survey-agent-interface-generators.md` §2.5).

---

## 8. Characterization: the MVP signal table (data, in `characterize.rs`)

All OBSERVED conventions are from the CLI-Anything LibreOffice harness and Click. The mapping is INFERRED, following
`survey-capability-discovery-and-distillation.md` §4.3.

| Signal (Click `to_info_dict` / CLI-Anything) | Source | Effect raised |
|---|---|---|
| global `--project` option present (CLI-Anything auto-save in `result_callback`, `libreoffice_cli.py:142-153`) | CONVENTION | `writes_filesystem`: stays UNKNOWN unless reviewed; recorded so the reviewer sees it |
| option/argument type `Path(writable=True)`, `File('w'/'a')`, or name `output`, `output_path`, `-o` | TARGET_DECLARED / KEYWORD | `writes_filesystem = YES` |
| flag named `overwrite`, `force`, `yes`, `purge`, `hard` | KEYWORD | `may_discard_data = YES` |
| command verb `remove`, `delete`, `clear`, `reset`, `drop`, `undo`, `redo` | KEYWORD | `mutates_target = YES`, `may_discard_data = YES` |
| command verb `add`, `new`, `set`, `create`, `import`, `save`, `rename`, `render`, `apply`, `open` | KEYWORD | `mutates_target = YES` |
| command verb `list`, `info`, `status`, `get`, `show`, `presets`, `profiles`, `formats`, `json`, `preset-info` | PROPOSAL | none applied; shown as "proposed READ" in review |
| option `hide_input=True` or `prompt` | TARGET_DECLARED | param `sensitive = true` (not an effect) |
| global `--dry-run` flag | TARGET_DECLARED | binding records `supports_dry_run` (used by the M3 sandbox lane) |

Keyword rules can only raise. Target-originated names and help text cannot lower anything or grant authority (survey
§4.3 rule 5). This table is the seam where usage-KDL `effect`, Fig `isDangerous`, PowerShell `ConfirmImpact`, sdef
and HTTP-method signals plug in later.

---

## 9. Ingesting CLI-Anything harnesses

CLI-Anything (owner's fork at `/Users/nhanclie/CLI-Anything`, Apache-2.0) gives us, per harness:
- a Click CLI with `--json`/`--project`/`--dry-run` globals (`HARNESS.md:498-529`);
- a `registry.json` entry `{name, display_name, version, description, requires, install_cmd, entry_point, skill_md, category}`;
- a canonical `skills/cli-anything-<name>/SKILL.md`.

It gives us no effect or authority metadata (`survey-agent-interface-generators.md` §2.7).

**Installation is out of the engine.** The operator installs the harness into a venv. The engine records the
`install_cmd` but never runs it:
```
uv venv ~/.local/share/aa/venvs/libreoffice --python 3.12
uv pip install --python ~/.local/share/aa/venvs/libreoffice/bin/python -e /Users/nhanclie/CLI-Anything/libreoffice/agent-harness
```

**Registration** (`aa target add-harness libreoffice --repo /Users/nhanclie/CLI-Anything --python <venv>/bin/python
--basis owner --note "..."`), all STATIC:
1. Read and hash `registry.json`, and select `clis[].name == "libreoffice"`. Record `repo_commit`
   (`git rev-parse HEAD`, read from `.git` files, not by running git).
2. Hash `skills/cli-anything-libreoffice/SKILL.md` and keep a copy as an untrusted reference document under
   `raw/`. It is used later by the renderer, which links to it rather than inlining it.
3. Resolve `<venv>/bin/<entry_point>`, and record the identity of the script and the interpreter.

**Discovery** (`click-introspect` source, INTROSPECT authority):
1. The engine writes `assets/click_introspect.py` (embedded with `include_str!`) into the run's private root. It runs
   `<python> -I <shim> <entry_point>` with stdin null, a scrubbed env and a 20 s deadline. The shim:

   ```python
   # click_introspect.py — protocol "aa-click-introspect/1"
   import hashlib, json, sys
   from importlib.metadata import entry_points
   import click
   name = sys.argv[1]
   eps = [e for e in entry_points(group="console_scripts") if e.name == name]
   if len(eps) != 1: json.dump({"error": "entry_point_not_found", "name": name}, sys.stdout); sys.exit(3)
   ep = eps[0]; obj = ep.load()
   root = obj if isinstance(obj, click.Command) else None
   if root is None:  # CLI-Anything entry points are `main()` wrappers around a module-level group (`cli`)
       mod = sys.modules[ep.module]
       groups = [v for v in vars(mod).values() if isinstance(v, click.Group)]
       children = {id(c) for g in groups for c in g.commands.values()}
       roots = [g for g in groups if id(g) not in children]
       if len(roots) != 1:
           json.dump({"error": "ambiguous_root", "candidates": [g.name for g in roots]}, sys.stdout); sys.exit(3)
       root = roots[0]
   record = ep.dist.read_text("RECORD") if ep.dist else None
   json.dump({
       "protocol": "aa-click-introspect/1",
       "entry_point": {"name": ep.name, "value": ep.value},
       "distribution": {"name": ep.dist.metadata["Name"] if ep.dist else None,
                        "version": ep.dist.version if ep.dist else None,
                        "record_sha256": hashlib.sha256(record.encode()).hexdigest() if record else None},
       "click_version": click.__version__,
       "root": root.to_info_dict(click.Context(root, info_name=name)),
   }, sys.stdout, default=str)
   ```

   Importing the module does not invoke the group, so the REPL never starts. Module-level side effects (e.g.
   `sys.path.insert` in `libreoffice_cli.py:25`) do happen. That is why this step is INTROSPECT, not STATIC, and why
   it is recorded.
2. Store the stdout bytes verbatim as `raw/<sha256>.json`. `candidates()` is a pure function over those bytes, so
   tests replay recorded harvests with no Python at all ("discover → record → verify",
   `rea-adding-a-new-target-walkthrough.md` distill 4).
3. **Synthesis rules:**
   - One capability per **leaf** command. The id is `<slug>.<group...>.<command>`, e.g.
     `libreoffice.writer.add-paragraph`.
   - Root-level params are mapped as follows. `--json` becomes a `fixed_global_args` entry and is not exposed.
     `--project` becomes the optional `project_path` (GLOBAL_OPTION, `is_path`). `--dry-run` becomes `dry_run`
     (GLOBAL_FLAG).
   - Click types map to JSON Schema: `STRING` → string; `INT`/`IntRange` → integer with min/max; `FLOAT` → number;
     `BOOL`/`is_flag` → boolean; `Choice` → enum; `Path`/`File` → string + `is_path`; `multiple`/`nargs=-1` → array.
   - Required comes from `required`. Defaults go into the schema as `default` (informational; the target applies its
     own).
   - `help`/`short_help` become `TargetText`.
   - Every capability starts `DISABLED[UNREVIEWED]`, plus `[NO_READ_BACK_ORACLE]` when the level ≠ READ.
4. Run L0 and L1 automatically, and activate the manifest. Reviews carry over across re-discovery only when
   `capability_digest` is unchanged. A changed capability goes back to UNREVIEWED (drift rule,
   `survey-capability-discovery-and-distillation.md` §6.3).

**Generic CLI binaries on day one:** `aa target add --exe PATH` detects a Python console script (shebang + entry
point present in that interpreter's metadata) and uses the same Click source. Any Click CLI therefore works, not only
CLI-Anything harnesses. Other binaries get the `manual` source: an operator-written manifest file (the IR's
proto3-JSON) that goes through the same characterize/verify/review path. Cobra, clap, Swift ArgumentParser and
`--help` parsing arrive in M1.

---

## 10. Front-ends on top

### 10.1 `aa` CLI (MVP)

```
aa engine status                                   # GetInfo
aa target add-harness <name> --repo P --python P --basis owner|licensed|employer --note TEXT [--slug S]
aa target add --exe PATH --basis ... [--slug S]
aa target list | show <t>
aa discover <t>                                    # prints diff + per-capability level/state
aa caps [<t>] [--state enabled|disabled] [--query Q]
aa cap show <id>                                   # schema, effects with signals, binding, reasons, lanes
aa cap approve <id> --read [--override-keywords] [--note T]   # TTY required unless --operator-noninteractive
aa cap disable <id>
aa verify <t>
aa call <id> [--input-json JSON|@FILE|-] [-a key=value ...] [--sensitive /ptr] [--timeout MS]
aa log [--target T] [--cap ID] [-n N] | aa log show <record_id> | aa log verify
```

- **Output:** compact JSON when stdout is not a TTY, human tables on a TTY, and `--json` to force JSON. Errors are
  printed in the same format on stdout, and exit codes follow §3.2. TOON/jsonl/filters are deferred (REA output stack,
  `rea-cli-surface-and-agent-packaging.md` §3).
- **Input:** `--input-json` accepts inline JSON, `@file`, or `-` for stdin (capped at 8 MiB; REA `cliJsonInput.ts:127-164`).
  `-a key=value` parses value as JSON and falls back to a string. The CLI fetches the capability, resolves `is_path`
  fields against its own cwd, then sends canonical args (REA `cliJsonInput.ts:273-313`). Derived per-capability flags
  (`aa run libreoffice export presets --flag`) arrive in M1, so the canonical object stays the only schema
  (REA §10.1 rule).
- **Approvals:** `aa cap approve` refuses when stdin is not a TTY unless `--operator-noninteractive` is given. The
  record stores the peer uid and the declared principal kind. This makes approvals *attributable, not unforgeable*: a
  same-uid agent can lie. It is stated as a non-claim, consistent with "AI agents never tick human readiness" being a
  governance rule rather than a mechanism (`rea-tool-design-philosophy-and-verification.md` §7.13).

### 10.2 MCP server (M2, `aa mcp`, a gRPC client)

- rmcp stdio server. On `tools/list` it calls `ListCapabilities(state=ENABLED)`. Each capability becomes a tool:
  - The name maps `.`→`__` (MCP host tool-name regexes commonly reject dots).
  - `inputSchema` = `input_schema_json`; `annotations` come from the engine.
  - The description is the target text prefixed with a provenance line.
- Two meta tools, `aa_describe_capabilities(query)` and `aa_invoke(capability_id, args)`, go through the same policy
  gate. They keep the catalog small for large targets (survey §4.4; REA's 120k-token catalog lesson,
  `rea-cli-surface-and-agent-packaging.md` §10.5).
- Results carry `structuredContent` = parsed result, plus `_meta: {"aa.record_id", "provenance": {"untrusted_content": true}}`
  (REA `toolResultEncoding.ts:30-49`). Errors use `isError` with the `aa.v1.Error` projection.
- `aa mcp doctor` spawns the stdio server as a client and checks that the inventory equals
  `ListCapabilities(ENABLED)` and the catalog digest (REA `src/mcpDoctor.ts:236-347`).

### 10.3 Agent Skills (M2, `PipelineService.Render(SKILL_MD)`)

- The engine renders `skills/aa-<slug>/SKILL.md`. The name must match the directory, per agentskills.io (§5.1 of the
  generator survey).
- Content: a generated route table ("when … → `aa call <id>` / MCP tool `<name>`") between marker comments, a CLI
  fallback section, and a link to the upstream CLI-Anything SKILL.md as an untrusted reference.
- Hand-written judgment prose lives outside the markers. CI runs `skills-ref validate`. A skill never names a
  disabled capability (REA R20).

---

## 11. State and persistence

```
$AA_HOME  (default: directories::ProjectDirs data dir, e.g. ~/.local/share/aa or ~/Library/Application Support/aa)
  engine.db                         SQLite (WAL), one writer thread
    targets(id PK, slug UNIQUE, body_json)                         -- proto3 JSON of Target
    manifests(digest PK, target_id, path, created_at, active BOOL)
    reviews(capability_id, capability_digest, body_json, record_id)
    records(seq INTEGER PK, record_id UNIQUE, prev_record_id, kind, target_id, capability_id, created_at, body_json)
      + triggers: BEFORE UPDATE/DELETE ON records -> RAISE(ABORT, 'append-only')
  manifests/<target_id>/<mf_digest>.json      immutable, content-addressed proto3 JSON (reviewable, diffable)
  manifests/<target_id>/raw/<sha256>.json     verbatim harvests (to_info_dict output, registry entry, SKILL.md copy)
$AA_RUNTIME_DIR (0700): engine.sock (0600), engine.lock, runs/<run_id>/ (per-invocation private cwd, removed)
```

- **Identity is content-addressed.** `record_id`, `capability_digest` and `manifest_digest` are SHA-256 over RFC 8785
  JCS of the proto3-JSON form, with the id field and timestamps removed. Display metadata never forks identity (REA
  `src/domain/evidence.ts:232-254`, via `rea-contracts-and-evidence-model.md` §3).
- **Hash chain.** Each record commits to `prev_record_id`. `VerifyChain` recomputes from genesis or from a given
  sequence number. Non-claim: a same-uid process can rewrite the whole file. The chain detects partial edits, not a
  full rewrite, until the head is anchored externally (M5: periodic head export / signed checkpoints).
- **Why SQLite for audit and registry, files for manifests.** `aa log --target X` needs indexed queries on day one,
  and migrating an audit format later is the most painful migration there is. Manifests are the reviewable product, so
  they live as plain files a human can diff and commit. Large outputs are not retained beyond the stdout budget, and
  retain-then-view paging is deferred (REA `rea-contracts-and-evidence-model.md` §4).
- **Secrets.** Sensitive args (target-declared `hide_input`, or caller `sensitive_pointers`) are stored as
  `{"sensitive": true, "bytes": N}` in `args_redacted_json` and argv. `args_digest` is computed before redaction, so
  replays can be compared without storing the secret (REA `processScenario.ts:98-103`). The engine does not guess
  secrecy from names (REA policy, `rea-contracts-and-evidence-model.md` §5).

---

## 12. Testing and verification strategy

The goal is set-equality and recorded-reality tests over snapshots, with real-software lanes that fail rather than
skip.

| Layer | What | Where | Gate |
|---|---|---|---|
| Unit (pure) | effect algebra: property test that no combination with any UNKNOWN hazard yields `read_only_hint`; keyword signals only raise; operator cannot lower OBSERVED; JCS digest stability (field order, key order, timestamps excluded); argv mapping table tests; outcome classification matrix (exit × stdout shape × convention); hash-chain verify detects edit/delete/reorder | `aa-core` | required |
| Replay | `candidates()` + `synthesize()` over **recorded** harvests: `fixtures/harvests/click-notes.json` and `libreoffice.json` (recorded once from the real harness) → insta snapshot of the manifest (the one place a snapshot is right: it is the IR diff a reviewer reads) | `aa-core/tests` | required |
| Catalog conformance | per manifest: every `CliParam` ↔ schema property (set equality); every capability has an effects decision; `annotations == derive(effects)`; every ENABLED capability is READ and links to a REVIEW record; ids unique; binding kind ↔ `Binding` impl table complete; `buf lint` + `buf breaking` against `main` (REA set-equality guards, `rea-capability-registry-and-composition.md` §10) | `aa-core/tests`, CI | required |
| Integration | in-process engine on a temp UDS; real `fixtures/click-notes` harness installed with `uv` into a temp venv; full RPC loop; socket 0600 / dir 0700 / refuses foreign-owned runtime dir; peer-uid check unit-tested via the extracted function | `aa-engine/tests` | required; **fails** if `uv`/Python are missing (CLI-Anything "no graceful degradation", `HARNESS.md:150-163`). The only escape is `AA_SKIP_PYTHON_LANE=<reason>`, recorded as a named skip in the test report |
| E2E | `assert_cmd` drives the real `aa` against a spawned `aad`: the demo script in §13.3, asserting exit codes and record fields (decision, status, redaction, chain ok) | workspace `tests/` | required |
| Live lane | the real CLI-Anything `libreoffice` harness in a uv venv; capabilities `export.presets`, `document.profiles`, `document.import-formats` approved READ and invoked; a dated verification record with commit SHAs, tool versions, host, and non-claims (REA `docs/verification/` pattern, `rea-tool-design-philosophy-and-verification.md` §4.9) | `scripts/live-cli-anything.sh` | manual/nightly in MVP; required lane from M3 |
| Not in MVP | mutation testing of the policy gate (M3, blocking floor), agent-eval L6 (M2+), runtime effect monitor (M3) | — | — |

`fixtures/click-notes` (~80 lines of Python) mirrors the CLI-Anything conventions so CI does not need LibreOffice:
- the globals `--json`, `--project`, `--dry-run`;
- a REPL when invoked without a subcommand;
- `handle_error` emitting `{"error","type"}`;
- read commands `note list` and `note show ID`;
- write commands `note add --text` and `note remove ID`;
- `export render OUT --overwrite`;
- one deliberately broken command that prints a traceback, to exercise UNREADABLE_OUTPUT/EXECUTION_FAILURE;
- a one-entry `registry.json` and SKILL.md.

---

## 13. The first vertical slice (buildable in one day by agents)

### 13.1 Definition of done

From a clean checkout on macOS or Linux with `uv`:
- `cargo test --workspace` passes, including the e2e loop against `fixtures/click-notes`.
- The live demo in §13.3 works against the real CLI-Anything `libreoffice` harness for `export.presets`.

### 13.2 Work packages (parallelizable after WP1)

| WP | Owner (agent) | Contents | Accept when |
|---|---|---|---|
| WP1 (hour 0-1) | A | Workspace, `rust-toolchain.toml`, `aa.proto` as in §3, `aa-proto` build.rs: `tonic_prost_build::configure().file_descriptor_set_path(...).compile_protos(...)` + `pbjson_build::Builder::new().register_descriptors(..).build(&[".aa.v1"])`; `buf lint` clean | `cargo build -p aa-proto`; a Manifest round-trips through serde JSON |
| WP2 | B | `discover::{cli_anything, click_introspect, manual}`, shim asset, `characterize` (table §8), `synthesize`, `verify` L0/L1, recorded-harvest replay tests | replay snapshot for click-notes; all-UNKNOWN-by-default property holds |
| WP3 | C | `runner` (argv mapping, env scrub, private cwd, deadline, killpg Drop guard, bounded capture, classification), `policy::admit_invocation`, `audit` (SQLite, JCS, chain, triggers) | unit matrix green; killing the RPC future kills a `sleep 30` child group within 2 s |
| WP4 | D | `aa-engine`: runtime-dir checks, flock, UDS bind 0600, peer-uid interceptor, 6 services as thin wrappers, error→Status mapping | integration test passes on temp dirs |
| WP5 | D (after WP4 stubs) | `aa-cli` commands of §10.1 (minus `render`), output formatting, exit codes | e2e script passes |
| WP6 | B | `fixtures/click-notes` + uv-based test helper; live script for libreoffice | e2e and live script pass |

### 13.3 Acceptance demo (also the e2e test, in order)

```
aad --home $T/home --runtime-dir $T/run &                      # prints {"socket": ...} on stderr
export AA_SOCKET=$T/run/engine.sock
aa engine status                                               # api_version aa.v1, isolation none / not-proven
aa target add-harness click-notes --repo fixtures/click-notes --python $VENV/bin/python --basis owner --note "test fixture"
aa discover click-notes                                        # 6 capabilities, all DISABLED[UNREVIEWED,...]
aa cap show click-notes.note.list                              # level UNKNOWN, proposal READ, signals listed
aa call click-notes.note.list -a project_path=$T/notes.json   # exit 3, CAPABILITY_DISABLED, record written
aa cap approve click-notes.note.list --read --operator-noninteractive --note "lists only"
aa call click-notes.note.list -a project_path=$T/notes.json   # exit 0, result JSON, record ALLOW/COMPLETED
aa call click-notes.note.list -a bogus=1                       # exit 2, REJECTED_INPUT, inspect_input_contract
aa cap approve click-notes.note.remove --read                  # refused: KEYWORD signals raise may_discard_data
aa call click-notes.broken                                     # (after approve) exit 1, UNREADABLE_OUTPUT/EXECUTION_FAILURE
touch-modify $VENV/bin/click-notes; aa call click-notes.note.list ...   # exit 3, TARGET_CHANGED
aa log -n 20 && aa log verify                                  # chain ok, N records, every attempt present
```

The live variant replaces the fixture with
`aa target add-harness libreoffice --repo /Users/nhanclie/CLI-Anything --python <venv>/bin/python ...`, then runs
`aa cap approve libreoffice.export.presets --read` and `aa call libreoffice.export.presets`.

### 13.4 Milestones after the slice (in order)

Each milestone is roughly one to two weeks of agent work. The second column maps them to `PLAN.md`'s M0-M4.

| # | Milestone | Content | PLAN.md mapping |
|---|---|---|---|
| M1 | Generic CLIs + ergonomics | `cli-help` discovery source (Cobra `__complete`, Click `_X_COMPLETE` for non-importable Click, Swift `--experimental-dump-help`, GNU `--help` parse fallback); `--version` banner identity; derived `aa run <slug> <cmd...> --flags` from schema; `aa` autostarts `aad` (flock-guarded); tonic-reflection + tonic-health; output schema sampled from COMPLETED results (`confidence: observed`) | M1 registry/adapter contract |
| M2 | Front doors from one IR | `aa mcp` (rmcp 3.8.0) + `aa mcp doctor`; `Render(SKILL_MD / MCP_TOOLS_JSON / CLI_REFERENCE)`; `skills-ref validate` in CI; catalog digest in `GetInfo`; families/scoping for large targets | M1 MCP skeleton |
| M3 | The write path | read-back oracle lane: per write capability, `oracle = {read capability id, JSON-pointer predicate}` (e.g. `note.add` ↔ `note.list`); sandbox run on a *copy* of `--project` files with before/after snapshot of owned paths (`observed ⊆ declared`), `--dry-run` where offered; seed-fails/end-passes admission (REA `rea-desktop-applications-and-agent-environments.md` distill 8); OBSERVED signals; WRITE approvals require L3 PASS; `NEEDS_CONFIRMATION` with `confirmation_id` bound to `args_digest`; idempotency keys; out-of-scope denylist by signer/publisher; mutation testing (blocking floor) on `policy` | M3 action records/confirm/allow-deny |
| M4 | More bindings, more platforms | `aa.adapter.v1` out-of-process protocol + bridge kit; first adapters: OpenAPI/HTTP and MCP-proxy; loopback TCP + token; Windows named pipes + Job Objects | M2 adapters |
| M5 | Distillation assist + drift | LLM proposal client (raise-only signals, descriptions, core-set selection) writing PROPOSAL signals; golden recordings + replay on version change; audit head anchoring | M4 verification lanes |
| M6 | Drive layer | `LIFETIME_SESSION`, `LAUNCH` tier, CLI-Anything REPL/session mode, a11y adapters, per-step identity re-check (REA `rea-desktop-applications-and-agent-environments.md` G1-G11) | M2/M3 |

---

## 14. What is deliberately deferred, and why

| Deferred | Why it is safe to defer | Seam left |
|---|---|---|
| Any enabled WRITE/DESTRUCTIVE capability | Without a read-back oracle we cannot claim an effect happened, and the research is unanimous on this gate (survey §6.1 rung 5; REA confirmed-write admission) | `DISABLED_REASON_NO_READ_BACK_ORACLE`, `oracle` field arrives in M3 |
| Confirmations / MCP elicitation | Nothing that needs one can be enabled yet | `DECISION_NEEDS_CONFIRMATION`, `ERROR_CODE_CONFIRMATION_REQUIRED` |
| Token auth on UDS | Adds nothing against same-uid (REA SECURITY.md) | `Principal` + interceptor already in place |
| TCP, TLS, Windows, multi-user | MVP is single-user Unix | transport isolated in `aa-engine/src/listen.rs` |
| Streaming RPCs (discover progress, catalog watch, events) | Discovery of a CLI takes seconds | add `WatchCatalog` / streaming `Discover` without breaking unary |
| Out-of-process adapters, bridge kit | One binding kind is enough to prove the loop | `Binding` trait, reserved `oneof` slots, draft `aa.adapter.v1` |
| Generic `--help`/Cobra discovery | Click covers CLI-Anything plus any Click CLI; `--help` parsing is the lowest-fidelity tier and needs an LLM to be good | `DiscoverySource` trait; `manual` source covers the gap |
| LLM synthesis | Every LLM output must be fenced by deterministic gates that do not exist yet | `SIGNAL_SOURCE_PROPOSAL` |
| Sandboxing / containers / VMs | Large; the honest `isolation: NONE` plus READ-only enablement bounds the blast radius | `Isolation` enum, `isolation_proof` |
| REA-grade verified process cleanup (token sweep, PID-reuse defence) | One-shot children in their own group cover the MVP; REA's kernel is ~3k lines | `CleanupStatus.INCOMPLETE`, `retry_cleanup` recovery |
| Retain-then-view paging, schema advertisement profiles, TOON | Catalogs are small; outputs are capped and fail with `RESOURCE_CONSTRAINT` | `page_token` fields, `RESOURCE_CONSTRAINT` |
| Harness installation (`aa harness install`) | Executing registry install strings is the supply-chain hole we must not copy (`installer.py:67-93`) | `install_cmd_recorded` field |
| Sessions / REPL / stateful lifetime | Biggest design surface; one-shot plus `--project` files cover CLI-Anything usefully | `LIFETIME_SESSION`, `AUTHORITY_TIER_LAUNCH` |
| Kill switch / rate limits | Nothing mutating is enabled; deadlines bound each call | policy module is the insertion point |

---

## 15. Risks

1. **Proto-as-IR coupling.** pbjson plus tonic-prost-build in one build script is the least-trodden part of the
   stack. Proto3 also has no "required", so validation lives in L0. *Mitigation:* do WP1 first and keep a serde-mirror
   fallback for `Manifest`. `buf breaking` governs IR evolution.
2. **Fail-closed friction undercuts "EASY".** Every capability starts disabled, and the slice has no automated way to
   lower effects. *Mitigation:* one-command batch review (`aa cap approve` over the proposal list in M1), fast READ
   approvals, and an M3 sandbox lane that produces OBSERVED evidence automatically. This is the core tension of the
   whole project; the MVP resolves it toward safety.
3. **Approvals are attributable, not unforgeable,** under same-uid agents. *Mitigation:* stated as a non-claim. The
   real fix is OS-principal separation (agent runs as a different uid; `survey-safety-rails-and-authorization-models.md`
   §9), which this design already supports because the engine checks peer uid. A future policy can require that
   approvals come from a different uid than invocations.
4. **Click introspection executes harness code at import time** (sys.path manipulation, heavy imports, possibly
   network). *Mitigation:* INTROSPECT authority recorded, scrubbed env, private cwd, deadline. A `--help`-recursion
   fallback lands in M1.
5. **CLI-Anything output contracts are inconsistent across ~69 harnesses.** Some commands may lack `handle_error`.
   Auto-save writes `--project` files. Some harnesses silently use substitutes for the real software (the GIMP harness
   falls back to Pillow, `survey-agent-interface-generators.md` §2.5). *Mitigation:* outcome classification instead of
   trust, stdin null, leaf-only capabilities, `requires`/backend recorded as a limitation. Per-harness acceptance goes
   through the live lane.
6. **Weak identity for Python targets.** A console-script hash says little, and editable installs escape RECORD.
   *Mitigation:* `IdentityStrength.PARTIAL` with limitations, RECORD digest, and interpreter hash. Full source-tree
   hashing is optional in M1.
7. **SQLite in an async server** (blocking, single writer). *Mitigation:* one writer thread with a channel; reads use
   a separate connection in WAL mode. Volume is tiny.
8. **Socket lifecycle edge cases** (macOS path length, `/tmp` reaping, stale sockets after a crash). *Mitigation:*
   flock-before-unlink, short runtime dir, `aa` reports exit 4 with remediation.
9. **The scope is "ANY software", but the MVP serves CLIs only for weeks.** *Mitigation:* the reserved `Binding`
   slots and adapter draft keep the IR stable. M4 is scheduled, not aspirational.
10. **rmcp 3.x churn and MCP host constraints** (tool-name regexes, schema dialect limits). *Mitigation:* MCP stays a
    thin client in M2; the engine owns names/annotations; compact schema profiles come later if needed.

---

## 16. Borrowed patterns index

| Pattern | Source | Used in |
|---|---|---|
| Effects vector → derived annotations, never hand-set | REA `src/contracts/toolEffects.ts:2-22,765-787`; `rea-contracts-and-evidence-model.md` §1 | `Effects`, §3.1 |
| Split target effects from session/audit effects | `rea-tool-design-philosophy-and-verification.md` §1.6, §7.4 | `Effects` (no `mutates_session`) |
| Absence = unknown = ask; flags only raise; max over signals | usage spec via `survey-capability-discovery-and-distillation.md` §3.2, §4.3 | `Tri`, §3.1, §8 |
| Discovery authority levels static < introspect < probe < explore | `survey-capability-discovery-and-distillation.md` §1.3, distill 2 | `DiscoveryAuthority` |
| Untrusted target content marked by default | REA `toolEffects.ts:788-816`, `toolResultEncoding.ts:30-49` | `TargetText`, `untrusted_content`, MCP `_meta` |
| Closed error envelope with recovery kinds; malformed ≠ unresolvable | REA `src/contracts/errorSchemas.ts:19-101`; `rea-tool-design-philosophy-and-verification.md` §3.9 | `Error`, §3.2 |
| Content-addressed evidence; identity excludes display metadata | REA `src/domain/evidence.ts:87-110,232-254` | `record_id`, digests, §11 |
| Controlled-replay record as the action-record shape | REA `src/domain/process/processCapture.ts:216-286`; `rea-contracts-and-evidence-model.md` "action record" | `ActionRecord` |
| Withheld-with-reason capability states | REA `docs/agent-environments.md:175-186` | `DisabledReason` |
| Owned one-shot child: deadline, bounded output, env token last, own group | REA `src/process/OwnedCommand.ts:163-210`, `ProviderProcess.ts:277-319`, `BoundedOutputExcerpt.ts` | `runner.rs` §7.1 |
| Private runtime root, short socket path, 0600 socket | REA `PrivateRuntimeRoot.ts:30-97`, `GhidraTransport.ts:42-60`; `rea-engine-bridges-and-process-supervision.md` §8.2-8.3 | §4 |
| Isolation claims carry proof status | REA `src/process/WindowsAuthority.ts:8-33` | `EngineInfo.isolation_proof` |
| Catalog digest + expected-digest stale check | REA `src/catalogIdentity.ts:43-56`; `rea-capability-registry-and-composition.md` distill 6 | `catalog_digest`, `expected_manifest_digest` |
| Set-equality conformance tests over snapshots | `rea-capability-registry-and-composition.md` §10 | §12 |
| Discover → record raw → replay as fixtures | `rea-adding-a-new-target-walkthrough.md` distill 4 | `harvest()`/`candidates()` split |
| Exit-code contract 0/1/2/3, canonical JSON input with `--input-json`, client-side path resolution | `rea-cli-surface-and-agent-packaging.md` §10.1-10.2; REA `src/cliJsonInput.ts` | §10.1 |
| Real backend required; installed entry point tested; never trust exit 0 | CLI-Anything `HARNESS.md:150-214`; `survey-agent-interface-generators.md` §2.5 | L1 lane, live lane, runner limitations |
| Don't execute registry shell strings | counter-example `cli-hub/cli_hub/installer.py:67-93` | `install_cmd_recorded` |
| Curated core + `describe`/`invoke` meta tools | `survey-capability-discovery-and-distillation.md` §4.4 | M2 MCP |
| Authority basis logged per target; DRM/anti-cheat/security-control exclusion | `survey-safety-rails-and-authorization-models.md` §10 | `AuthorityGrant`, M3 denylist |

---

## 17. Open questions for the judge and synthesis

1. Is proto-as-IR acceptable to the other proposals, or will the synthesis want a language-neutral JSON Schema IR
   with proto only on the wire? This proposal's answer: proto for both, and the cost is coupling to generated types.
2. Should READ approvals be allowed from the same uid at all, or should the MVP already require a second OS
   principal for approvals? This proposal says same uid with attribution now, and an uid split as an M3 policy option.
3. Should Click-introspection be the default even for CLI-Anything harnesses whose SKILL.md is richer? The SKILL.md is
   prose without types, so this proposal uses it only as a rendered reference, never as a source of truth.
4. Are one-shot `--project` file sessions enough for CLI-Anything users until M6, or do agents need the REPL session
   model earlier?
