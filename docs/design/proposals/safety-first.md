# Proposal: Safety/Authority-First architecture for agentic-anything

Status: independent proposal (one of three), 2026-11-05. Lens: **safety and authority first**.
Hard constraints honoured: the core is a Rust gRPC engine (tonic + prost); the `aa` CLI, an MCP server and generated
Agent Skills are thin clients of it; HKUDS CLI-Anything harnesses (owner's fork at `/Users/nhanclie/CLI-Anything`,
Apache-2.0) are one binding source among several.

Citation keys used below (all under `docs/research/`):
`[SAFE]` survey-safety-rails-and-authorization-models, `[REG]` rea-capability-registry-and-composition,
`[CLI]` rea-cli-surface-and-agent-packaging, `[WALK]` rea-adding-a-new-target-walkthrough,
`[SUP]` rea-engine-bridges-and-process-supervision, `[TDV]` rea-tool-design-philosophy-and-verification,
`[DESK]` rea-desktop-applications-and-agent-environments, `[GEN]` survey-agent-interface-generators,
`[DISC]` survey-capability-discovery-and-distillation, `[EVD]` rea-contracts-and-evidence-model,
`[MCP]` rea-mcp-server-adapter. REA paths (`src/...`) are relative to `/Users/nhanclie/rea` (HEAD `3484432fb`);
CLI-Anything paths are relative to `/Users/nhanclie/CLI-Anything` (HEAD `34f5195`).

Evidence legend for claims I made in this session: **CHECKED** = I read the file or queried crates.io / docs.rs today;
**INFERRED** = design judgement; **UNVERIFIED** = assumption the first slice must confirm.

---

## 0. Thesis

**The engine is the single policy enforcement point, and nothing gets authority by default.** Every effectful thing
the system does goes through one gate inside `aad` (the engine daemon), no matter which front-end asked: agent calls
from the CLI or MCP, and also the engine's *own* pipeline work (introspecting a harness, probing a binary, installing
a Python env). The gate always runs the same steps in the same order:

```
authenticate principal (socket + peer creds + token)
  → resolve capability (by id + digest) → admission state (default WITHHELD)
  → effective authority = max(derived-from-effects, declared); UNKNOWN effect ⇒ assumed TRUE
  → kill switch / circuit breaker / rate limit
  → re-verify target identity (+ generation)
  → policy: hard exclusions → deny → ask → allow; no match ⇒ deny
  → (ask) park a PendingConfirmation bound to an action digest; return it in-band
  → write the intent to the hash-chained audit log
  → dispatch to an out-of-process, supervised binding
  → re-observe (oracle / state diff) → ActionRecord(outcome) → audit
```

Five opinions make this concrete:

1. **Authority is data, derived mechanically, and it can only rise.** The effects vector is REA's `ToolEffects`
   (`src/contracts/toolEffects.ts:1-22`, CHECKED) widened for driving, but every flag is **tri-state**. Proto3's zero
   default is `TRI_UNKNOWN`, so *a missing field means unknown, and unknown means "assume the worst"*. That is the
   wire-level form of `[DISC]` §4.3 rule 1 ("max over signals; absence = unknown; unknown = ask"). Authority
   (`observe < launch < attach < input < mutate < destructive`) is a projection of the vector. A declaration, an LLM
   proposal or harvested vendor text can raise it but never lower it. Lowering takes sandbox evidence or a signed-off
   human attestation (`[DISC]` §4.3 rules 3-4; `[TDV]` §7.3).
2. **Default-deny at three layers.** (a) *Emission*: a synthesized capability is born `ADMISSION_WITHHELD` (zero
   value). Observe-class capabilities are admitted after the deterministic lanes pass. Mutating ones stay withheld
   until a read-back oracle has been verified (`[DISC]` distill 6; `[DESK]` §8.3 confirmed-write admission).
   Destructive ones are never auto-admitted. (b) *Policy*: no matching rule means deny. (c) *API*: every gRPC method
   carries a `(aa.common.v1.rpc_policy)` option naming the roles allowed to call it. A method without one is
   unreachable, and a test enforces this.
3. **Confirmation authority comes from the channel, not from a flag.** The engine listens on two sockets.
   `agent.sock` can invoke but can never approve. `operator.sock` can approve, halt and grant. This applies the
   confused-deputy/POLA argument from `[SAFE]` §3.4: designation and authority travel together, so an injected agent
   cannot approve its own action by passing `confirm: true`. There are no permission booleans in any input schema
   (`[TDV]` rubric R8). MCP elicitation is a third, *host-mediated* channel with a lower recorded assurance, and by
   default policy it may not approve destructive actions.
4. **Port REA's process-ownership kernel; never signal what you can't prove you own.** Every binding that runs target
   code runs out of process under `aa-supervise`. That crate is a Rust port of `src/process/` (`[SUP]` §2): run token
   applied last, own process group, identity→token→identity revalidation, fail-closed `incomplete` stops that can be
   retried. The engine process itself runs no third-party code.
5. **Every act leaves evidence that can't be quietly rewritten.** Every decision (allow, deny, confirm, halt, grant,
   policy change, admission change) and every action is an entry in an append-only, hash-chained audit log.
   Intent is written *before* dispatch and outcome after, so a crash leaves a detectable "intent without outcome" ⇒
   `EXECUTION_STATE_UNKNOWN` (REA Cutter receipts, `[SUP]` §3.3). The engine **boots halted** if the chain tail fails
   to verify.

What this proposal does not claim: on a single OS account with the agent holding a shell, this is **not a sandbox**.
REA states the same about itself (`SECURITY.md:31-35`, CHECKED). §5.4 defines assurance tiers, and every
ActionRecord states which tier was in force. This follows REA's `WindowsAuthority` "proof status, never assert"
pattern (`[SAFE]` §4, `src/process/WindowsAuthority.ts:8-33`).

---

## 1. Cargo workspace layout

The workspace has 18 crates. Dependencies flow inward, as in REA (`domain → contracts → providers → application →
adapters`, `[REG]` §5 `scripts/lib/module-boundaries.mjs`). The pure crates (`aa-core`, `aa-policy`,
`aa-characterize`, `aa-manifest`) must not depend on tokio, tonic or any I/O. A `cargo deny`-style ban list in CI
enforces this, mirroring REA's module-boundary lint.

```
agentic-anything/
├─ Cargo.toml                 # [workspace], resolver = "3", edition 2024, rust-version = "1.99"
├─ proto/                     # buf module; source of truth for wire AND on-disk IR
│  ├─ buf.yaml  buf.gen.yaml  # lint: DEFAULT minus ENUM_ZERO_VALUE_SUFFIX (we use safe zero values deliberately)
│  └─ aa/{common,manifest,engine,binding}/v1/*.proto
├─ crates/
│  ├─ aa-proto/          generated types; descriptor set; pbjson serde; RpcPolicy extension reader
│  ├─ aa-core/           Tri, Effects, Authority, derive_authority(), annotations(), CapabilityId, TargetIdentity,
│  │                     JCS canonical digest, ev_/act_/cap_ id schemes. PURE.
│  ├─ aa-policy/         policy TOML model, evaluator (exclusion→deny→ask→allow, default deny), grants, scopes. PURE.
│  ├─ aa-characterize/   effect-signal table as code, raise-only combiners, oracle pairing. PURE.
│  ├─ aa-manifest/       capability IR assembly (synthesize), overlays, admission computation, catalog digest. PURE.
│  ├─ aa-audit/          hash-chained JSONL log, ActionRecord/Evidence builders, redaction, chain verify, CAS blobs
│  ├─ aa-supervise/      REA src/process port: spawn_owned, ownership proof, verified cleanup, stop state machine,
│  │                     deadlines, bounded output, private runtime roots, leases, rlimits, token redaction
│  ├─ aa-store/          on-disk layout + SQLite index (rebuildable from audit + manifests)
│  ├─ aa-bind/           Binding trait + registry; built-in cli_argv driver; out-of-process aa.binding.v1 client
│  ├─ aa-discover/       DiscoveryAdapter trait; adapters: cli_anything (first), later cobra/argparser/sdef/openapi/mcp
│  ├─ aa-verify/         deterministic lanes L0-L4 (lint, parser conformance, state diff, read-back oracle)
│  ├─ aa-gate/           THE policy enforcement point: admission, identity re-verify, policy, confirmations,
│  │                     kill switch, circuit breaker, rate limits, dispatch, record
│  ├─ aa-render/         renderers from IR+admission: MCP tool descriptors, CLI command spec, SKILL.md, server.json
│  ├─ aa-engine/         bin `aad`: tonic server on agent.sock/operator.sock, peer-cred + role interceptor, services
│  ├─ aa-client/         shared async client (UDS connector, token, typed helpers, exit-code mapping)
│  ├─ aa-cli/            bin `aa`: clap; static meta-commands + dynamic per-capability commands from catalog
│  ├─ aa-mcp/            bin `aa-mcp`: rmcp stdio server; agent-socket client; elicitation relay
│  └─ aa-testkit/        fixture Click harness, fake binding, temp-engine harness, tamper helpers (dev-only)
└─ shims/python/aa_click_introspect.py   # embedded via include_str!, written into a private root per run
```

### 1.1 Key dependencies (versions read with `cargo search` on 2026-11-05, CHECKED)

| Crate | Version | Used by | Why |
|---|---|---|---|
| `tonic` | 0.14.9 | engine, client, bind | gRPC server/client; UDS via `serve_with_incoming` + `UdsConnectInfo` peer creds |
| `prost` / `prost-types` | 0.14.4 | proto | messages, `Timestamp`, `Duration` |
| `tonic-prost` | 0.14.9 | proto | prost codec for tonic 0.14 (the codec split out in 0.14) |
| `tonic-prost-build` | 0.14.9 | proto (build) | `configure().file_descriptor_set_path(..).compile_protos(..)` (docs.rs, CHECKED) |
| `prost-reflect` | 0.16.5 | proto | read the custom `rpc_policy` MethodOptions from the descriptor set at startup |
| `pbjson` / `pbjson-build` | 0.9.0 | proto | proto3-JSON serde for manifests/audit on disk (UNVERIFIED: pbjson 0.9 ↔ prost 0.14 compatibility; fallback is prost-reflect's serde) |
| `tonic-health` / `tonic-reflection` | 0.14.9 | engine | health; reflection **on operator.sock only** (it leaks the catalog shape) |
| `tokio` | 1.54.1 | runtime crates | `net::UnixListener`, `process`, `time`, `signal` |
| `tokio-stream` / `tokio-util` | 0.1.19 / 0.7.20 | engine | `UnixListenerStream`, `CancellationToken` (kill switch fan-out) |
| `tower` / `hyper-util` | 0.5.3 / 0.1.21 | client | UDS connector (`service_fn` + `TokioIo`) |
| `rmcp` | 3.8.0 | aa-mcp | features `server`, `transport-io`, `elicitation`, `schemars`; docs reference MCP spec 2026-07-28 (docs.rs, CHECKED) |
| `clap` | 4.6.7 | aa-cli | builder API for dynamic commands built from the catalog |
| `serde` / `serde_json` | 1.0.229 / 1.0.154 | all | |
| `serde_jcs` | 0.2.0 | core, audit | RFC 8785 canonical JSON for digests (REA `canonicalDigest.ts`, `[EVD]` §3) |
| `sha2` | 0.11.1 | core, audit, supervise | SHA-256 for every content id (interop with REA `ev_` ids) |
| `hmac` | 0.13.0 | audit | keyed commitment of argument values (no plain hashes of secrets) |
| `subtle` | 2.6.1 | engine, bind | constant-time token compare (REA `compare_digest`, `[SUP]` §3.1) |
| `ed25519-dalek` | 3.0.0 | audit | signed chain checkpoints (`aa audit anchor`) |
| `rand` | 0.10.3 | supervise, engine | 32-byte tokens, run ids |
| `ulid` / `uuid` | 3.0.0 / 1.29.0 | core | `act_<ulid>` action ids (sortable), `run_id` UUIDs |
| `rusqlite` | 0.41.1 (`bundled`) | store | sync, single writer thread; chosen over `sqlx` 0.9.0 (async/compile-time checks are unnecessary here and add surface) |
| `nix` | 0.31.3 | supervise | `killpg`, `getpgid`, `setrlimit`, signals |
| `libc` | 0.2.x (stable; crates.io latest shows `1.0.0-alpha.5`, which is not to be used) | supervise | macOS `sysctl(KERN_PROCARGS2)`, `proc_pidinfo`, `LOCAL_PEERPID` |
| `tempfile` | 3.27.0 | supervise | owner-only `mkdtemp` private roots (`[SUP]` §2.6) |
| `fs4` | 1.1.0 | audit, store | advisory lock on the audit writer |
| `directories` | 6.0.0 | store | XDG / Application Support paths |
| `toml` | 1.2.0 | policy | policy + overlays (human-edited) |
| `globset` / `regex` | 0.4.21 / 1.13.1 | policy, characterize | capability-id globs; verb wording lint |
| `jsonschema` | 0.60.2 | gate, verify | canonical input validation (always on, regardless of what is advertised; `[TDV]` §1.3) |
| `schemars` | 1.2.3 | mcp | rmcp tool schema plumbing |
| `notify` | 9.0.0-rc.7 | engine | watch the `HALT` sentinel file. The RC is acceptable *only* because a poll fallback exists; pin exactly |
| `thiserror` / `anyhow` | 2.0.21 / 1.0.104 | libs / bins | |
| `tracing` / `tracing-subscriber` | 0.1.44 / 0.3.23 | all | diagnostics (never the audit log) |
| `jiff` | 0.2.39 | core | timestamps |
| dev: `proptest` 1.12.0, `insta` 1.49.0, `assert_cmd` 2.2.2 | | tests | `cargo-mutants` and `cargo-fuzz` as tools |

Toolchain on this machine (CHECKED): rustc 1.99.1, cargo 1.99.1, libprotoc 36.1, buf 1.75.0, `uv`, and
`/opt/homebrew/bin/soffice` (for the LibreOffice live lane).

---

## 2. The authority model

### 2.1 Effects vector (tri-state), extending REA's ten flags

REA's flags (`src/contracts/toolEffects.ts:2-22`, CHECKED) are kept with the same meanings. The additions are the
drive-plane flags proposed in `[SAFE]` §9, `[REG]` distill 3 and `[EVD]` "Borrow directly 4". One split matters:
**target-facing vs session-facing.** REA's merged `mutatesSession` makes 109 of 234 tools non-read-only purely because
they record Evidence (`[TDV]` §1.6). Here `mutates_session` never affects `readOnlyHint` or authority.

| Flag | Meaning | UNKNOWN treated as |
|---|---|---|
| `mutates_target` | changes target application/document state | TRUE |
| `writes_filesystem` | writes any file (incl. temp outside a private root) | TRUE |
| `launches_process` | starts a process (incl. the target CLI itself) | TRUE |
| `accesses_network` | opens a non-loopback connection | TRUE |
| `changes_ui_state` | focus, window, selection, navigation | TRUE |
| `sends_input` | synthetic keyboard/mouse/AX action | TRUE |
| `attaches_process` | attaches to a process the engine did not start | TRUE |
| `executes_foreign_code` | runs code the engine does not own (REA definition, `toolEffects.ts:11-17`) | TRUE |
| `may_discard_data` | overwrite/clear/drop unrecoverable state | TRUE |
| `mutates_device` | changes an attached device | TRUE |
| `external_communication` | sends data to another party (mail, upload, publish, post) | TRUE |
| `reads_screen` / `reads_clipboard` | privacy-sensitive observation | TRUE |
| `writes_clipboard` | cross-app data flow (exfil path) | TRUE |
| `reads_credentials` | touches keychains, tokens, password fields | TRUE |
| `irreversible` | cannot be undone by the app's own undo or by restore | TRUE |
| `idempotent` | safe to repeat | **FALSE** (the conservative direction is inverted) |
| `mutates_session` | engine-side state only (evidence, sessions) | TRUE, but excluded from authority and readOnlyHint |

### 2.2 Authority ladder (projection, `aa-core::derive_authority`)

The rules are evaluated top-down and the first match wins. The value used for each flag is its conservative
resolution (UNKNOWN → TRUE, except `idempotent`).

| Authority | Derived when any of |
|---|---|
| `DESTRUCTIVE` | `may_discard_data`, `irreversible`, `executes_foreign_code`, `mutates_device`, `external_communication`, `reads_credentials` |
| `MUTATE` | `mutates_target`, `writes_filesystem`, `writes_clipboard`, `accesses_network` (open-world: assume it can change remote state until an overlay scopes it to loopback, with evidence) |
| `INPUT` | `sends_input`, `changes_ui_state` |
| `ATTACH` | `attaches_process` |
| `LAUNCH` | `launches_process` |
| `OBSERVE` | none of the above (`reads_screen`/`reads_clipboard` stay OBSERVE but match privacy policy rules) |

`effective_authority = max(derive_authority(effects), declared_authority)`. `AUTHORITY_UNSPECIFIED` (zero) is treated
as `DESTRUCTIVE` everywhere. REA's rule "foreign code or device change ⇒ destructive" is kept (`toolEffects.ts:779-783`,
CHECKED). One deliberate departure: the ladder puts `ATTACH` below `INPUT`, as the owner ordered. In practice,
attaching a debugger or instrumentation sets `executes_foreign_code`, which makes it `DESTRUCTIVE` regardless
(`[SAFE]` §8: "inject/attach should be the highest tier").

### 2.3 MCP annotations (derived, `aa-render`)

The REA formula (`toolEffects.ts:764-786`) is adjusted for the target/session split:
- `readOnlyHint` = effective authority is OBSERVE, and every target-facing flag is `TRI_FALSE` (not UNKNOWN).
- `destructiveHint` = effective authority is DESTRUCTIVE.
- `idempotentHint` = `idempotent == TRI_TRUE`.
- `openWorldHint` = any of `launches_process`, `accesses_network` or `external_communication` is not `TRI_FALSE`.

Annotations are hints for hosts and are never inputs to the engine's own decisions (`[SAFE]` §3.3: "clients MUST
consider tool annotations to be untrusted").

### 2.4 Discovery levels map onto the same gate

`[DISC]` §1.3 found that discovery has side effects (`kubectl __complete` dialled an API server; Click/Cobra
completion runs program code). This proposal goes one step further: **pipeline steps are capabilities too**, with ids
under `aa.pipeline.*`, effects and authority, and they pass through the same gate.

| Discovery level | Example | Effective authority | Isolation requirement |
|---|---|---|---|
| `STATIC` | read `registry.json`, hash the harness tree, read SKILL.md | OBSERVE | none |
| `INTROSPECT` | import the Click app and call `to_info_dict()` (runs harness module top-level code) | LAUNCH (+ `executes_foreign_code` UNKNOWN ⇒ DESTRUCTIVE unless the target is operator-trusted) | scrubbed env, private HOME/TMPDIR, owned process |
| `PROBE` | run a subcommand with wrong-typed args to check the parser | MUTATE | private workspace; `Environment.isolation ≥ process`, recorded truthfully |
| `EXPLORE` | drive the UI, execute writes | DESTRUCTIVE | `container`/`virtual-machine` proof required, else refused with `ISOLATION_NOT_PROVEN` |
| (install) | create a venv, `uv pip install` a harness (runs `setup.py`) | DESTRUCTIVE (`executes_foreign_code`, `writes_filesystem`) | operator confirmation, every time |

The operator can mark a target `trusted_for_introspection`. This is recorded in the audit log with a reason. It lets
`INTROSPECT` run as LAUNCH with ask→allow for that target only. Otherwise first-time introspection asks.

### 2.5 Hard exclusions (not configurable by policy)

These are compiled into `aa-policy::exclusions` and evaluated before deny rules. Configuration can extend the
category list but cannot remove an entry from it (`[SAFE]` §10 "Operational guidance"):
- Targets whose identity matches a DRM, licence-check, anti-cheat, EDR or security-product category (by signer/bundle
  id/package) get no `attach`, `input` or `executes_foreign_code` capabilities, even with confirmation.
- Licence, registration and consent dialogs are never driven. Do not port REA's Hopper demo clicker
  (`scripts/hopper-demo-x11.py`, `[SUP]` scope note; `[DESK]` distill 13).
- Vendor hardening is a hard stop. If a target disables its automation surface (e.g. an Electron fuse), refuse and
  point to the supported surfaces (`[DESK]` §9, REA `docs/electron-observation.md:309`).
- Password managers and banking apps are denied by default for `input`, `reads_screen` and `reads_credentials`.
  This is a configurable default, not part of the hard list.

---

## 3. The gRPC API (draft `.proto`)

Design rules for the protos:
- **Safe zero values everywhere.** `TRI_UNKNOWN`, `AUTHORITY_UNSPECIFIED`(=destructive), `ADMISSION_WITHHELD`,
  `CONTENT_ORIGIN_TARGET` (untrusted) and `CONFIDENCE_INFERRED` are all zero. An omitted field therefore fails safe.
  buf lint's `ENUM_ZERO_VALUE_SUFFIX` is disabled for exactly this reason, and `buf breaking` runs in CI.
- **Results are in-band.** `confirmation_required` is a normal outcome, not an error. Errors inside a stream use the
  REA envelope (`[TDV]` §3.9). RPC-level refusals (wrong role, halted, malformed) use a gRPC `Status` whose `details`
  bytes are an encoded `aa.common.v1.Error`.
- **Every RPC declares its policy** through a custom method option. The engine builds its role table from the
  descriptor set at startup. A method without the option is unreachable.

### 3.1 `aa/common/v1/common.proto`

```proto
syntax = "proto3";
package aa.common.v1;

import "google/protobuf/descriptor.proto";
import "google/protobuf/timestamp.proto";

// ---------- RPC-level policy (enforced by the engine interceptor) ----------
enum Role {
  ROLE_UNSPECIFIED = 0;   // never granted; a method listing only this is unreachable
  ROLE_AGENT = 1;         // agent.sock: invoke, read catalog, open sessions, request grants
  ROLE_OPERATOR = 2;      // operator.sock: confirm, halt/unhalt, grants, policy, admission, export
}
message RpcPolicy {
  repeated Role roles = 1;            // empty ⇒ unreachable
  bool allowed_when_halted = 2;       // default false: refused while the kill switch is engaged
  bool audited = 3;                   // writes an audit entry for every call
}
extend google.protobuf.MethodOptions { RpcPolicy rpc_policy = 50001; }

// ---------- Tri-state and authority ----------
enum Tri { TRI_UNKNOWN = 0; TRI_FALSE = 1; TRI_TRUE = 2; }

enum Authority {
  AUTHORITY_UNSPECIFIED = 0;  // evaluated as DESTRUCTIVE
  AUTHORITY_OBSERVE = 1;
  AUTHORITY_LAUNCH = 2;
  AUTHORITY_ATTACH = 3;
  AUTHORITY_INPUT = 4;
  AUTHORITY_MUTATE = 5;
  AUTHORITY_DESTRUCTIVE = 6;
}

enum DiscoveryLevel {
  DISCOVERY_LEVEL_UNSPECIFIED = 0;  // evaluated as EXPLORE
  DISCOVERY_LEVEL_STATIC = 1;
  DISCOVERY_LEVEL_INTROSPECT = 2;
  DISCOVERY_LEVEL_PROBE = 3;
  DISCOVERY_LEVEL_EXPLORE = 4;
}

enum Confidence { CONFIDENCE_INFERRED = 0; CONFIDENCE_DERIVED = 1; CONFIDENCE_OBSERVED = 2; CONFIDENCE_DECLARED = 3; }

message EffectSignal {
  string source = 1;                 // "click-verb:remove", "harness-source:subprocess", "overlay:operator", "llm:proposal"
  Tri value = 2;
  Confidence confidence = 3;
  repeated string evidence_ids = 4;  // ev_… backing this signal
  bool lowers = 5;                   // true only for overlay/sandbox-evidence signals; gate rejects lowering signals without evidence or attestation
}
message EffectProvenance { repeated EffectSignal signals = 1; }

message Effects {
  Tri mutates_target = 1;
  Tri writes_filesystem = 2;
  Tri launches_process = 3;
  Tri accesses_network = 4;
  Tri changes_ui_state = 5;
  Tri sends_input = 6;
  Tri attaches_process = 7;
  Tri executes_foreign_code = 8;
  Tri may_discard_data = 9;
  Tri mutates_device = 10;
  Tri external_communication = 11;
  Tri reads_screen = 12;
  Tri reads_clipboard = 13;
  Tri writes_clipboard = 14;
  Tri reads_credentials = 15;
  Tri irreversible = 16;
  Tri idempotent = 17;       // UNKNOWN ⇒ FALSE
  Tri mutates_session = 18;  // engine-side only; excluded from authority/readOnlyHint
  map<string, EffectProvenance> provenance = 32;  // key = field name
}

// ---------- Content provenance (untrusted-content labelling) ----------
enum ContentOrigin {
  CONTENT_ORIGIN_TARGET = 0;    // default: target-originated ⇒ untrusted data, never instructions
  CONTENT_ORIGIN_ENGINE = 1;    // engine-authored
  CONTENT_ORIGIN_CALLER = 2;    // echoed caller input
  CONTENT_ORIGIN_OPERATOR = 3;  // operator-authored (overlays, reviewed summaries)
}
message TaintedText { string text = 1; ContentOrigin origin = 2; string source = 3; }  // e.g. source="click-docstring"
message Progress { uint64 seq = 1; string phase = 2; string message = 3; }

message Redaction { string pointer = 1; string reason = 2; }  // RFC 6901; reason: transport-credential|explicit-sensitive-value
message Payload {
  string json = 1;                 // inline, ≤ inline budget
  string blob_ref = 2;             // "sha256:<hex>" in the CAS when over budget
  uint64 bytes = 3;
  ContentOrigin origin = 4;
  repeated Redaction redactions = 5;
  string media_type = 6;
}

// ---------- Principals ----------
enum Channel { CHANNEL_UNSPECIFIED = 0; CHANNEL_AGENT_SOCKET = 1; CHANNEL_OPERATOR_SOCKET = 2; CHANNEL_LOOPBACK_TCP = 3; CHANNEL_ENGINE_INTERNAL = 4; }
message Principal {
  Role role = 1;
  Channel channel = 2;
  uint32 uid = 3;
  int32 pid = 4;                   // from LOCAL_PEERPID / SO_PEERCRED; 0 if unavailable
  string exe_path = 5;             // realpath of peer executable, best effort
  string process_start = 6;        // linux-starttime:<n> | darwin:<sec>:<usec>, defeats pid reuse
  string token_id = 7;             // id (not value) of the bearer token used
  string client_name = 8;          // self-asserted ("aa-cli", "aa-mcp")
  string client_version = 9;
  string mcp_client = 10;          // self-asserted MCP clientInfo, if relayed by aa-mcp
}

// ---------- Errors (REA envelope + drive codes; [TDV] §3.9, §7.7) ----------
enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  ERROR_CODE_INVALID_REQUEST = 1;
  ERROR_CODE_ACCESS_DENIED = 2;           // wrong role/channel/peer
  ERROR_CODE_POLICY_DENIED = 3;           // a deny rule or default deny
  ERROR_CODE_EXCLUDED = 4;                // hard exclusion
  ERROR_CODE_CONFIRMATION_DECLINED = 5;
  ERROR_CODE_CONFIRMATION_EXPIRED = 6;
  ERROR_CODE_ENGINE_HALTED = 7;
  ERROR_CODE_CIRCUIT_OPEN = 8;
  ERROR_CODE_RATE_LIMITED = 9;
  ERROR_CODE_CAPABILITY_WITHHELD = 10;    // not admitted; details carry WithheldReason[]
  ERROR_CODE_CAPABILITY_UNAVAILABLE = 11; // binding/backend missing
  ERROR_CODE_TARGET_UNAVAILABLE = 12;
  ERROR_CODE_TARGET_IDENTITY_CHANGED = 13;
  ERROR_CODE_TARGET_STATE_CHANGED = 14;   // generation moved since precondition/confirmation
  ERROR_CODE_ACTION_UNCONFIRMED = 15;     // dispatched, but post-condition not observed
  ERROR_CODE_EXECUTION_STATE_UNKNOWN = 16;// do not retry automatically; check receipt
  ERROR_CODE_ISOLATION_NOT_PROVEN = 17;
  ERROR_CODE_PROVIDER_TIMEOUT = 18;
  ERROR_CODE_RESOURCE_CONSTRAINT = 19;
  ERROR_CODE_CANCELLED = 20;
  ERROR_CODE_CLEANUP_INCOMPLETE = 21;
  ERROR_CODE_UNREADABLE_OUTPUT = 22;
  ERROR_CODE_EXECUTION_FAILURE = 23;
  ERROR_CODE_CONFIGURATION_INVALID = 24;
  ERROR_CODE_REVISION_CONFLICT = 25;      // expected digest/generation mismatch
  ERROR_CODE_TRUNCATED = 26;
}
enum ErrorCategory {
  ERROR_CATEGORY_UNSPECIFIED = 0; ERROR_CATEGORY_INVALID_INPUT = 1; ERROR_CATEGORY_AUTHORIZATION = 2;
  ERROR_CATEGORY_UNAVAILABLE = 3; ERROR_CATEGORY_INTEGRITY = 4; ERROR_CATEGORY_TIMEOUT = 5;
  ERROR_CATEGORY_CANCELLED = 6; ERROR_CATEGORY_RESOURCE = 7; ERROR_CATEGORY_EXECUTION = 8; ERROR_CATEGORY_SAFETY_STOP = 9;
}
message Remediation { string action = 1; }   // required; generated from the limits/messages registry
message Recovery {
  oneof kind {
    string inspect_input_contract = 1;        // operation
    InspectTimeout inspect_timeout_state = 2;
    string retry_cleanup = 3;                 // owner id
    string reobserve_target = 4;              // target id
    string await_confirmation = 5;            // confirmation id (human must act; agent must not loop)
    string check_receipt = 6;                 // action id
  }
}
message InspectTimeout { string operation = 1; uint64 timeout_ms = 2; }
message Error {
  ErrorCode code = 1;
  ErrorCategory category = 2;
  string message = 3;            // engine-authored; never echoes target text or secrets
  bool retryable = 4;
  Remediation remediation = 5;
  Recovery recovery = 6;
  string details_json = 7;       // typed per code (schema generated); e.g. withheld reasons, limit facts
}

// ---------- Isolation (REA WindowsAuthority proof pattern) ----------
enum Proof { PROOF_NOT_PROVEN = 0; PROOF_PROVEN = 1; PROOF_NOT_APPLICABLE = 2; }
enum Isolation { ISOLATION_NONE = 0; ISOLATION_PROCESS = 1; ISOLATION_CONTAINER = 2; ISOLATION_VIRTUAL_MACHINE = 3; }
message IsolationClaim { string property = 1; Proof proof = 2; string evidence = 3; } // e.g. "agent-uid-separated"
message Environment {
  string platform = 1; string arch = 2;
  Isolation isolation = 3;
  repeated IsolationClaim claims = 4;
}
```

### 3.2 `aa/manifest/v1/manifest.proto` (the capability IR, on wire and on disk)

```proto
syntax = "proto3";
package aa.manifest.v1;
import "aa/common/v1/common.proto";

enum TargetKind {
  TARGET_KIND_UNSPECIFIED = 0;
  TARGET_KIND_CLI_ANYTHING_HARNESS = 1;
  TARGET_KIND_INSTALLED_CLI = 2;
  TARGET_KIND_INSTALL_DIR = 3;       // [DESK] distill 1: four meanings of "app"
  TARGET_KIND_INSTALLED_APP = 4;
  TARGET_KIND_RUNNING_WINDOW = 5;
  TARGET_KIND_SERVICE_API = 6;
}

message FileIdentity { string realpath = 1; string sha256 = 2; uint64 size = 3; int64 mtime_ns = 4; uint64 dev = 5; uint64 ino = 6; }
message ProcessIdentity { int32 pid = 1; string start = 2; string exe_realpath = 3; string exe_sha256 = 4; }
message TargetIdentity {
  repeated FileIdentity files = 1;       // harness entry module, interpreter, backend binary (soffice)
  string package_name = 2;               // "cli-anything-libreoffice"
  string package_version = 3;            // "1.0.1"
  string source_tree_digest = 4;         // sha256 over sorted (relpath, sha256) of the harness tree
  string interpreter_lock_digest = 5;    // resolved venv packages + versions
  string backend_version_banner = 6;     // e.g. `soffice --version` first line
  ProcessIdentity process = 7;           // for running targets (later milestones)
  uint64 generation = 8;                 // bumps when identity-relevant state changes
  string identity_digest = 9;            // JCS digest of fields 1-7
}

message Target {
  string id = 1;                         // "cli-anything.libreoffice"
  TargetKind kind = 2;
  string display_name = 3;
  TargetIdentity identity = 4;
  repeated BackendRequirement backends = 5;
  Exclusion exclusion = 6;
  Provenance provenance = 7;
  aa.common.v1.TaintedText vendor_description = 8;  // from registry.json: untrusted
}
message BackendRequirement { string name = 1; string resolved_path = 2; aa.common.v1.Proof present = 3; aa.common.v1.TaintedText vendor_requires = 4; }
message Exclusion { bool excluded = 1; string category = 2; string rule_id = 3; }

message Provenance {
  string discovery_adapter = 1;          // "cli_anything@1"
  aa.common.v1.DiscoveryLevel level = 2;
  repeated string raw_digests = 3;       // verbatim harvested inputs in raw/
  repeated string evidence_ids = 4;
  string synthesized_by = 5;             // "deterministic" | "llm-proposal:<model>" (proposal stage only)
}

enum Lifetime { LIFETIME_UNSPECIFIED = 0; LIFETIME_ONE_SHOT = 1; LIFETIME_SESSION = 2; }

// ----- Bindings: how the engine reaches the capability (REA six shapes, [SUP] §8.1) -----
enum KillScope { KILL_SCOPE_NONE = 0; KILL_SCOPE_OWN_PID_ONLY = 1; KILL_SCOPE_OWNED_GROUP = 2; KILL_SCOPE_OWNED_JOB = 3; }
message Binding {
  KillScope kill_scope = 1;
  oneof shape {
    CliArgv cli_argv = 10;               // owned one-shot child (CLI-Anything, plain CLIs)
    GrpcPlugin grpc_plugin = 11;         // out-of-process aa.binding.v1 server
    ExternalMcp external_mcp = 12;       // loopback MCP server the engine does not launch
    PipeRepl pipe_repl = 13;             // later
  }
}
message CliArgv {
  string executable_file = 1;            // realpath; must equal a TargetIdentity.files entry
  repeated string argv_prefix = 2;       // ["-I", "-m", "cli_anything.libreoffice"]
  repeated string forced_global_args = 3;// ["--json"] — the engine always adds these
  repeated string command_path = 4;      // ["writer", "add-paragraph"]
  repeated ParamMapping params = 5;
  string session_state_flag = 6;         // "--project": supplied by the engine from the session, never by the agent
  string dry_run_flag = 7;               // "--dry-run"
  OutputContract output = 8;
  repeated string env_allowlist = 9;     // names only; values come from the engine's scrubbed env
  uint64 deadline_ms = 10;
  uint64 max_stdout_bytes = 11;
  uint64 max_stderr_bytes = 12;
}
message ParamMapping {
  string field = 1;                      // canonical input field (snake_case)
  oneof form { string flag = 2; uint32 positional_index = 3; }
  bool repeated_flag = 4;
  bool boolean_flag = 5;
  bool path_field = 6;                   // resolved + checked against session path scope before argv build
  PathAccess path_access = 7;
}
enum PathAccess { PATH_ACCESS_UNSPECIFIED = 0; PATH_ACCESS_READ = 1; PATH_ACCESS_WRITE = 2; }
message OutputContract {
  enum Format { FORMAT_UNSPECIFIED = 0; FORMAT_JSON_STDOUT = 1; FORMAT_TEXT_STDOUT = 2; }
  Format format = 1;
  string error_shape = 2;                // "cli-anything/v1": {"error","type"} + exit 1
  map<int32, aa.common.v1.ErrorCode> exit_codes = 3;
}
message GrpcPlugin { string plugin_id = 1; string executable_file = 2; repeated string argv = 3; string protocol = 4; /* "aa.binding.v1" */ }
message ExternalMcp { string endpoint = 1; /* loopback only */ string pinned_catalog_digest = 2; string upstream_tool = 3; }
message PipeRepl { string executable_file = 1; repeated string argv = 2; string framing = 3; }

// ----- Verification oracles ([DISC] §6.1 rung 5) -----
message OracleRef {
  oneof kind {
    string read_back_capability = 1;     // e.g. "libreoffice.writer.list"
    FileMagic file_magic = 2;            // CLI-Anything "Output Verification Methodology"
    string state_view = 3;               // typed state view id ([DESK] distill 7)
  }
  string predicate_json = 10;            // declarative, no regex ([DESK] distill 8)
}
message FileMagic { string path_field = 1; bytes magic = 2; string container = 3; /* "zip:mimetype" */ }

enum DryRunSemantics { DRY_RUN_NONE = 0; DRY_RUN_UNVERIFIED = 1; DRY_RUN_SKIPS_STATE_SAVE_ONLY = 2; DRY_RUN_NO_EFFECTS_VERIFIED = 3; }
message DryRun { string flag = 1; DryRunSemantics semantics = 2; }

// ----- Admission (computed by the engine; renderers only reflect it) -----
enum AdmissionState {
  ADMISSION_WITHHELD = 0;                // default for anything synthesized
  ADMISSION_ENABLED = 1;
  ADMISSION_ENABLED_CONFIRM_ALWAYS = 2;  // enabled, but every call asks regardless of grants
  ADMISSION_DISABLED_BY_OPERATOR = 3;
  ADMISSION_EXCLUDED = 4;
}
enum WithheldReason {
  WITHHELD_REASON_UNSPECIFIED = 0;
  WITHHELD_REASON_NOT_VERIFIED = 1;
  WITHHELD_REASON_UNKNOWN_EFFECTS = 2;          // any TRI_UNKNOWN on a capability that would otherwise be observe
  WITHHELD_REASON_NO_READ_BACK_ORACLE = 3;
  WITHHELD_REASON_ORACLE_UNVERIFIED = 4;
  WITHHELD_REASON_OBSERVED_EXCEEDS_DECLARED = 5;
  WITHHELD_REASON_TARGET_IDENTITY_DRIFT = 6;
  WITHHELD_REASON_SCHEMA_INVALID = 7;
  WITHHELD_REASON_BINDING_UNAVAILABLE = 8;
  WITHHELD_REASON_DESTRUCTIVE_REQUIRES_OPERATOR = 9;
  WITHHELD_REASON_HARD_EXCLUSION = 10;
}
message Admission {
  AdmissionState state = 1;
  repeated WithheldReason reasons = 2;
  string verification_report_digest = 3;
  string overlay_digest = 4;
  string attested_by = 5;                // operator principal summary when overridden
}

message CliProjection { string command = 1; repeated string positionals = 2; string omit_reason = 3; }
message SkillProjection { string use_when = 1; repeated string follow_up = 2; }
message Example { string title = 1; string input_json = 2; }

message Capability {
  string id = 1;                         // "libreoffice.writer.add_paragraph" (<app>.<noun>.<verb>, [DISC] §4.4)
  string target_id = 2;
  string title = 3;
  string summary = 4;                    // engine/operator-authored only (ContentOrigin ENGINE/OPERATOR)
  aa.common.v1.TaintedText vendor_help = 5;   // harvested docstring/help: untrusted
  string input_schema_json = 6;          // JSON Schema 2020-12, additionalProperties:false
  string output_schema_json = 7;
  aa.common.v1.Effects effects = 8;
  aa.common.v1.Authority declared_authority = 9;
  aa.common.v1.Authority effective_authority = 10;  // computed; never authored; recomputed on load
  Binding binding = 11;
  Lifetime lifetime = 12;
  repeated OracleRef oracles = 13;
  bool returns_target_content = 14;      // stored inverted in code: default TRUE unless audited ([REG] §7)
  DryRun dry_run = 15;
  repeated string families = 16;
  Provenance provenance = 17;
  CliProjection cli = 18;
  SkillProjection skill = 19;
  repeated Example examples = 20;
  Admission admission = 30;              // computed; excluded from digest
  string digest = 31;                    // sha256(JCS(fields 1-20)) — grants/confirmations bind to this
}

message Manifest {
  string format = 1;                     // "aa-manifest/1"
  Target target = 2;
  repeated Capability capabilities = 3;
  string catalog_digest = 4;             // over sorted capability digests + CLI names ([REG] §8.1)
}
```

### 3.3 `aa/engine/v1/engine.proto` (services)

Role options are abbreviated: `A` = `ROLE_AGENT`, `O` = `ROLE_OPERATOR`, `H` = `allowed_when_halted`.

```proto
syntax = "proto3";
package aa.engine.v1;
import "aa/common/v1/common.proto";
import "aa/manifest/v1/manifest.proto";
import "google/protobuf/timestamp.proto";
import "google/protobuf/duration.proto";

// ===== Identity / health =====
service EngineService {
  rpc GetIdentity(GetIdentityRequest) returns (EngineIdentity) {
    option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true };
  }
}
message GetIdentityRequest { string expected_catalog_digest = 1; string expected_engine_version = 2; }
message EngineIdentity {
  string engine_version = 1; string build_commit = 2;
  aa.common.v1.Principal you = 3;                 // whoami, as the engine sees the caller
  string policy_digest = 4;
  AuditHead audit_head = 5;
  HaltState halt = 6;
  aa.common.v1.Environment environment = 7;      // isolation claims with proof status
  AssuranceTier assurance_tier = 8;
  repeated ConfirmationChannel confirmation_channels = 9;
  string catalog_digest = 10;
  Alignment alignment = 11;                      // vs expected_* ([MCP] identity)
}
enum AssuranceTier { ASSURANCE_TIER_0_SAME_PRINCIPAL = 0; ASSURANCE_TIER_1_SEPARATED_AGENT = 1; ASSURANCE_TIER_2_ISOLATED_TARGET = 2; }
enum ConfirmationChannel { CONFIRMATION_CHANNEL_UNSPECIFIED = 0; CONFIRMATION_CHANNEL_OPERATOR_SOCKET = 1; CONFIRMATION_CHANNEL_MCP_ELICITATION = 2; }
message AuditHead { uint64 seq = 1; string entry_hash = 2; bool verified = 3; }
message Alignment { bool aligned = 1; repeated string reasons = 2; }

// ===== Catalog (read-only) =====
service CatalogService {
  rpc ListTargets(ListTargetsRequest) returns (ListTargetsResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc ListCapabilities(ListCapabilitiesRequest) returns (ListCapabilitiesResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc GetCapability(GetCapabilityRequest) returns (aa.manifest.v1.Capability) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc SearchCapabilities(SearchCapabilitiesRequest) returns (ListCapabilitiesResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
}
message ListTargetsRequest { uint32 limit = 1; string page_token = 2; }
message ListTargetsResponse { repeated aa.manifest.v1.Target targets = 1; string next_page_token = 2; }
message ListCapabilitiesRequest {
  string target_id = 1; repeated string families = 2;
  bool include_withheld = 3;             // agents see withheld caps only if asked, with reasons
  aa.common.v1.Authority max_authority = 4;
  uint32 limit = 5; string page_token = 6;
}
message ListCapabilitiesResponse { repeated aa.manifest.v1.Capability capabilities = 1; string next_page_token = 2; string catalog_digest = 3; }
message GetCapabilityRequest { string capability_id = 1; }
message SearchCapabilitiesRequest { string query = 1; string target_id = 2; uint32 limit = 3; }

// ===== Sessions =====
service SessionService {
  rpc OpenSession(OpenSessionRequest) returns (Session) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] audited: true }; }
  rpc GetSession(GetSessionRequest) returns (Session) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc CloseSession(CloseSessionRequest) returns (CloseSessionResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
}
message PathScope { repeated string read_roots = 1; repeated string write_roots = 2; repeated string deny = 3; } // Deno-style; deny wins
message OpenSessionRequest {
  string target_id = 1;
  string purpose = 2;                     // recorded authority basis ([SAFE] §10: "log the authority basis per session")
  aa.common.v1.Authority authority_ceiling = 3;   // a session can only narrow; UNSPECIFIED ⇒ OBSERVE
  PathScope requested_paths = 4;          // ⊆ policy-allowed roots, else POLICY_DENIED
  bool elicitation_relay = 5;             // set by aa-mcp when the MCP client advertised elicitation
}
message Session {
  string session_id = 1; string target_id = 2;
  aa.common.v1.Principal owner = 3;
  aa.common.v1.Authority authority_ceiling = 4;
  PathScope paths = 5;
  string workspace_root = 6;              // engine-owned private root (0700) holding the session state file
  bool tainted = 7;                       // has received CONTENT_ORIGIN_TARGET payloads
  SessionState state = 8;
  uint32 consecutive_denials = 9; uint32 total_denials = 10;
  repeated string active_grant_ids = 11;
}
enum SessionState { SESSION_STATE_UNSPECIFIED = 0; SESSION_STATE_OPEN = 1; SESSION_STATE_SUSPENDED = 2; SESSION_STATE_CLOSED = 3; SESSION_STATE_CLEANUP_PENDING = 4; }
message GetSessionRequest { string session_id = 1; }
message CloseSessionRequest { string session_id = 1; }
message CloseSessionResponse { Cleanup cleanup = 1; }

// ===== Actions: the only path to target effects =====
service ActionService {
  rpc Invoke(InvokeRequest) returns (stream ActionEvent) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] audited: true }; }
  rpc Resume(ResumeRequest) returns (stream ActionEvent) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] audited: true }; }
  rpc Cancel(CancelRequest) returns (CancelResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
  rpc GetAction(GetActionRequest) returns (ActionRecord) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc GetReceipt(GetActionRequest) returns (Receipt) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
}
message InvokeRequest {
  string session_id = 1;                  // optional for LIFETIME_ONE_SHOT observe caps
  string capability_id = 2;
  string expected_capability_digest = 3;  // if set and different ⇒ REVISION_CONFLICT (stale skill/catalog)
  string args_json = 4;                   // canonical input object; validated against the canonical schema
  repeated string sensitive_arg_pointers = 5;   // RFC 6901; committed as {sensitive:true, bytes:n}
  bool dry_run = 6;
  string idempotency_key = 7;             // required for non-idempotent caps on retry ([SAFE] §9 ACRFence)
  uint64 expected_generation = 8;
  google.protobuf.Duration wait_for_confirmation = 9;  // 0 ⇒ return pending immediately (CLI); ≤ TTL
  string stated_reason = 10;              // agent's reason: shown to the human labelled as agent-authored
}
message ResumeRequest { string confirmation_id = 1; string action_digest = 2; }
message CancelRequest { string action_id = 1; }
message CancelResponse { Outcome outcome = 1; }
message GetActionRequest { string action_id = 1; }

message ActionEvent {
  oneof event {
    Accepted accepted = 1;
    aa.common.v1.Progress progress = 2;
    PendingConfirmation confirmation_required = 3;
    ActionResult result = 4;
    ActionFailure failure = 5;
  }
}
message Accepted { string action_id = 1; aa.common.v1.Authority effective_authority = 2; Decision decision = 3; }
// Progress lives in aa.common.v1 (shared with aa.binding.v1): { uint64 seq; string phase; string message; }
// — REA progress-as-sequence, no fabricated percentages ([MCP]).
message ActionResult { ActionRecord record = 1; aa.common.v1.Payload result = 2; }
message ActionFailure { aa.common.v1.Error error = 1; ActionRecord record = 2; }

// ===== Confirmations =====
service ConfirmationService {
  rpc ListPending(ListPendingRequest) returns (ListPendingResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; } // agent: own only
  rpc WatchPending(WatchPendingRequest) returns (stream PendingConfirmation) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc Decide(DecideRequest) returns (DecideResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] audited: true }; }
  // Only for a session opened with elicitation_relay=true, only with the relay_nonce delivered on the
  // originating stream, and only if policy permits host-mediated assurance for that authority.
  rpc RelayElicitation(RelayElicitationRequest) returns (DecideResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT] audited: true }; }
}
enum ConfirmationKind { CONFIRMATION_KIND_UNSPECIFIED = 0; CONFIRMATION_KIND_ACTION = 1; CONFIRMATION_KIND_GRANT = 2; CONFIRMATION_KIND_PIPELINE = 3; }
message PendingConfirmation {
  string confirmation_id = 1;             // "cnf_<ulid>"
  ConfirmationKind kind = 2;
  string action_id = 3;
  string action_digest = 4;               // sha256(JCS{capability_digest, target identity_digest, generation, args commitment, session_id, policy_digest})
  string session_id = 5;
  aa.common.v1.Principal requested_by = 6;
  string capability_id = 7;
  aa.common.v1.Authority effective_authority = 8;
  aa.common.v1.Effects effects = 9;
  string target_summary = 10;             // ENGINE-authored from structured identity
  string args_rendering = 11;             // ENGINE-rendered; sensitive values masked; paths absolute
  aa.common.v1.Payload dry_run_preview = 12;   // if the capability has a verified dry run; origin TARGET
  aa.common.v1.TaintedText stated_reason = 13; // agent's reason, labelled
  bool session_tainted = 14;
  repeated ConfirmationChannel accepted_channels = 15;  // policy-derived; DESTRUCTIVE ⇒ operator socket only
  google.protobuf.Timestamp expires_at = 16;   // default TTL 120 s; single use
  string relay_nonce = 17;                // present only on the originating stream, never in ListPending
  repeated GrantOffer grant_offers = 18;  // scopes the human may choose ("this session", "15 min") — never broader than policy max
}
message GrantOffer { string offer_id = 1; string description = 2; google.protobuf.Duration ttl = 3; uint32 max_uses = 4; }
message ListPendingRequest { string session_id = 1; }
message ListPendingResponse { repeated PendingConfirmation pending = 1; }
message WatchPendingRequest {}
enum DecisionValue { DECISION_VALUE_UNSPECIFIED = 0; DECISION_VALUE_APPROVE = 1; DECISION_VALUE_DECLINE = 2; DECISION_VALUE_CANCEL = 3; }
message DecideRequest {
  string confirmation_id = 1;
  string action_digest = 2;               // must match: approvals cannot change args (no TOCTOU)
  DecisionValue decision = 3;             // UNSPECIFIED ⇒ decline
  string grant_offer_id = 4;              // optional: also mint a narrow grant
  string note = 5;
}
message RelayElicitationRequest {
  string confirmation_id = 1; string action_digest = 2; string relay_nonce = 3;
  DecisionValue decision = 4;             // MCP accept→APPROVE; decline→DECLINE; cancel→CANCEL
  string mcp_client = 5;                  // clientInfo asserted by aa-mcp
}
message DecideResponse { DecisionValue recorded = 1; string grant_id = 2; }

// ===== Kill switch =====
service ControlService {
  // Anyone may pull the brake; only the operator may release it.
  rpc Halt(HaltRequest) returns (HaltState) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
  rpc Unhalt(UnhaltRequest) returns (HaltState) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
  rpc GetHaltState(GetHaltStateRequest) returns (HaltState) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc WatchHalt(GetHaltStateRequest) returns (stream HaltState) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc ResumeSession(ResumeSessionRequest) returns (Session) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] audited: true }; }
}
enum HaltScope { HALT_SCOPE_ENGINE = 0; HALT_SCOPE_SESSION = 1; HALT_SCOPE_TARGET = 2; }  // zero = widest
message HaltRequest { HaltScope scope = 1; string scope_id = 2; string reason = 3; }
message UnhaltRequest { HaltScope scope = 1; string scope_id = 2; string reason = 3; string expected_halt_epoch = 4; }
message GetHaltStateRequest {}
message HaltState {
  bool engine_halted = 1; uint64 epoch = 2; string reason = 3;
  aa.common.v1.Principal by = 4; google.protobuf.Timestamp at = 5;
  repeated string halted_sessions = 6; repeated string halted_targets = 7;
  HaltSource source = 8;
}
enum HaltSource { HALT_SOURCE_UNSPECIFIED = 0; HALT_SOURCE_RPC = 1; HALT_SOURCE_SENTINEL_FILE = 2; HALT_SOURCE_SIGNAL = 3; HALT_SOURCE_AUDIT_VERIFY_FAILED = 4; HALT_SOURCE_BOOT_RESTORED = 5; }
message ResumeSessionRequest { string session_id = 1; string reason = 2; }

// ===== Policy, grants, admission (operator) =====
service PolicyService {
  rpc GetPolicy(GetPolicyRequest) returns (PolicyDocument) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc ValidatePolicy(PolicyDocument) returns (ValidatePolicyResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc ApplyPolicy(ApplyPolicyRequest) returns (PolicyDocument) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
  rpc RequestGrant(RequestGrantRequest) returns (PendingConfirmation) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT] audited: true }; }
  rpc CreateGrant(Grant) returns (Grant) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] audited: true }; }
  rpc RevokeGrant(RevokeGrantRequest) returns (Grant) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true audited: true }; } // revoking is always safe
  rpc ListGrants(ListGrantsRequest) returns (ListGrantsResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
}
message GetPolicyRequest {}
message PolicyDocument { string toml = 1; string digest = 2; }
message ValidatePolicyResponse { bool valid = 1; repeated string problems = 2; }
message ApplyPolicyRequest { PolicyDocument policy = 1; string expected_current_digest = 2; }
message Grant {
  string grant_id = 1;
  string principal_selector = 2;          // "session:<id>" | "client:aa-mcp" — never "*"
  string target_id = 3;
  string capability_glob = 4;             // "libreoffice.writer.*"
  aa.common.v1.Authority max_authority = 5;   // grants can never reach DESTRUCTIVE
  PathScope paths = 6;
  google.protobuf.Timestamp expires_at = 7;   // mandatory
  uint32 max_uses = 8; uint32 uses = 9;
  aa.common.v1.Principal granted_by = 10;
}
message RequestGrantRequest { string session_id = 1; Grant requested = 2; string stated_reason = 3; }
message RevokeGrantRequest { string grant_id = 1; string reason = 2; }
message ListGrantsRequest { string session_id = 1; }
message ListGrantsResponse { repeated Grant grants = 1; }

service AdmissionService {
  rpc GetAdmission(GetAdmissionRequest) returns (aa.manifest.v1.Admission) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  // Lowering an effect requires evidence ids from a verification lane OR an explicit attestation string.
  rpc ApplyOverlay(ApplyOverlayRequest) returns (ApplyOverlayResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] audited: true }; }
  rpc SetAdmission(SetAdmissionRequest) returns (aa.manifest.v1.Admission) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] audited: true }; }
}
message GetAdmissionRequest { string capability_id = 1; }
message ApplyOverlayRequest { string target_id = 1; string overlay_toml = 2; string expected_manifest_digest = 3; }
message ApplyOverlayResponse { aa.manifest.v1.Manifest manifest = 1; repeated string effect_changes = 2; }
message SetAdmissionRequest { string capability_id = 1; string capability_digest = 2; aa.manifest.v1.AdmissionState state = 3; string attestation = 4; }

// ===== Pipeline (discover → characterize → synthesize → verify → package) =====
service PipelineService {
  rpc StartJob(JobSpec) returns (Job) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] audited: true }; }  // gated as aa.pipeline.* capabilities
  rpc WatchJob(WatchJobRequest) returns (stream JobEvent) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc CancelJob(WatchJobRequest) returns (Job) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
}
message JobSpec {
  oneof spec {
    IngestCliAnything ingest_cli_anything = 1;
    DiscoverSpec discover = 2;
    VerifySpec verify = 3;
    PackageSpec package = 4;
  }
  aa.common.v1.DiscoveryLevel max_level = 10;   // ceiling; adapters above it are refused, not downgraded silently
  string session_id = 11;
}
message IngestCliAnything {
  string repo_root = 1;                   // /Users/nhanclie/CLI-Anything
  string harness = 2;                     // "libreoffice"
  string python = 3;                      // operator-provided interpreter with deps (slice); later: engine-managed env
  bool create_env = 4;                    // M3+: DESTRUCTIVE pipeline action, always confirmed
}
message DiscoverSpec { string target_ref = 1; repeated string adapters = 2; }
message VerifySpec { string target_id = 1; repeated string lanes = 2; bool require_live = 3; }
message PackageSpec { string target_id = 1; repeated string renderers = 2; string out_dir = 3; }
message Job { string job_id = 1; JobState state = 2; string action_id = 3; }
enum JobState { JOB_STATE_UNSPECIFIED = 0; JOB_STATE_PENDING_CONFIRMATION = 1; JOB_STATE_RUNNING = 2; JOB_STATE_SUCCEEDED = 3; JOB_STATE_FAILED = 4; JOB_STATE_CANCELLED = 5; }
message WatchJobRequest { string job_id = 1; }
message JobEvent {
  oneof event {
    aa.common.v1.Progress progress = 1;
    PendingConfirmation confirmation_required = 2;
    aa.manifest.v1.Manifest manifest = 3;
    VerificationReport report = 4;
    aa.common.v1.Error error = 5;
    Job finished = 6;
  }
}
message VerificationReport {
  string target_id = 1; string manifest_digest = 2; string digest = 3;
  repeated LaneResult lanes = 4;
  repeated string non_claims = 5;         // required: what this report does NOT establish ([TDV] distill 12)
}
message LaneResult {
  string lane = 1; string capability_id = 2;
  LaneStatus status = 3;
  repeated string evidence_ids = 4;
  aa.common.v1.Effects observed_effects = 5;
  string reason = 6;
}
enum LaneStatus { LANE_STATUS_NOT_RUN = 0; LANE_STATUS_PASS = 1; LANE_STATUS_FAIL = 2; LANE_STATUS_SKIPPED = 3; LANE_STATUS_UNKNOWN = 4; } // only PASS counts

// ===== Audit & evidence =====
service AuditService {
  rpc Query(AuditQuery) returns (AuditPage) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; } // agent: own sessions
  rpc Tail(AuditQuery) returns (stream AuditEntry) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc VerifyChain(VerifyChainRequest) returns (VerifyChainResponse) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc GetEvidence(GetEvidenceRequest) returns (Evidence) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_AGENT, ROLE_OPERATOR] allowed_when_halted: true }; }
  rpc Anchor(AnchorRequest) returns (Checkpoint) { option (aa.common.v1.rpc_policy) = { roles: [ROLE_OPERATOR] allowed_when_halted: true audited: true }; }
}
message AuditQuery { string session_id = 1; string action_id = 2; uint64 from_seq = 3; uint32 limit = 4; }
message AuditPage { repeated AuditEntry entries = 1; uint64 next_seq = 2; }
message VerifyChainRequest { uint64 from_seq = 1; uint64 to_seq = 2; }
message VerifyChainResponse { bool ok = 1; uint64 verified_through = 2; uint64 first_bad_seq = 3; string reason = 4; }
message GetEvidenceRequest { string evidence_id = 1; }
message AnchorRequest {}
message Checkpoint { uint64 seq = 1; string entry_hash = 2; bytes signature = 3; string public_key = 4; }

// ===== Records =====
message Subject { string target_id = 1; string identity_digest = 2; uint64 generation = 3; }
message ProviderRef { string id = 1; string version = 2; }
enum EvidenceAuthority { EVIDENCE_AUTHORITY_ANALYST_INFERENCE = 0; EVIDENCE_AUTHORITY_SHIPPED_ARTIFACT = 1; EVIDENCE_AUTHORITY_CONTROLLED_REPLAY = 2; EVIDENCE_AUTHORITY_LIVE_DRIVE = 3; EVIDENCE_AUTHORITY_EXTERNAL_SERVICE = 4; }
message Coverage { string scope = 1; CoverageStatus status = 2; }
enum CoverageStatus { COVERAGE_STATUS_UNKNOWN = 0; COVERAGE_STATUS_PARTIAL = 1; COVERAGE_STATUS_COMPLETE = 2; }  // missing coverage = unknown

message Evidence {                          // REA envelope ([EVD] §3), content-addressed
  string evidence_id = 1;                   // "ev_" + sha256(JCS(record without evidence_id))
  string predicate_type = 2;                // aa.observation | aa.action | aa.discovery | aa.verification | aa.state-diff
  Subject subject = 3;
  ProviderRef provider = 4;
  string operation = 5;
  string parameters_commitment = 6;         // HMAC; values never stored for sensitive pointers
  aa.common.v1.Payload result = 7;
  aa.common.v1.Confidence confidence = 8;
  EvidenceAuthority authority = 9;
  aa.common.v1.Environment environment = 10;
  repeated string limitations = 11;
  Coverage coverage = 12;
  repeated string links = 13;
  google.protobuf.Timestamp observed_at = 14;
}

message ArgsCommitment {
  string canonical_json_redacted = 1;       // sensitive pointers replaced by {"sensitive":true,"bytes":n}
  string hmac_sha256 = 2;                   // over the unredacted canonical args with the engine audit key
}
enum DecisionKind {
  DECISION_KIND_UNSPECIFIED = 0;
  DECISION_KIND_ALLOW_RULE = 1; DECISION_KIND_ALLOW_GRANT = 2; DECISION_KIND_CONFIRMED = 3;
  DECISION_KIND_DENIED_DEFAULT = 4; DECISION_KIND_DENIED_RULE = 5; DECISION_KIND_DENIED_EXCLUSION = 6;
  DECISION_KIND_DENIED_HALTED = 7; DECISION_KIND_DENIED_CIRCUIT = 8; DECISION_KIND_DENIED_RATE = 9;
  DECISION_KIND_DENIED_WITHHELD = 10; DECISION_KIND_DECLINED = 11; DECISION_KIND_EXPIRED = 12;
}
enum ConfirmationAssurance { CONFIRMATION_ASSURANCE_NONE = 0; CONFIRMATION_ASSURANCE_HOST_MEDIATED = 1; CONFIRMATION_ASSURANCE_OPERATOR_SAME_UID = 2; CONFIRMATION_ASSURANCE_OPERATOR_SEPARATED = 3; }
message Decision {
  DecisionKind kind = 1;
  string policy_digest = 2; string rule_id = 3; string grant_id = 4;
  string confirmation_id = 5;
  ConfirmationAssurance assurance = 6;
  aa.common.v1.Principal decided_by = 7;
  bool session_tainted = 8;
}
enum DispatchOutcome { DISPATCH_OUTCOME_NOT_DISPATCHED = 0; DISPATCH_OUTCOME_DISPATCHED = 1; DISPATCH_OUTCOME_TARGET_EXITED = 2; DISPATCH_OUTCOME_FAILED = 3; DISPATCH_OUTCOME_UNKNOWN = 4; }
message Dispatch {
  google.protobuf.Timestamp scheduled_at = 1; google.protobuf.Timestamp dispatched_at = 2; google.protobuf.Timestamp settled_at = 3;
  DispatchOutcome outcome = 4;
  string run_id = 5; int32 leader_pid = 6; int32 exit_code = 7; string signal = 8;
  repeated string argv_redacted = 9;         // exact argv with sensitive values masked
}
enum FileEffectKind { FILE_EFFECT_UNKNOWN = 0; FILE_EFFECT_UNCHANGED = 1; FILE_EFFECT_CREATED = 2; FILE_EFFECT_MODIFIED = 3; FILE_EFFECT_DELETED = 4; }
message FileEffect { string path = 1; FileEffectKind kind = 2; string before_sha256 = 3; string after_sha256 = 4; }
message ObservedEffects {
  repeated FileEffect files = 1;             // within observed roots only
  repeated string observed_roots = 2;        // what was actually watched (coverage)
  repeated aa.manifest.v1.ProcessIdentity processes = 3;    // children seen in the owned group
  repeated string unowned_process_notes = 4; // e.g. LaunchServices-spawned soffice: followed, never signalled
  bool exceeds_declared = 5;
}
enum Outcome {
  OUTCOME_UNSPECIFIED = 0;
  OUTCOME_COMPLETED = 1;                     // dispatched AND post-condition observed (or observe-only)
  OUTCOME_COMPLETED_UNVERIFIED = 2;          // dispatched, exit ok, no oracle ran ⇒ never "success" for mutating caps
  OUTCOME_ACTION_FAILED = 3;
  OUTCOME_CAPTURE_FAILED_AFTER_ACTION = 4;   // "failure does not prove absence of effects" (REA NativeUiObservation.ts:312-314)
  OUTCOME_DENIED = 5; OUTCOME_DECLINED = 6; OUTCOME_EXPIRED = 7;
  OUTCOME_CANCELLED = 8; OUTCOME_HALTED = 9;
  OUTCOME_EXECUTION_UNKNOWN = 10;            // intent without outcome after crash/poison
}
message OracleResult { string oracle = 1; LaneStatus status = 2; repeated string evidence_ids = 3; }
enum CleanupStatus { CLEANUP_STATUS_UNKNOWN = 0; CLEANUP_STATUS_VERIFIED = 1; CLEANUP_STATUS_ALREADY_EXITED = 2; CLEANUP_STATUS_NOT_OWNED = 3; CLEANUP_STATUS_INCOMPLETE = 4; }
message Cleanup { CleanupStatus status = 1; string reason = 2; repeated string retained_resources = 3; }
message ResidualUnknown { string scope = 1; string reason = 2; }

message ActionRecord {
  string action_id = 1;                      // "act_<ulid>"
  string session_id = 2;
  aa.common.v1.Principal requested_by = 3;
  string capability_id = 4;
  string capability_digest = 5;
  aa.manifest.v1.TargetIdentity target_identity = 6;   // as re-verified at dispatch
  aa.common.v1.Authority effective_authority = 7;
  aa.common.v1.Effects declared_effects = 8;
  ArgsCommitment args = 9;
  string action_digest = 10;
  Decision decision = 11;
  bool dry_run = 12;
  repeated string precondition_evidence_ids = 13;
  Dispatch dispatch = 14;
  repeated string postcondition_evidence_ids = 15;
  ObservedEffects observed_effects = 16;
  OracleResult oracle = 17;
  Outcome outcome = 18;
  Cleanup cleanup = 19;
  aa.common.v1.Environment environment = 20;  // isolation actually in force
  AssuranceTier assurance_tier = 21;
  repeated ResidualUnknown residual_unknowns = 22;
  repeated string limitations = 23;
  aa.common.v1.Error error = 24;
  string idempotency_key = 25;
}
message Receipt { string action_id = 1; Outcome outcome = 2; DispatchOutcome dispatch = 3; bool intent_logged = 4; bool outcome_logged = 5; }

// ===== Audit log entry (one JSONL line, JCS-canonical) =====
message AuditEntry {
  string format = 1;                         // "aa-audit/1"
  uint64 seq = 2;
  string prev_hash = 3;                      // entry_hash of seq-1; genesis = sha256("aa-audit/1:genesis:" + install_id)
  google.protobuf.Timestamp at = 4;
  oneof body {
    ActionRecord intent = 10;                // written BEFORE dispatch (outcome = UNSPECIFIED)
    ActionRecord action = 11;                // outcome
    PendingConfirmation confirmation_opened = 12;
    DecideRequest confirmation_decided = 13;
    HaltState halt = 14;
    PolicyDocument policy_applied = 15;      // digest + full text
    Grant grant = 16;
    SetAdmissionRequest admission = 17;
    ApplyOverlayRequest overlay = 18;
    Session session = 19;
    string engine_event = 20;                // boot, shutdown, chain-verify result
  }
  aa.common.v1.Principal principal = 30;
  string entry_hash = 31;                    // sha256(JCS(entry without entry_hash))
}
```

### 3.4 `aa/binding/v1/binding.proto` (out-of-process bindings; the engine is the client)

A binding gets no policy power. It receives only an invocation that is already validated, approved and scoped. It
cannot call the engine except by streaming events back. Its handshake is REA's identity contract (`[SUP]` §8.4).

```proto
syntax = "proto3";
package aa.binding.v1;
import "aa/common/v1/common.proto";

service BindingService {
  rpc Handshake(HandshakeRequest) returns (HandshakeResponse);
  rpc Invoke(BindingInvokeRequest) returns (stream BindingEvent);
  rpc Status(StatusRequest) returns (StatusResponse);     // must work while poisoned
  rpc Ack(AckRequest) returns (AckResponse);              // clears receipts
  rpc Shutdown(ShutdownRequest) returns (ShutdownResponse);
}
message HandshakeRequest { string run_id = 1; string protocol = 2; /* "aa.binding.v1" */ uint64 max_frame_bytes = 3; }
message HandshakeResponse {
  string run_id = 1; int32 process_id = 2; string binding_version = 3; string host_version = 4;
  string target_identity_digest = 5; uint64 generation = 6;
  repeated string capabilities = 7;          // exact set; engine compares set-equal, else teardown
  bool read_only = 8; uint64 max_frame_bytes = 9;
}
message BindingInvokeRequest {
  string action_id = 1; string request_id = 2;  // uuid for receipts
  string capability_id = 3; string args_json = 4;
  uint64 expected_generation = 5; uint64 deadline_ms = 6; bool dry_run = 7;
}
message BindingEvent {
  oneof event {
    aa.common.v1.Progress progress = 1;
    aa.common.v1.Payload observation = 2;     // origin forced to TARGET by the engine on receipt
    BindingResult result = 3;
    aa.common.v1.Error error = 4;
  }
}
message BindingResult { aa.common.v1.Payload payload = 1; uint64 generation_after = 2; }
message StatusRequest {}
message StatusResponse { bool poisoned = 1; string busy_request_id = 2; uint64 busy_ms = 3; repeated ReceiptEntry receipts = 4; }
message ReceiptEntry { string request_id = 1; ReceiptState state = 2; }
enum ReceiptState { RECEIPT_STATE_UNKNOWN = 0; RECEIPT_STATE_NOT_STARTED = 1; RECEIPT_STATE_COMPLETE = 2; RECEIPT_STATE_FAILED = 3; }
message AckRequest { repeated string request_ids = 1; }
message AckResponse {}
message ShutdownRequest {}
message ShutdownResponse { bool document_closed = 1; bool document_retained = 2; }
```

### 3.5 gRPC status mapping (RPC-level only)

| Condition | gRPC code | `Error.code` in details |
|---|---|---|
| Method not reachable for the caller's role/channel | `PERMISSION_DENIED` | `ACCESS_DENIED` |
| Engine halted and method not `allowed_when_halted` | `UNAVAILABLE` | `ENGINE_HALTED` (`retryable:false`, recovery: none; a human must unhalt) |
| Malformed request (proto-level) | `INVALID_ARGUMENT` | `INVALID_REQUEST` + `inspect_input_contract` |
| Expected digest mismatch | `FAILED_PRECONDITION` | `REVISION_CONFLICT` |
| Unknown capability/session/confirmation id | `NOT_FOUND` | `INVALID_REQUEST` (valid shape, unresolvable selector; no schema hint, `[TDV]` R16) |

Everything about an *action* (policy denial, withheld capability, execution failure, confirmation required) is in-band
in `ActionEvent`. A client never has to parse status strings to learn whether a human is needed.

---

## 4. Transport and local security

### 4.1 Sockets

- **Default transport: two Unix domain sockets**, and nothing listens on TCP.
  - Linux: `$XDG_RUNTIME_DIR/aa/{agent,operator}.sock`.
  - macOS: `$TMPDIR/aa/` (per-user, already private). If the path would exceed 100 bytes, fall back to
    `/tmp/aa-<uid>/`, created with `O_DIRECTORY|O_NOFOLLOW`, uid-checked and `fchmod 0700` through the fd. This is
    REA's lease-directory and short-socket-path handling (`[SUP]` §2.6-2.7; macOS `sockaddr_un` is 104 bytes).
  - Bind with `umask 077` first, then `chmod 0600`, then verify with `lstat` (owner, mode, socket type). Refuse to
    start on any mismatch.
- **The listening socket is also the single-instance lease** (REA `OwnedLease`, `[SUP]` §2.7). A stale socket is
  detected by `ECONNREFUSED` and recovered under a `.recovery` lock, so no PID file can go stale.
- **Peer credentials on every connection.** tonic's `UdsConnectInfo` exposes tokio `UCred` (uid, gid, pid). On macOS
  pid comes from `LOCAL_PEERPID`. The interceptor requires:
  - `operator.sock`: peer uid == engine owner uid, always.
  - `agent.sock`: peer uid ∈ `agent_uids` (default `[owner_uid]`; in Tier 1, the dedicated agent account).
  - Each connection is recorded as a `Principal` with exe realpath and process start identity. The process start
    identity is the REA PID-reuse defence (`[SUP]` §2.2).
- **Bearer tokens are defence in depth, not the boundary.** `state/tokens/{agent,operator}.token` are 32 random
  bytes, files 0600, sent as gRPC metadata `authorization: Bearer …` and compared in constant time. On one uid both
  files are readable to any same-uid process, so the real boundary is socket file permissions plus peer uid. The token
  binds a *role label* to a connection and is mandatory on TCP. Tokens never go in argv or env (`SECURITY.md`,
  `[SUP]` §1).
- **Loopback TCP is opt-in and agent-role only.** `aad serve --agent-tcp 127.0.0.1:0` exists for agents in a
  container that can't mount a UDS, and for a future Windows build. It requires a token, refuses non-loopback binds
  in v1, and *never* serves the operator role. Remote and multi-host serving is out of scope for v1 (§13).
- **Message limits:** a 4 MiB decode limit on the agent socket. Larger observations become CAS blob references
  (REA retain-then-view, `[EVD]` §4; overflow becomes `resource_constraint`, never truncation).
- **Reflection** (`tonic-reflection`) is served only on `operator.sock`.

### 4.2 Assurance tiers (stated honestly in every ActionRecord)

| Tier | Setup | What a confirmation proves | Default allowed channels |
|---|---|---|---|
| **0: same principal** (default dev setup) | agent, aa-mcp and human all run as one uid | A human *or* any same-uid process approved via operator.sock. It protects against model mistakes and injection steering the normal tool path. It does **not** protect against an agent with a shell that deliberately drives `aa confirm` | operator socket (assurance `OPERATOR_SAME_UID`); MCP elicitation for ≤ MUTATE (`HOST_MEDIATED`) |
| **1: separated agent** | agent host (Claude Code, Codex, etc.) runs as account `aa-agent`, or in a container/VM with only `agent.sock` reachable (0660, group `aa-agents`); `operator.sock` stays 0600 to the human | Only the human's uid could approve | operator socket (`OPERATOR_SEPARATED`); elicitation per policy |
| **2: isolated target** | Tier 1, plus the target runs in a container/VM lane (`Environment.isolation` proven) | Tier 1, and target effects are contained | as Tier 1; required by default for `EXPLORE` discovery |

The engine computes the tier from proofs (peer uid ≠ owner uid on agent connections, socket modes, isolation
claims). It never infers the tier from configuration alone (REA `WindowsAuthority` pattern). `GetIdentity` reports
the tier, `aa doctor` explains how to reach Tier 1, and the policy can require a tier per authority
(`require_tier = 1` for DESTRUCTIVE).

---

## 5. The gate (`aa-gate`): enforcement order and confirmation flow

### 5.1 Order (each step either passes or produces an in-band, audited refusal)

1. **Interceptor.** Check the role/channel against the method's `rpc_policy`, then halt state.
2. **Resolve.** Look up the capability by id, check `expected_capability_digest`, then admission. Anything but
   `ENABLED*` returns `CAPABILITY_WITHHELD` with its reasons.
3. **Authority.** Recompute `effective_authority` from the stored effects. Never trust the stored value: it is
   recomputed on load and compared, and a mismatch means corrupt state, so halt. Clamp to the session's
   `authority_ceiling` (above it ⇒ `POLICY_DENIED`).
4. **Validate.** Check args against the *canonical* input schema (`jsonschema`). Resolve `path_field`s to absolute
   paths and check them against the session `PathScope` (deny wins).
5. **Brakes.** Check the circuit breaker (session suspended after 3 consecutive or 20 total denials/declines; the
   Claude Code numbers, `[SAFE]` §3.1) and the rate limits (token buckets: MUTATE+ 30/min, launches 10/min, per
   session).
6. **Identity.** Re-verify the target identity: stat-tuple cache, re-hash on any change; mismatch ⇒
   `TARGET_IDENTITY_CHANGED`, plus re-admission drift (`[DISC]` §6.3). Check `expected_generation`.
7. **Policy.** Evaluate hard exclusions → deny → ask → allow; no match ⇒ deny. Matching is on structured descriptors
   (target id, capability glob, authority, individual effect flags, path scopes, `session_tainted`, assurance tier),
   never on command text (`[SAFE]` §7 "textual matching is not a boundary"). Grants can satisfy `ask` only up to
   `max_authority < DESTRUCTIVE`, and only when `admission != ENABLED_CONFIRM_ALWAYS`.
8. **Ask.** Mint a `PendingConfirmation` bound to `action_digest` (single use, TTL 120 s). Emit it on the stream,
   then either wait (`wait_for_confirmation`) or end the stream with the action parked.
9. **Intent.** Append the audit intent entry and fsync it (MUTATE and above).
10. **Dispatch.** Hand off to the binding under `aa-supervise` with the deadline, output budgets and the cancel token
    from the kill switch.
11. **Post-observe.** Re-run the read-back oracle or state diff, compute `ObservedEffects`, and check
    `exceeds_declared`. If it exceeds, fail the action, auto-withhold the capability (`OBSERVED_EXCEEDS_DECLARED`)
    and audit both.
12. **Record.** Write the `ActionRecord` outcome entry and return the result `Payload` with `origin = TARGET`.
    Set `session.tainted = true`.

### 5.2 Confirmation flow by front-end

```
agent (CLI/MCP) ──Invoke──▶ aad gate ──ask──▶ PendingConfirmation{id, action_digest, TTL}
     │                                   │
     │ CLI: exit 3 + JSON {confirmation_id, approve: "aa confirm <id>"}         (agent.sock)
     │ MCP: elicitation/create (if client supports) ──accept──▶ RelayElicitation (HOST_MEDIATED, ≤MUTATE)
     │      else: isError result {code: CONFIRMATION_REQUIRED-equivalent, recovery.await_confirmation}
     ▼
human ──`aa confirm <id>` / `aa watch`──▶ operator.sock Decide(id, action_digest, APPROVE)
     ▼
agent ──Resume(id, action_digest)──▶ gate re-runs steps 1-7 (halt, identity, generation, policy digest
                                     unchanged) ──▶ dispatch. Any change ⇒ TARGET_STATE_CHANGED / new confirmation.
```

Rules:
- **Approvals cannot edit.** `Decide` carries the `action_digest`. Args, target identity, generation, capability
  digest and policy digest are all inside the digest, so nothing can change between ask and act.
- **The prompt is engine-authored.** `target_summary` and `args_rendering` are rendered from structured fields. The
  agent's `stated_reason` is shown separately and labelled "agent-authored". Target-originated text (a dry-run
  preview) is labelled untrusted. Injected text therefore cannot write the question the human answers.
- **MCP decline and cancel both mean "do not act"** (`[SAFE]` §9). Elicitation never asks for secrets (MCP spec;
  `[SAFE]` §3.3). Credential entry is a human-takeover step outside the protocol (deferred).
- **No MCP tool can approve anything.** The MCP surface has no `confirm` tool. The fallback text tells the *user*
  to run `aa confirm <id>` in their terminal.
- **Grants are narrow, expiring and revocable** (Deno/POLA, `[SAFE]` §3.4). A human approving with a `GrantOffer`
  mints at most `session × capability-glob × max_authority<DESTRUCTIVE × ttl ≤ 1h × max_uses`. Agents can
  `RequestGrant` (it becomes a `CONFIRMATION_KIND_GRANT` pending item, matching REA's "narrow grant resumed,
  expansion reprompted, denial launched nothing" readiness stage, `[EVD]` §7) and can revoke grants but never mint
  them.

### 5.3 Kill switch

- **Sources:** `Halt` RPC from either socket ("anyone may pull the brake; only the operator releases"), `touch
  <state>/HALT` (watched with `notify`, plus a 1 s poll fallback), `SIGUSR1` to `aad`, and automatic halts on audit
  chain verification failure at boot or at runtime.
- **Effect** (fan-out through one `CancellationToken` tree):
  1. Admission closes: an atomic epoch is checked at gate entry, before dispatch and after a confirmation.
  2. In-flight actions are cancelled. The binding receives abort, and `aa-supervise` runs verified cleanup of
     *owned* groups only. Attached or LaunchServices-spawned processes are never signalled (REA "never kill what you
     cannot prove you own", `[MCP]`; `[DESK]` §11.6).
  3. Pending confirmations expire, and sessions are suspended.
  4. The halt is audited.
- Halt state is persisted, and **the engine boots halted if it shut down halted**. `Unhalt` requires the operator
  and the current `halt_epoch`, so a stale UI cannot unhalt a newer halt.
- CLI exit code 4. MCP returns `ENGINE_HALTED` with no retry hint.

### 5.4 Untrusted-content labelling and taint

- Every `Payload` defaults to `CONTENT_ORIGIN_TARGET`. aa-mcp maps it to `_meta.provenance.untrusted_content: true,
  source: "target"` and appends REA-style `UNTRUSTED_CONTENT_INSTRUCTIONS` to its server instructions
  (`toolResultEncoding.ts:29-30`, `[MCP]`). The CLI marks it on stderr (`[CLI]` §10.1).
- **Harvested text is tainted too.** This covers Click docstrings, `--help`, `registry.json` descriptions and
  CLI-Anything SKILL.md bodies. Tool descriptions are a prompt-injection channel (tool poisoning). The renderer
  builds MCP descriptions from an engine template plus at most 300 characters of vendor help inside an explicit
  `Vendor description (untrusted data): «…»` block. The full vendor text is available only through
  `GetCapability`/`aa describe`, labelled. Operator-reviewed summaries (`ContentOrigin OPERATOR`) replace the
  template once reviewed.
- **Session taint (CaMeL-lite).** Once a session has received target content, policy predicates can escalate:
  default rules ask (even under a grant) for `external_communication`, `executes_foreign_code`,
  `reads_credentials` and `writes_clipboard`. This is the targeted middle ground recommended in `[SAFE]` §8 and REA's
  host guidance "requiring approval for process-launching or device-mutating tools after target content was read"
  (`docs/mcp-contracts.md:523-526`, via `[EVD]`).
- Nothing harvested can lower an effect or grant authority (`[DISC]` §4.3 rule 5).

### 5.5 Default policy (`~/.config/aa/policy.toml`, operator-owned, refused if group/other-writable)

```toml
format = "aa-policy/1"
default = "deny"

rules = [
  { id = "observe-admitted",    effect = "allow", when = { authority = "observe", privacy = false } },
  { id = "privacy-reads",       effect = "ask",   when = { any_effect = ["reads_screen", "reads_clipboard"] } },
  { id = "launch-owned-cli",    effect = "allow", when = { authority = "launch", binding = "cli_argv", kill_scope = "owned_group" } },
  { id = "input-attach",        effect = "ask",   when = { authority = ["attach", "input"] } },
  { id = "mutate",              effect = "ask",   when = { authority = "mutate" }, channels = ["operator", "elicitation"], grantable = true },
  { id = "destructive",         effect = "ask",   when = { authority = "destructive" }, channels = ["operator"], grantable = false },
  { id = "tainted-exfil",       effect = "ask",   when = { session_tainted = true, any_effect = ["external_communication", "executes_foreign_code", "reads_credentials", "writes_clipboard"] }, grantable = false },
  { id = "pipeline-static",     effect = "allow", when = { capability = "aa.pipeline.discover.*", level = "static" } },
  { id = "pipeline-introspect", effect = "ask",   when = { capability = "aa.pipeline.discover.*", level = "introspect" }, channels = ["operator"] },
]

[paths]
write_roots = ["~/aa-workspaces"]     # sessions get private roots below this
deny        = ["~/.ssh", "~/.config/aa", "~/Library/Keychains", "~/.aws"]
```

The evaluator stays deny > ask > allow regardless of rule order: all matching rules are collected and the strictest
wins (Claude Code ordering, `[SAFE]` §3.1). The policy's digest is in every Decision.

---

## 6. Process ownership and supervision (`aa-supervise`, ported from REA `src/process/`)

The port follows REA's own recommendation to split it into small, separately tested units (`[SUP]` distill 3).

| Unit | REA source | Rust design |
|---|---|---|
| `spawn_owned` | `ProviderProcess.ts:254-341` | `tokio::process::Command` with `process_group(0)`, `env_clear()`, allowlisted env, `AA_RUN_ID` applied **last**, stdin `/dev/null` unless the protocol needs a pipe, no shell ever. Unenforceable limits are refused (`memory_limit_unsupported` on macOS) instead of ignored |
| ownership proof | `ProcessOwnership.ts:356-512`, `ProcessOwnershipObservation.ts:1245-1285` | Linux: `/proc/<pid>/stat` field 22 before and after reading `/proc/<pid>/environ`. **macOS: call `sysctl(KERN_PROCARGS2)` and `proc_pidinfo` directly through `libc`.** This removes REA's runtime Swift compile (`DarwinHelperCache.ts`, an open question in `[SUP]`). Hardened or platform processes that withhold env land in an `unverified` ledger and are never signalled |
| verified cleanup | `cleanupOwnedProcessGroup` | read table → plan → revalidate each group immediately before `killpg` (limit 2 revalidations) → `ESRCH` ok → token sweep until grace deadline |
| stop state machine | `ProviderProcessSupervisor.stop()` `:636-768` | `Verified / AlreadyExited / Terminated / Killed / NotOwned / Incomplete(reason)`. One shared in-flight stop; `Incomplete` clears the shared future so a later call retries; `stop_requested` is set before the first signal |
| deadlines | `ProviderDeadline.ts` | one absolute `Instant`; `remaining()` passed to each phase; `Cancelled` vs `Timeout` classification |
| bounded output | `BoundedOutputExcerpt.ts`, `BoundedControlLines.ts` | head+tail excerpt; `read_line(limit+1)`; overflow drained and reported, never retained |
| token redaction | `ProviderDiagnosticRedaction.ts:4-31` | redacts truncated prefixes too |
| private root | `PrivateRuntimeRoot.ts` | `tempfile::Builder` under a realpath'd parent (0700); close is idempotent and a failed removal is retried |
| lease | `OwnedLease.ts` | listening-UDS liveness lock (also the engine single-instance lock and the per-project write lock) |
| rlimits | `LinuxResourceLimits.ts` | `setrlimit` in `pre_exec` (AS, FSIZE, CPU, CORE=0); never raises inherited limits |
| poison | `[SUP]` distill 4 | a binding whose host can't cancel in-flight work is poisoned when its deadline expires ⇒ `EXECUTION_STATE_UNKNOWN`, receipts via `GetReceipt` |
| cleanup debt | `AnalysisClientCreation.ts:82-89` | while `Incomplete` cleanup is retained for a target, new sessions on that target are blocked with `retry_cleanup` recovery |

Kill scope is declared per binding (`KillScope`, `[SUP]` distill 7). The CLI-Anything binding is `OWNED_GROUP`. The
libreoffice harness's macOS fallback `open -W -n -a <bundle>` (`utils/lo_backend.py:135-161`, CHECKED) creates a
LaunchServices-owned soffice outside our group. The supervisor records it in `unowned_process_notes` and as a
residual unknown and never signals it. This is the same posture REA takes for Hopper's LaunchServices GUI
(`[SUP]` §3.1, `BridgeLauncher.ts:180-193`).

---

## 7. Adapter / binding model

- **In-process (inside `aad`):** only first-party Rust *drivers*, which are protocol clients that spawn or connect to
  something else:
  - `cli_argv`: an owned one-shot child, REA shape 5 (`[SUP]` §8.1).
  - the `aa.binding.v1` client;
  - later `external_mcp` (loopback only, no redirects, pinned `tools/list` profile, UTF-8 checked before parse, the
    REA IDA/JEB rules in `[SUP]` §3.7).

  No third-party code and no target code is loaded into the engine. The engine parses untrusted bytes only with
  bounded readers.
- **Out-of-process (everything that runs target-adjacent code):**
  - the Python Click introspection shim (one-shot);
  - the CLI-Anything harness itself (one-shot per action);
  - future persistent bridges: a Python bridge-kit for in-host scripting, a macOS AX helper ported from the
    `ReaNativeUI.swift` identity-binding recipe (`[DESK]` distill 2), UIA/AT-SPI.

  Persistent bridges speak `aa.binding.v1` on a UDS inside a per-run private root, with a token in a 0600 descriptor
  that is deleted after reading (`[SUP]` §8.2). Channel choice follows REA's rule: pipe if the engine is the parent
  and the host can do stdio; socket+bearer if the host owns its event loop; mutual HMAC for attach (`[SUP]` §8.1).
- **Binding output is re-validated by the engine**: against the output schema, the target identity and the byte
  budgets, as REA's `ElfNotesService` re-parses provider output (`[REG]` §11d, distill 12). For mutating
  capabilities the result is never "success" until the oracle has run (`OUTCOME_COMPLETED_UNVERIFIED` otherwise).
- **Static registry, no generic JSON dispatcher in the engine.** Built-in driver kinds are a closed Rust enum matched
  exhaustively. *Data-driven* capabilities (from manifests) are allowed only for driver kinds whose argument
  construction is fully schema-checked (`cli_argv` builds an argv vector, never a string). This answers `[WALK]` open
  question 2: typed drivers, data-driven capabilities.

---

## 8. Pipeline → crates → RPCs

| Stage | Crate | RPC / job | Safety-relevant rule |
|---|---|---|---|
| **Discover** | `aa-discover` | `PipelineService.StartJob{discover \| ingest_cli_anything}` | each adapter declares its `DiscoveryLevel`; the job's `max_level` is a ceiling; the step runs as an `aa.pipeline.discover.<adapter>` capability through the gate; raw inputs stored verbatim with digests in `raw/` (`[DISC]` §7) |
| **Characterize** | `aa-characterize` | inside the job; `AdmissionService.ApplyOverlay` for operator audits | effect-signal table as code; max-combine; UNKNOWN stays UNKNOWN; keyword and LLM signals can only raise; lowering requires `EffectSignal.lowers` + evidence ids or an attestation |
| **Synthesize** | `aa-manifest` | job output `Manifest` | deterministic naming `<app>.<noun>.<verb>`; closed schemas; `summary` engine-templated; every capability starts `ADMISSION_WITHHELD`; capability digest computed |
| **Verify** | `aa-verify` | `StartJob{verify}` → `VerificationReport` | lanes L0-L4 (§10); only `PASS` counts; `non_claims` required; admission computed from report + overlay + policy |
| **Package** | `aa-render` | `StartJob{package}`; also live via `CatalogService` | renderers read IR **and admission**: withheld capabilities are omitted from MCP and CLI help and listed in SKILL.md under "Withheld (reason)"; catalog digest stamped into every artefact |

LLM-assisted synthesis (descriptions, grouping, scenario drafting) is a later *proposal* stage. Its only outputs are
overlay proposals that an operator applies (`[DISC]` §5: "the manifest is the product").

---

## 9. Ingesting CLI-Anything harnesses

What a harness looks like (CHECKED in this session):
- **Catalogue.** `registry.json` holds `meta` plus 79 `clis` entries, each with `{name, display_name, version,
  description, requires, homepage, install_cmd, entry_point, skill_md, category, contributors}`.
- **The libreoffice harness** is an 836-line Click app (`libreoffice/agent-harness/cli_anything/libreoffice/libreoffice_cli.py`).
  The top-level group has global options `--json`, `--project <file>` and `--dry-run`, and with no subcommand it
  starts a REPL (`invoke_without_command=True`). It has 40 subcommands in the groups document, writer, calc, impress,
  style, export, session and repl. Errors are `{"error", "type"}` JSON with exit 1 (`handle_error`).
- **Auto-save side effect.** A `result_callback` **auto-saves the project file after any one-shot command that
  modified state, unless `--dry-run`** (`libreoffice_cli.py:140-152`).
- **`--dry-run` only skips the save.** `export render` still writes its output file.
- **The backend** shells out to `soffice` and on macOS may retry through LaunchServices.

### 9.1 Ingest job stages (`aa ingest cli-anything --repo … --harness libreoffice --python <venv>/bin/python`)

1. **STATIC (OBSERVE).**
   - Read the `registry.json` entry. Every string becomes `TaintedText`.
   - Record `install_cmd` but **never execute it**. Flag shell metacharacters; CLI-Hub runs these with `shell=True`
     (`[GEN]` §2.7).
   - Hash the harness tree into `source_tree_digest`.
   - Read SKILL.md as untrusted vendor docs.
   - Grep harness sources for `subprocess`, `shutil.which`, `os.system`, `requests`, `urllib`, `socket`, `smtplib`
     and `open(…, "w")`. These become *harness-level effect floors* (`launches_process`, `accesses_network`,
     `writes_filesystem` = TRUE with `source: harness-source:*`) applied to every command until per-command sandbox
     evidence lowers them.
2. **INTROSPECT (LAUNCH + foreign code ⇒ operator confirmation on first ingest).**
   - Run `<python> -I <private_root>/aa_click_introspect.py cli_anything.libreoffice.libreoffice_cli:cli` as an owned
     one-shot. Use a scrubbed env (`PATH` = venv bin + `/usr/bin:/bin`, `HOME`/`TMPDIR` = private root, no inherited
     tokens/proxies, `[DISC]` distill 2) with a 20 s deadline and a 4 MiB stdout cap.
   - The shim imports the module and prints `click.Context(cli).to_info_dict()`. This exists in Click ≥ 8.0, but
     whether it works on this harness is UNVERIFIED: my attempt to run it in the scratchpad was blocked in this
     session, so slice task T5 must prove it. The fallback is a static Python-AST parse in the shim (CLI-Anything's
     own `skill_generator.py:34-78` approach, `[GEN]` §2.6), which needs no module import and so is STATIC level.
   - Record the Evidence `aa.discovery` with `isolation: process` and the limitation "network not isolated on macOS".
3. **CHARACTERIZE.**
   - Map Click params to JSON Schema: STRING→string, INT→integer, FLOAT→number, flag→boolean, `Choice`→enum,
     `multiple`→array, `required` kept, defaults recorded. Arguments become positional `ParamMapping`s.
     `additionalProperties:false`.
   - Global options are handled specially. `--json` becomes a `forced_global_args` entry. `--project` becomes
     `session_state_flag`, supplied by the engine from the session's private workspace and never exposed to the
     agent. `--dry-run` becomes `DryRun{semantics: DRY_RUN_SKIPS_STATE_SAVE_ONLY}`, not trusted as a preview until
     verified.
   - Param names matching `path|output|file|dir` (plus Click `Path`/`File` types) become `path_field` with
     READ/WRITE access by verb.
   - Verb floors (raise-only): `info|list|get-*|status|history|profiles|presets|preset-info|import-formats|json` →
     candidate OBSERVE. `new|open|import|add-*|set-*|create|modify|apply|rename-*|save|undo|redo` → `mutates_target`
     and, with a project, `writes_filesystem`. `remove*` → `may_discard_data`. `render` → `writes_filesystem` +
     `launches_process`.
   - **Every command run with `--project` gets `writes_filesystem = UNKNOWN`** until the state-diff lane shows that
     auto-save didn't fire. This auto-save behaviour is exactly why read-only cannot be inferred from the verb.
   - `repl` and the bare group are **never emitted** (interactive, unbounded).
   - Oracle pairing by noun: `writer.add_*`/`set_text`/`remove` ↔ `writer.list`; `calc.set_cell` ↔ `calc.get_cell`;
     `calc.*_sheet` ↔ `calc.list_sheets`; `impress.*` ↔ `impress.list_slides`; `style.*` ↔ `style.list`;
     `export.render` ↔ `file_magic` (PDF `%PDF-`; ODF zip with `mimetype` entry, per CLI-Anything HARNESS "Output
     Verification Methodology", `[GEN]` §2.5); `document.new|open|import` ↔ `document.info`.
4. **SYNTHESIZE.**
   - Ids `libreoffice.writer.add_paragraph` etc. The binding is `CliArgv{executable_file: <python realpath>,
     argv_prefix: ["-I","-m","cli_anything.libreoffice"], forced_global_args: ["--json"], command_path: [...]}`
     (the package has `__main__.py`, CHECKED).
   - Every argv is built as a vector, with values that start with `-` passed as `--opt=value` and positionals after
     `--`.
   - `TargetIdentity.files` includes the interpreter, the entry module, the harness tree digest, `soffice`'s realpath
     and SHA-256, and the `soffice --version` banner. The GIMP harness's silent Pillow fallback
     (`gimp/agent-harness/cli_anything/gimp/core/export.py`, `[GEN]` §2.5) is why the backend is part of the
     identity, and why a missing backend makes the capability `BINDING_UNAVAILABLE` rather than letting the fallback
     run silently.
5. **VERIFY / PACKAGE**: see §10 and §11.

### 9.2 Runtime invocation of a harness capability

- **Session workspace.** The session gets a private workspace (0700) under `write_roots`, and the engine owns
  `<ws>/project.json`. A per-project lease serialises MUTATE actions, complementing CLI-Anything's own `flock`
  (`core/session.py:10-33`).
- **Child setup.** The child runs with `cwd = <ws>`, private `HOME` (so LibreOffice uses an isolated
  `UserInstallation`, matching what `lo_backend` already passes via `-env:UserInstallation`), stdin `/dev/null`, a
  deadline and output caps.
- **Exit-code mapping.**
  - Exit 0 with valid JSON ⇒ result `Payload` (origin TARGET).
  - Exit 1 with `{"error","type"}` ⇒ `EXECUTION_FAILURE` (type in details).
  - Non-JSON stdout ⇒ `UNREADABLE_OUTPUT`.
  - Exit 2 ⇒ `INVALID_REQUEST`. The engine's canonical validation should have caught it first, so this is logged as
    a schema drift signal.
- **State diff around every MUTATE call.** Snapshot `<ws>` (rel path → sha256) before and after the call. This
  yields `ObservedEffects`; anything outside the declared paths fails the action and withholds the capability.

### 9.3 What we do *not* take from CLI-Anything

- Their SKILL.md is not served (we render our own from the IR; theirs is untrusted input to a later LLM proposal
  stage).
- `install_cmd` is never run.
- `/cli-anything:validate` is an LLM prompt, not a program, and is not used as evidence (`[GEN]` §2.5).
- Their claimed test counts are not imported as verification. Only our lanes count.

---

## 10. Verification strategy

### 10.1 Lanes for generated capabilities (deterministic, never an LLM; `[TDV]` §6, `[DISC]` §6.1)

| Lane | Runs | Gates admission of |
|---|---|---|
| L0 static | IR lint: rubric R1-R11, R15-R16, R21-R22 (`[TDV]` §5); closed schemas; name grammar; wording-vs-effects regex extended with `click\|type\|press\|submit\|send\|delete\|save\|close\|quit\|render\|export\|remove` (port of `toolEffects.test.ts:16-170`); no permission booleans in inputs | all |
| L1 parser conformance | owned child in a private root: for each command, a wrong-typed value ⇒ expect Click usage error exit 2; unknown flag ⇒ exit 2; `--help` parse ok. Note: Click runs the group callback before parsing subcommand help, so this executes harness code. Runs at PROBE level | all |
| L2 contract (fake backend) | `aa-testkit` fixture Click harness + recorded replies (incl. refusals; `[WALK]` distill 4: recorded fixtures, not hand-written mocks) | binding logic |
| L3 state diff | seeded private workspace; run the capability; diff the workspace; `observed ⊆ declared`; for `--project` read commands, prove auto-save did not write | OBSERVE (lowers `writes_filesystem` UNKNOWN→FALSE with evidence) |
| L4 read-back oracle | seed → post-condition must **fail on the seed** and **pass after** the action (REA admission gate, `[DESK]` §6, distill 8); oracle via paired read capability or `file_magic` | MUTATE |
| L5 live backend | real `soffice` required; honest `skipped{reason}` that never counts as pass; `--require-tools` makes it mandatory | "live-verified" badge |

Admission rules: OBSERVE requires L0+L1+L3 pass and no UNKNOWN target-facing effects. MUTATE requires those plus
L4 pass. DESTRUCTIVE is never auto-admitted; it needs an operator `SetAdmission(ENABLED_CONFIRM_ALWAYS, attestation)`.
A target identity change sets every capability of that target back to `TARGET_IDENTITY_DRIFT` until the lanes
re-pass.

### 10.2 Engine test strategy (the safety core gets the strictest tests)

- **Property tests (`proptest`) in `aa-core` and `aa-policy`:**
  - For any effects vector, replacing any `TRI_FALSE` with `TRI_UNKNOWN` never lowers `derive_authority`.
  - Overlays without evidence never lower it.
  - deny > ask > allow holds regardless of rule order (shuffled rule order produces the same decision; REA's
    metamorphic catalog test idea, `[TDV]` R1).
  - Grants never yield DESTRUCTIVE.
- **Golden sets** (REA `toolEffects.test.ts:172-205` style): a frozen list of DESTRUCTIVE capabilities per ingested
  harness. Any change fails the test until it is reviewed.
- **RPC policy completeness test:** load the descriptor set and assert every method has a non-empty `rpc_policy`.
  A generated table test then calls every method from the agent socket and asserts `PERMISSION_DENIED` exactly where
  `ROLE_AGENT` is absent. It also asserts every non-`allowed_when_halted` method returns `ENGINE_HALTED` while
  halted.
- **Confirmation integrity:**
  - Decide with a wrong digest is refused.
  - Approval reuse is refused (single use).
  - An expired confirmation is refused.
  - Args, target or generation changing between Decide and Resume yields `TARGET_STATE_CHANGED`.
  - `RelayElicitation` without the stream nonce is refused.
  - `RelayElicitation` for DESTRUCTIVE is refused.
- **Audit:**
  - A byte flip at seq *n* makes `VerifyChain` report `first_bad_seq = n`.
  - Truncating the tail is detected against the last anchor.
  - Boot with a bad chain comes up halted.
  - A crash between intent and outcome (simulated with a kill -9 of `aad` in a test) yields
    `OUTCOME_EXECUTION_UNKNOWN` on restart.
- **Supervision conformance** (REA list, `[SUP]` §8.9): wrong token; wrong run_id; extra keys; oversize frame
  without LF; host hang beyond the deadline ⇒ poison; owner killed ⇒ bridge exits within its idle bound; simulated
  PID reuse ⇒ cleanup refuses; cleanup failure ⇒ `Incomplete` then successful retry.
- **Mutation testing** (`cargo-mutants`) on `aa-policy`, `aa-gate` and `aa-supervise`, with a **blocking score
  floor**. REA keeps its mutation score non-blocking; `[TDV]` distill 10 argues authz code deserves a blocking floor.
- **Fuzzing** (`cargo-fuzz`) of everything that parses target bytes: the introspection JSON, harness stdout JSON,
  the manifest/overlay TOML and the bounded line readers.
- **Live lanes** on this Mac: the libreoffice harness against `/opt/homebrew/bin/soffice`. Each is recorded as a
  dated verification record with SHAs and non-claims (`[TDV]` §4.9).

---

## 11. Front-ends on top of the engine

### 11.1 `aa` CLI (`aa-cli`, clap 4.6)

- **Static meta-commands:** `aa status` (identity, tier, halt, audit head), `aa targets`, `aa caps <target>
  [--all]`, `aa describe <cap>`, `aa session open|close`, `aa call <cap> [--input-json <inline|file|->]
  [--session S] [--dry-run]`, `aa resume <cnf>`, `aa receipt <act>`, `aa ingest cli-anything …`, `aa verify
  <target>`, `aa package <target>`, `aa audit verify|tail|anchor`, `aa halt [--reason]`, `aa doctor`.
- **Operator commands** use `operator.sock` and refuse on agent.sock: `aa confirm <cnf> [--grant <offer>]`,
  `aa decline <cnf>`, `aa watch` (live pending queue for a human terminal), `aa unhalt`, `aa admit`, `aa overlay
  apply`, `aa policy apply`, `aa grant create|revoke`.
- **Dynamic per-capability commands** (`aa libreoffice writer add-paragraph --text …`) are built with clap's builder
  API from `ListCapabilities` and cached by catalog digest. There is always an `--input-json` escape hatch
  (canonical object). Path fields are resolved against cwd before canonical validation (`[CLI]` §5, §10.2). Only
  admitted capabilities get subcommands.
- **Output**: the REA contract (`[CLI]` §3): `--json` compact when piped, JSONL with a trailing metadata record, and
  untrusted content flagged on stderr.
- **Exit codes** (one wrapper for every command; `[CLI]` distill 5 plus a split of "needs human" from "refused"):

| Code | Meaning | Agent should |
|---|---|---|
| 0 | result produced (completeness is in the payload `coverage`) | read the payload |
| 1 | operation failed (typed `Error` on stdout) | follow `remediation`/`recovery` |
| 2 | usage/validation error before any engine I/O | fix the input |
| **3** | **confirmation (or grant) required**: stdout `{confirmation_id, effective_authority, approve_hint:"ask the user to run: aa confirm cnf_…", resume:"aa resume cnf_…"}` | stop and ask the human; do not loop |
| **4** | **refused**: policy denial, hard exclusion, withheld, halted, circuit open | do not retry |
| 130/143 | signal, after cleanup settles | |
| 141 | EPIPE | |

### 11.2 MCP server (`aa-mcp`, rmcp 3.8 stdio)

- **Client of `agent.sock` only.** It holds no policy and no state beyond the session id per MCP connection.
- **`tools/list`** comes from `ListCapabilities` (admitted only): a curated core set per target plus three meta tools
  (`aa_describe_capabilities(query)`, `aa_invoke(capability_id, args)` subject to the same gate, `aa_status`), per
  `[DISC]` distill 9. Annotations are derived as in §2.3, and `requiresUserInteraction: true` is set on every
  DESTRUCTIVE tool (Claude Code honours it, `[SAFE]` §3.3). The catalog digest is checked via
  `expected_catalog_digest`.
- **Results**: `structuredContent` = ActionRecord + payload; `_meta.provenance.untrusted_content`; oversize results
  become a `resource_constraint` with a continuation (`[MCP]`).
- **Confirmation.** At `initialize`, read the client's `capabilities.elicitation` (as REA does at
  `src/server/mcpClientMetadata.ts:58-66`) and open the session with `elicitation_relay` set accordingly.
  - When `confirmation_required` arrives, send `elicitation/create` with a flat schema (`approve: boolean` plus an
    optional grant-offer enum) and the engine-authored message. Map accept to `RelayElicitation` and decline/cancel
    to decline.
  - If the client lacks elicitation, or the policy channel is operator-only, return `isError:true` with the
    confirmation id and `recovery.await_confirmation`. The text says *ask the user* to run `aa confirm <id>`; the
    agent then calls the same tool again with `_meta.aa_resume=<id>` (or `aa_invoke` with `resume`).
- **No HTTP transport in v1.** REA's HTTP MCP is unauthenticated and single-client (`[MCP]` Constraints).

### 11.3 Generated Agent Skills (`aa-render`)

- The skill is rendered into `skills/aa-<target>/SKILL.md` from the IR and admission state:
  - a route table ("when X → `libreoffice.writer.add_paragraph` / `aa libreoffice writer add-paragraph`") with an
    **authority column**;
  - a "Withheld capabilities (reason)" section;
  - a fixed safety section ("the engine enforces policy; this skill grants nothing; exit 3 means ask the human; never
    attempt to run `aa confirm` yourself").
- It is stamped with the catalog digest. Generated sections sit between markers and the hand-authored judgment sits
  outside them (`[CLI]` distill 11). The skill tells agents to trust the live catalog (`aa caps`) over the skill
  (REA `SKILL.md:26`). A stale skill naming a since-withheld capability simply gets `CAPABILITY_WITHHELD`.

---

## 12. State and persistence

```
~/.config/aa/                    (0700; refused if group/other-writable)
  policy.toml                    operator-owned; loaded only via PolicyService.ApplyPolicy (digest audited)
  engine.toml                    agent_uids, tiers, limits
~/.local/share/aa/  (macOS: ~/Library/Application Support/aa/)   (0700)
  targets/<target_id>/raw/<sha256>          verbatim harvested inputs (registry entry, info_dict, SKILL.md, help)
  targets/<target_id>/manifest/<digest>.json  proto3-JSON Manifest (pbjson), immutable, content-addressed
  targets/<target_id>/overlays/*.toml        operator effect audits / summaries / enables (append-only history)
  targets/<target_id>/verification/<digest>.json  VerificationReport
  envs/<target_id>-<lockdigest>/             engine-managed venvs (M3+)
  cas/sha256/ab/cdef…                         evidence payload blobs (screens, large outputs)
  audit/audit-000001.jsonl                    hash-chained, JCS lines; segment rollover carries prev_hash
  audit/anchors.jsonl                         ed25519-signed checkpoints {seq, entry_hash, sig}
  keys/audit.ed25519 , keys/args-hmac.key     0600
  tokens/{agent,operator}.token               0600
  state.db                                    SQLite (WAL) INDEX: targets, capabilities+admission, sessions,
                                              grants, pending confirmations, action index, halt state
  HALT                                        sentinel (presence = halt)
```

- **The audit log is the source of truth. SQLite is a rebuildable index** (`aa audit reindex`). Grants, admissions,
  policy applications and halts are all audit entries, so state can be replayed. Pending confirmations are
  deliberately *not* restored across restart: they expire, which fails safe.
- **Write discipline.** A single writer task serialises audit appends. `fs4` takes an exclusive lock on the active
  segment. fsync runs on every MUTATE+ intent/outcome and every policy/halt/grant entry; OBSERVE entries are
  batch-fsynced every 250 ms. `aa audit anchor` prints the signed head for the operator to copy elsewhere, for
  example into a git commit. Same-uid tamper-*evidence* only becomes tamper-*resistance* in Tier 1, when the
  log directory is owned by the engine account.
- **Redaction** follows REA (`[EVD]` §5): credentials and caller-declared sensitive pointers are never persisted. A
  sensitive value is committed as `{sensitive:true, bytes:n}` with only an HMAC over the full canonical args, never
  a plain hash of a secret. Engine errors never echo target text.

---

## 13. First vertical slice (one day, parallelisable across agents)

**Goal:** prove the safety spine end to end on one real harness. Ingest the CLI-Anything libreoffice harness, run an
admitted OBSERVE capability, then run a MUTATE capability through *exit 3 → operator confirm → resume*, with a
hash-chained audit, a working kill switch, and role separation over two UDS sockets. No MCP, no SQLite, no
verification lanes beyond L0 lint, and no Linux/Windows. macOS (this machine) first.

**Crates in the slice:** `aa-proto` (common, manifest, and the engine subset: EngineService, CatalogService,
SessionService, ActionService, ConfirmationService.Decide/ListPending, ControlService, AuditService.VerifyChain,
PipelineService with IngestCliAnything only), `aa-core`, `aa-policy` (built-in default policy, no TOML loader yet),
`aa-audit`, `aa-supervise` (lite: own pgid, run token, deadline, bounded output, TERM→KILL on the owned pgid,
private root; no cross-process ownership sweep), `aa-bind` (`cli_argv` only), `aa-discover` (cli_anything adapter),
`aa-characterize` (verb table + harness floors), `aa-manifest`, `aa-gate`, `aa-engine`, `aa-client`, `aa-cli`
(static commands + `aa call`), `aa-testkit` (fixture Click harness).

**Prerequisite (human, 2 min):** `uv venv ~/aa-envs/lo && uv pip install --python ~/aa-envs/lo/bin/python click
prompt-toolkit defusedxml`. The engine does not install anything in the slice.

**Task breakdown (parallel tracks):**

| # | Task | Done when |
|---|---|---|
| T1 | `proto/` + buf lint/breaking config + `aa-proto` build.rs (`tonic-prost-build`, descriptor set) | `cargo build -p aa-proto`; `buf lint` clean |
| T2 | `aa-core`: Tri, Effects, `derive_authority`, `annotations`, JCS digest, ids + proptests | proptest "unknown never lowers" passes |
| T3 | `aa-audit`: JSONL hash chain, append/verify, intent/outcome, tamper test | flip-a-byte test reports correct seq |
| T4 | `aa-supervise` lite + `aa-bind::cli_argv` (vector argv, scrubbed env, `--json` forced, exit/JSON mapping, workspace diff) | runs fixture harness; deadline kill test; LaunchServices note path stubbed |
| T5 | `shims/python/aa_click_introspect.py` + `aa-discover::cli_anything` (registry entry, tree digest, introspect via T4) | **validates the UNVERIFIED `to_info_dict()` assumption** on libreoffice; else AST fallback |
| T6 | `aa-characterize` + `aa-manifest`: Click→schema, verb floors, harness floors, `--project`/`--json`/`--dry-run` handling, ids, admission default WITHHELD, L0 lint subset | `libreoffice.document.profiles` & `document.import_formats` admitted OBSERVE (no project ⇒ no auto-save path, verbs observe, harness floors scoped to backend-calling modules—**or** withheld with reasons if floors apply; either outcome is correct and is recorded) |
| T7 | `aa-gate` + `aa-engine`: two sockets, peer-cred check, `rpc_policy` interceptor from descriptor, halt (RPC + `HALT` file), pending confirmations (in memory), Resume with digest check | RPC-policy completeness test + agent-socket-cannot-Decide test pass |
| T8 | `aa-cli`: status, ingest, caps, session, call, confirm (operator), resume, halt/unhalt, audit verify; exit codes 0/1/2/3/4 | acceptance script below passes |

**Acceptance script** (an `assert_cmd` integration test plus a manual run):

```sh
aad serve --state-dir "$T/aa" &                              # agent.sock + operator.sock, boots unhalted, empty chain
aa status --json                                             # tier 0, audit verified, policy digest
aa ingest cli-anything --repo /Users/nhanclie/CLI-Anything --harness libreoffice --python ~/aa-envs/lo/bin/python
#   → exit 3 (introspect = pipeline capability, operator channel)
aa confirm cnf_… && aa resume cnf_…                          # operator approves; manifest synthesized
aa caps libreoffice --all --json                             # ~38 caps; effects tri-state; admission + reasons
aa call libreoffice.document.profiles --json                 # exit 0 if admitted (else exit 4 CAPABILITY_WITHHELD, with reasons)
aa admit libreoffice.document.new --state enabled-confirm-always --attest "slice demo"   # operator override, audited
aa session open libreoffice --json                           # private workspace under ~/aa-workspaces
aa call libreoffice.document.new --session S --input-json '{"type":"writer","name":"r"}'   # exit 3
aa confirm cnf_…                                             # via operator.sock; agent.sock attempt → PERMISSION_DENIED
aa resume cnf_…                                              # exit 0; ActionRecord.observed_effects lists project.json CREATED;
                                                             #   outcome COMPLETED_UNVERIFIED (no oracle in slice) — honest
aa halt --reason drill ; aa call libreoffice.document.profiles   # exit 4 ENGINE_HALTED
aa unhalt --reason done
aa audit verify                                              # ok through seq N
# tamper: flip one byte in audit-000001.jsonl; restart aad → boots halted (HALT_SOURCE_AUDIT_VERIFY_FAILED)
```

**What the slice proves:**
- Default-deny emission.
- Unknown-effects conservatism.
- Confirmation bound to a digest, with role separation by socket.
- Pipeline steps gated like actions.
- Process ownership of the harness child.
- A truthful ActionRecord.
- Tamper-evident audit with a fail-closed boot.
- A kill switch.

**What it doesn't prove:** MCP/elicitation, read-back oracles, Linux/Windows supervision, Tier 1, real
`export render` with soffice.

### 13.1 Milestones after the slice (in order)

1. **M1: MCP + renderers.**
   - `aa-mcp` (rmcp 3.8, stdio) with derived annotations, untrusted `_meta`, elicitation relay plus operator-only
     fallback, `requiresUserInteraction`, and catalog-digest alignment.
   - `aa-render` SKILL.md and MCP descriptors.
   - `aa mcp doctor` (spawn self, exact inventory, digest; `[CLI]` distill 13).
2. **M2: verification and admission.**
   - Lanes L0-L5.
   - Oracle pairing and the seed-fails/end-passes gate.
   - Computed admission and `OBSERVED_EXCEEDS_DECLARED` auto-withhold.
   - `export render` live lane with `file_magic`.
   - Batch-ingest all 79 registry entries and publish an admission report (how many caps per harness reach
     OBSERVE/MUTATE). This is the honest measure of "easy".
3. **M3: full kernel and persistence.**
   - The full `aa-supervise` port (Linux procfs, macOS `KERN_PROCARGS2`, verified cleanup, leases, rlimits, cleanup
     debt).
   - SQLite index, TOML policy loader, grants, circuit breaker, rate limits, signed anchors.
   - Engine-managed harness envs (DESTRUCTIVE pipeline action, hash-pinned locks).
   - Restart semantics.
4. **M4: second and third binding sources.**
   - `aa.binding.v1` + Python bridge-kit.
   - `external_mcp` adapter (pinned catalog).
   - One more discovery source with high yield and low risk (Cobra `__complete` or Swift `--experimental-dump-help`,
     `[DISC]` distill 3).
   - PTY `pipe_repl` for CLI-Anything REPL-only harnesses.
5. **M5: Tier 1 + GUI input.**
   - A documented, `aa doctor`-verified separated-agent setup (dedicated account / container with agent.sock only).
   - macOS AX one-shot helper porting the `ReaNativeUI.swift` identity binding, behind INPUT authority, with
     per-step identity/generation re-verify and closed action vocabulary + role/name locators (`[DESK]` distill 2, 6).
6. **M6: evaluation.**
   - Agent-eval lane (REA gate shape: first tool, zero repeats keyed by target generation, negative routing;
     `[TDV]` §4.8).
   - Mutation-score floor enforced in CI.
   - Fuzzing in nightly CI.

---

## 14. Deferred, and why

| Deferred | Why |
|---|---|
| Windows (named pipes + DACL, Job Objects, UIA) | No Windows dev box or test lane here. The design keeps Windows seams (`KillScope::OwnedJob`, `CHANNEL_LOOPBACK_TCP`, isolation claims) so it isn't foreclosed |
| GUI/a11y driving, vision fallback | Highest-risk authority (`INPUT`). It needs Tier 1 and per-step identity binding first (M5). Vision is the last rung (`PLAN.md` technique ladder) |
| Dynamic instrumentation / attach | Own-software-only, last rung. The research report is partial. It would be DESTRUCTIVE (`executes_foreign_code`) under this model anyway |
| Remote / non-loopback / multi-user serving, TLS | The operator channel must not be network-reachable in v1. A remote design needs real authn (mTLS + device binding), which is out of scope |
| MCP HTTP transport | REA's is unauthenticated and single-client (`[MCP]`). stdio covers the target hosts |
| Code-mode MCP (`execute` scripts) | Breaks per-operation authority unless every host callback re-checks it (`[GEN]` distill 4); the action-log representation is unresolved (`[GEN]` open question 2) |
| LLM synthesis stage | Deterministic first. LLM output may only be raise-only overlay *proposals* an operator applies (`[DISC]` §5) |
| Snapshot/VM restore, undo | Local-only reversibility, semantic-rollback hazards (`[SAFE]` §3.6). The ActionRecord carries `irreversible` and idempotency keys meanwhile |
| Full CaMeL data-flow capabilities | Policy-authoring burden and utility loss (`[SAFE]` §3.4). Session-taint escalation is the pragmatic middle |
| Auto-install from `registry.json` / CLI-Hub | `install_cmd` strings run with `shell=True`, and in-repo entries are unpinned git installs (`[GEN]` §2.7). Engine-managed envs come in M3, built from local paths with hash-pinned locks |
| macOS network isolation for introspection (`sandbox-exec`) | Deprecated API, and its fidelity is unclear. Recorded as `ISOLATION_PROCESS` with the limitation "network not isolated" until Tier 2 lanes exist |
| Deferred/recorded catalog boot (REA deferred runtime) | A startup optimisation that isn't needed at our catalog size |
| Release signing, setup transaction across MCP clients | M4+ packaging work. The slice is source-built |

---

## 15. Risks

1. **Tier 0 confirmations are forgeable by a determined same-uid agent.** An agent with a shell can run `aa confirm`
   or edit the token files. *Mitigation:* say so in every ActionRecord (`assurance_tier`, `CONFIRMATION_ASSURANCE_*`);
   allow `require_tier = 1` for DESTRUCTIVE; ship Tier 1 setup in M5 with `aa doctor` proof. *Residual:* most users
   will run Tier 0.
2. **Confirmation fatigue leads to blanket allow.** Users disable prompts when prompted too often (`[SAFE]` §7
   "YOLO modes"). *Mitigation:* OBSERVE is auto-allowed; there are narrow expiring grants for MUTATE; DESTRUCTIVE and
   tainted-exfil are never grantable; a plan-then-approve batch is a later candidate. *Risk:* operators edit policy to
   `allow` everything. The policy digest in every record at least makes this visible.
3. **The conservative defaults may withhold so much that "easy" fails.** Harness-level floors (`subprocess` anywhere
   ⇒ `launches_process` on every command) may keep most capabilities withheld until M2's lanes exist. *Mitigation:*
   the M2 batch admission report measures this. Floors can be scoped per module via the import graph (only commands
   whose modules reach `lo_backend` inherit them).
4. **CLI-Anything harness fidelity varies.** There is doctrine/practice drift (the GIMP Pillow fallback), side
   effects such as auto-save, `--dry-run` that isn't a real dry run, and REPL defaults. *Mitigation:* the backend is
   part of the identity, state-diff lanes, `repl` is never emitted, and `DryRun` semantics stay unverified until
   proven.
5. **Introspection runs harness code.** Importing a module executes it, and parser probes execute group callbacks.
   *Mitigation:* operator confirmation on first introspection, owned process, scrubbed env, private HOME, deadline;
   the AST fallback is STATIC. *Residual:* no network isolation on macOS in v1.
6. **Writes outside the observed roots are invisible on macOS** without elevated tracing. *Mitigation:* the coverage
   field (`observed_roots`) is explicit, so a COMPLETE label is never claimed. Linux Landlock *enforcement* of write
   roots in M3 turns observation into prevention there.
7. **Proto3 zero-default semantics are subtle.** Contributors may "fix" lint by renaming zero values to
   `_UNSPECIFIED`, which silently changes meaning. *Mitigation:* a proptest asserts each safety enum's zero value
   evaluates to the most restrictive behaviour; `buf breaking`; a comment on every enum.
8. **Dependency churn.** tonic 0.14 recently split its codec crates, rmcp 3.x targets a 2026 MCP spec, `notify` 9 is
   an RC, and pbjson↔prost 0.14 compatibility is unverified. *Mitigation:* exact pins, a thin wrapper per external
   API, and the prost-reflect serde fallback for pbjson.
9. **Audit growth and performance.** fsync on every MUTATE plus per-call identity re-hash of large binaries could add
   latency. *Mitigation:* a stat-tuple cache with re-hash only on change, batched OBSERVE fsync, segment rollover.
10. **Scope and complexity.** 18 crates and a rich proto may slow early delivery. *Mitigation:* the slice uses 14
    crates with minimal surfaces. The proto is complete on day 1 (wire stability) but services are implemented
    incrementally; unimplemented RPCs return `UNIMPLEMENTED` and stay role-gated.
11. **Legal and ToS exposure for service-backed harnesses.** CLI-Anything includes jumpserver, mailchimp and others
    that act on remote services. *Mitigation:* `external_communication` ⇒ DESTRUCTIVE tier; the session `purpose`
    records the authority basis; the service's ToS, not the local licence, governs (`[SAFE]` §10); hard exclusions
    for circumvention categories.
12. **macOS ownership proof gaps.** `KERN_PROCARGS2` may withhold env for hardened or platform binaries. *Mitigation:*
    for processes the engine spawned itself, pgid tracking plus our own exit observation counts as positive proof (as
    REA does, `ProviderProcess.ts:719-729`). Other processes stay `unverified` and are never signalled.
