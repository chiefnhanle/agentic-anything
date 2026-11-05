# agentic-anything: final architecture

- **Status:** Accepted design (2026-11-05). This is the synthesis of three proposals in `proposals/` (mvp-first, safety-first,
  generator-first) and three judge verdicts.
- **How the verdicts shaped it:** two judges picked generator-first and one picked safety-first. The result is
  generator-first's distillation backbone, with safety-first's authority mechanisms and mvp-first's build discipline grafted on.
- **Companion artifacts:**
  - ADRs: `adr/0001`–`adr/0013`
  - draft protos: `proto/agentic_anything/v1/*.proto` (buf lint STANDARD clean)
  - first vertical slice: `build-plan.md`
- **Verification status:** everything marked *verified* was checked on this Mac on 2026-11-05:
  - rustc 1.99.1, protoc 36.1 and buf 1.75.0;
  - a scratch `aa-proto` build of these exact protos with tonic-prost-build 0.14.9, pbjson-build 0.9.0 and prost-reflect 0.16.5;
  - Seatbelt confinement through `/usr/bin/sandbox-exec`;
  - git 2.48.1 behaviour;
  - CLI-Anything at `/Users/nhanclie/CLI-Anything` HEAD `34f5195`.

Research is cited as `[report §n]` (files in `docs/research/`). REA is cited as `rea:path:line` (`/Users/nhanclie/rea` @ `3484432fb`).

---

## 0. The decisions on one page

1. **The core is a Rust gRPC engine, `aad`** (tonic 0.14.9 / prost 0.14.4). It serves one proto package, `agentic_anything.v1`, with 8 services and 41
   RPCs. The `aa` CLI, the `aa mcp` server and generated SKILL.md files are clients or renderings. Policy is never implemented
   in a front end (ADR 0001).
2. **The product is the manifest.** Each target is a directory with two files:
   - `target.aa.yaml`, a small overlay written by a human;
   - `capabilities.lock.json`, a deterministic, JCS-digested compiled lock.

   `lock = compile(harvest artifacts, overlay, rule versions)`, and `aa compile --check` gates CI (ADR 0003).
3. **The pipeline:** `discover → characterize → synthesize → verify → package`.
   - It is deterministic code throughout.
   - LLM output enters only through the typed `ManifestProposal` fence. A proposal can raise effects, narrow schemas and describe
     capabilities. It can never lower an effect, widen a schema, admit a capability or grant anything (§13).
4. **Effects are REA's ToolEffects, made tri-state (18 fields).**
   - Missing means UNKNOWN, and policy treats UNKNOWN as TRUE. Signals combine by max.
   - Lowering a field takes one of: OBSERVED lane evidence at an isolation that can see that field; a DECLARED trusted spec; or an
     attributed OPERATOR assertion on `operator.sock`.
   - Running the binding's own pinned executable is not an effect (§6).
5. **Admission depends on the effect level** (ADR 0007, 0011):
   - READ needs lanes L0+L1+L3.
   - WRITE adds an L4 read-back oracle that fails on the seed and passes after the write.
   - DESTRUCTIVE always asks.
   - UNKNOWN is never servable.

   On macOS, L3 runs under **Seatbelt confinement**: network denied, exec allow-list, writes denied outside the root (verified). The
   capability is then **served under the same confinement**, so a wrong READ admission still cannot write or phone home.
6. **Authority comes from the listener, never from a request field.**
   - The engine listens on `agent.sock` and `operator.sock`. Only `operator.sock` can decide confirmations, unhalt, assume
     effects or issue grants.
   - Every RPC carries an `rpc_policy` option, which is compiled into a static table. The build fails if a method lacks one.
   - Every safety enum has a fail-safe zero value (ADR 0002, 0005).
7. **One gate.** It runs in a fixed order and is used for agent calls and for the engine's own pipeline steps, such as the INTROSPECT
   harvest that runs harness code. Confirmations bind to an `action_digest` (§5.3).
8. **The audit log is hash-chained JSONL.**
   - Exactly one ActionRecord per attempt after resolution.
   - For WRITE+ and INTROSPECT, the intent is written and fsynced before dispatch. An intent without an outcome becomes `OUTCOME_UNKNOWN`.
   - The engine boots halted if the chain fails to verify (ADR 0006).
9. **Bindings.**
   - Generic bindings (`cli_argv` now; `mcp`, `http`, `grpc` later) run in-process as typed drivers over data. There is no generic code
     dispatcher and no per-capability codegen.
   - Foreign runtimes run out-of-process over `AdapterService` (ADR 0004, 0008).
10. **CLI-Anything harnesses are ingested, never trusted.**
    - They are imported from the local checkout.
    - The real Click tree is introspected under confinement.
    - `--json`, `--project` and `--dry-run` are mapped onto the IR.
    - `install_cmd` is never run (ADR 0009).
11. **Slice 1 is 7 crates and 7 work packages** (`build-plan.md`):
    - `cli_argv` binding;
    - harvesters: Click, Cobra, `-h`/`--help` and CLI-Anything;
    - L0/L1/L3 lanes and READ admission;
    - policy evaluator, gate and JSONL audit;
    - `aa` over UDS;
    - e2e against real `git` (and `gh` when present) on this Mac.

---

## 1. Goals and non-goals

**Goals**
- **G1 EASY.** Point the engine at software the owner owns or may automate, and get a typed, verified, policy-gated interface
  through the CLI, the gRPC API, MCP and Agent Skills.
  - Scaffolding takes minutes, and that state is reported honestly (`TARGET_STATUS_SCAFFOLDED`).
  - Trust grows per capability through evidence: `withheld → servable`, `scaffolded → live-verified → hardened`
    (`[rea-adding-a-new-target-walkthrough §What to distill 10]`, `[pilot-pipeline-on-real-targets §8]`).
- **G2 Any software.** The binding sources include:
  - CLIs (Click/CLI-Anything, Cobra, Swift ArgumentParser, `--help`);
  - MCP servers;
  - OpenAPI/gRPC/GraphQL services;
  - macOS sdef/App Intents/AX;
  - in-host plugin bridges.

  Spec-shaped surfaces need zero code.
- **G3 Safe to drive.** Effects are explicit. Unknown means withheld. Approvals cannot be forged through the agent's own channel.
  Every act leaves tamper-evident evidence.
- **G4 One source of truth.** Every surface is rendered from the lock, and set-equality tests replace parity tests.

**Non-goals**
- DRM or licence defeat, protected-asset extraction, evading security controls, and driving licence, registration or consent dialogs.
  These are compiled-in hard exclusions (§5.7).
- A security boundary against a same-uid agent that has a shell. That requires Tier 1, a separate agent uid (§5.2). Tier 0 is labelled
  a guardrail.
- An LLM inside the engine.
- Remote or multi-user serving.
- Vision or computer-use as a primary technique. It is the bottom rung of the technique ladder (PLAN.md).
- Generating harness code. CLI-Anything already does that; we verify its output.

---

## 2. Architecture overview

```
                 ┌──────────── front ends (clients) ─────────────┐
 agent ── MCP ──▶│ aa mcp (rmcp stdio) ─┐                        │
 agent ── sh ───▶│ aa x / aa run ───────┼──▶ agent.sock ──┐      │
 human ── tty ──▶│ aa approve/unhalt ───┼──▶ operator.sock┤      │
 CI    ─────────▶│ aa compile --check   │                │      │
                 └──────────────────────┘                ▼      │
  ┌──────────────────────────── aad (aa-engine) ───────────────────────────────────┐
  │ listener role layer → RPC_POLICY check → services                               │
  │  TargetService  DistillService  CatalogService  RuntimeService  PolicyService   │
  │  AuditService   EngineService                                                   │
  │                         │ every effectful step                                  │
  │                         ▼                                                       │
  │  GATE: resolve → admission → schema → path scope → halt/breaker → identity      │
  │        → policy (strictest wins) → [ask: park on action_digest] → intent(fsync)  │
  │        → dispatch (aa-exec, confined) → observe → ActionRecord                   │
  │                                                                                 │
  │ aa-core (pure): effects lattice · compile · characterize · admission · policy   │
  │ aa-harvest: cli-anything · click-info · cobra · help-text (harvest | candidates) │
  │ aa-exec: owned groups · deadlines · bounded output · Seatbelt Confiner · lanes   │
  │ aa-store: workspace · CAS · JSONL hash-chained audit · halt state               │
  └──────────┬──────────────────────────────────────────────────────┬──────────────┘
             │ owned process group (never a shell)                   │ M2: AdapterService
             ▼                                                       ▼ (one-shot | gRPC/UDS)
     git · gh · cli-anything-libreoffice (python -I) · …       Swift AX/sdef · Python bridge · …
```

Pipeline, crates and RPCs:

| Stage | Code | RPC | Artifact | LLM role |
|---|---|---|---|---|
| Discover | `aa-harvest` (+ one-shot shims) | `DistillService.Harvest` | verbatim raw bytes in the CAS + `harvest/index.json` | none |
| Characterize | `aa-core::characterize` | inside `Compile` | signals per field (rule tables as code, `rule:<name>@n`) | none |
| Synthesize | `aa-core::manifest::compile` + `aa-core::proposal` (slice 2) | `Compile`, `ApplyProposal` | `capabilities.lock.json` | proposes overlay ops only |
| Verify | `aa-core::lanes::static_lane` (L0) + `aa-exec::lanes` (L1–L4) | `Verify` | `verification/<cap>/vr_*.json` | drafts examples/fixtures as proposals |
| Package | `aa-core::render` (slice 2) | `Package`, `CatalogService.GetExposure` | `packages/**`, catalog digest | none |

`aa distill <target>` runs harvest → compile → verify → package with defaults.

---

## 3. Workspace layout

### 3.1 Repository

```
Cargo.toml                 # [workspace] resolver="3", edition 2024, rust-version "1.88"; [workspace.dependencies] pinned
rust-toolchain.toml        # channel = "1.99.1"
buf.yaml                   # v2; lint STANDARD minus ENUM_ZERO_VALUE_SUFFIX; breaking FILE
proto/agentic_anything/v1/ # common, manifest, record, engine_service, target_service, distill_service,
                           # catalog_service, runtime_service, policy_service, audit_service, adapter_service
crates/
  aa-proto/    # generated messages; `grpc` feature adds tonic services; RPC_POLICY table; descriptor set
  aa-core/     # PURE: ids, digest(JCS), errors, effects, characterize, manifest(compile), admission, policy,
               #       lanes::static_lane, shared traits (ProcessRunner, Binding, Harvester, AuditSink, Confiner)
  aa-exec/     # supervision kernel, Seatbelt Confiner, cli_argv binding, L1/L3 (L4 slice 2)
  aa-harvest/  # cli-anything-registry, click-info (python shim), cobra-complete, help-text
  aa-store/    # workspace FS layout, CAS, JSONL audit chain, intent recovery, halt persistence
  aa-engine/   # `aad`: listeners, role layer, gate, services, composition root
  aa-cli/      # `aa`: client; `mcp` feature (slice 2); e2e tests (tests/)
crates/aa-harvest/shims/aa_click_info.py   # embedded with include_str!; written into a private root per run
targets/                        # committed examples: git, fixture-notes, cli-anything.libreoffice
skills/aa-distill/SKILL.md      # hand-authored: the proposal loop and its fence (slice 2)
```

Dependency rule (ADR 0013), enforced by a `cargo metadata` test:
- `aa-proto ← aa-core ← {aa-exec, aa-harvest, aa-store} ← aa-engine`.
- `aa-cli` depends on `aa-proto[grpc]` only, plus `aa-engine` as a dev-dependency.
- `aa-core` bans tokio, tonic, hyper and rusqlite.

### 3.2 On-disk state

```
$AA_HOME (default ~/.local/share/agentic-anything; `aa --workspace ./aa` for a project-local, committable workspace)
  targets/<target-id>/
    target.aa.yaml                # OVERLAY (human); committed
    capabilities.lock.json        # LOCK (compiled, canonical proto3-JSON); committed
    harvest/index.json            # SourceRef list -> CAS digests
    verification/<cap-id>/vr_<sha>.json
    packages/{mcp,skill,cli}/...  # slice 2
    proposals/<id>.json           # slice 2
  cas/sha256/ab/cdef...           # immutable raw harvests, outputs, observations
  sessions/<session-id>/          # 0700 session workspaces (engine-supplied state handles)
  audit/segment-000001.jsonl  audit/HEAD
  state/halt.json  state/hmac.key (0600)  state/HALT (sentinel, optional)
~/.config/agentic-anything/policy.toml   # operator-owned; refused if group/other-writable
runtime: $TMPDIR/aa/ (macOS; fallback /tmp/aa-<uid>/) | $XDG_RUNTIME_DIR/aa/ (Linux)
  agent.sock  operator.sock  engine.lock  runs/<run-id>/   (0700 dir, 0600 sockets)
```

---

## 4. gRPC services

The source of truth is `proto/agentic_anything/v1/*.proto`. Roles come from the listener (§5.1).

| Service | RPCs (roles; **H** = allowed when halted) |
|---|---|
| `EngineService` | `GetStatus` (A,O,H) · `Halt` (A,O,H; anyone may pull the brake) · `Unhalt` (O,H; needs current `halt_epoch`) · `Shutdown` (O,H) |
| `TargetService` | `ListTargets`/`GetTarget` (A,O,H) · `CreateTarget` (A,O; STATIC; authorization statement required) · `ImportCliAnything` (A,O; STATIC) · `AssumeEffects` (**O only**; per field, audited) |
| `DistillService` | `Harvest` stream (A,O; INTROSPECT is a gated pipeline action) · `Compile` (pure) · `Verify` stream · `Package` · `GetProposalContext` (H) · `ApplyProposal` (A,O; fenced) |
| `CatalogService` | `ListCapabilities` · `SearchCapabilities` · `DescribeCapability` · `GetExposure` · `WatchCatalog` stream (all A,O,H) |
| `RuntimeService` | `OpenSession` · `CloseSession` (H) · `Invoke` stream · `Resume` stream · `Cancel` (H) · `GetActionStatus` (H) |
| `PolicyService` | `ExplainPolicy` (A,O,H) · `ListPendingConfirmations` (A,O,H) · `DecideConfirmation` (**O only**) · `RelayElicitation` (A; verified relay only) · `IssueGrant` (**O only**) · `ListGrants` (H) · `RevokeGrant` (A,O,H; agents may narrow) |
| `AuditService` | `QueryAudit` · `VerifyChain` (A,O,H) |
| `AdapterService` | served *by adapters*, called by the engine (`ROLE_ENGINE_INTERNAL`): `Handshake`, `Harvest`, `Invoke`, `GetRequestStatus`, `Observe`, `Shutdown` |

Conventions:
- Streams are used only where progress matters (Harvest, Verify, Invoke, Resume, WatchCatalog). The terminal event is always one of `result`,
  `error`, `confirmation` or `summary`.
- RPCs that are not yet implemented return `UNIMPLEMENTED`. The surface is fixed now so that clients and adapters can be written against it.

**Error mapping** (from mvp-first):

| Situation | Transport | ActionRecord? | `aa` exit |
|---|---|---|---|
| Wrong role/listener/peer; method not in `RPC_POLICY` for the role | `PERMISSION_DENIED` + `Error{ACCESS_DENIED}` in details | no (AdminEvent if audited) | 3 |
| Halted and the method is not allowed when halted | `UNAVAILABLE` + `Error{ENGINE_HALTED, recovery: RESTART_ENGINE}` | no | 4 |
| Unknown target or capability; malformed envelope | `NOT_FOUND` / `INVALID_ARGUMENT` + `Error` | no | 2 |
| Input fails the canonical schema | OK, in-band `Error{INVALID_REQUEST}`, `OUTCOME_NOT_DISPATCHED` | **yes** | 2 |
| Withheld, policy deny, path scope, identity changed, catalog stale | OK, in-band `Error`, `NOT_DISPATCHED` | **yes** | 3 |
| Confirmation required | OK, `PendingConfirmation` event | **yes** (`NOT_DISPATCHED`) | 3 |
| Target ran and completed | OK, `InvokeResult` | yes | 0 |
| Target error, unreadable output, timeout, resource limit | OK, in-band `Error` | yes | 1 |
| Client dropped the RPC | n/a | yes (`CANCELLED`, group killed) | 130 |
| Engine unreachable | connect error | no | **4**: restart the engine, do not retry the call |

---

## 5. Transport, security and the authority model

### 5.1 Listeners and roles (ADR 0005)
- The runtime dir is created 0700 and checked with `lstat`. The engine refuses a symlink, a foreign owner, or group/other bits.
- Startup sequence:
  1. take `flock(engine.lock)`;
  2. unlink stale sockets;
  3. bind `agent.sock` and `operator.sock`;
  4. `chmod 0600`;
  5. re-`lstat`.

  The engine refuses euid 0.
- Two `Server::builder().layer(RoleLayer(role)).serve_with_incoming(UnixListenerStream)` futures run side by side. On every request, a shared interceptor:
  - reads `UdsConnectInfo.peer_cred`. The uid must be the owner on `operator.sock`, and must be in `agent_uids` (default: the owner) on `agent.sock`;
  - looks up the method path in the generated `RPC_POLICY` table, then checks the role and the halt state.
- No token is used on UDS. Loopback TCP (M4) is opt-in and agent-role only. It requires a 0600 bearer token, a constant-time compare, and a
  loopback-literal `:authority`.
- Peer pid → exe realpath → sha256 is recorded in `Principal`, and is *required* for `RelayElicitation` (§5.4).
- Clients use tonic's native `unix://` endpoint (`Endpoint::from_shared`, verified at `endpoint.rs:186-215`). No hyper-util or tower is needed.

### 5.2 Assurance tiers (computed from proofs, recorded in every `PolicyDecision`)

| Tier | Setup | What an approval proves |
|---|---|---|
| 0 `SAME_PRINCIPAL` (default) | agent host, `aa mcp` and the human share one uid | It guards against model error and injection steering the normal tool path. It is **not** a boundary against an agent with a shell, which can reach `operator.sock`. |
| 1 `SEPARATED_AGENT` (M4 setup, `aa doctor`) | the agent host runs as `aa-agent` (or in a container) and can reach only `agent.sock` (0660, group `aa-agents`) | only the human's uid could approve |
| 2 `ISOLATED_TARGET` | Tier 1 + the target runs in a container/VM lane | Tier 1, plus target effects are contained |

### 5.3 The gate (single enforcement point, `aa-engine::gate`)
The authority object is a required constructor dependency of the dispatcher, so no code path can skip it. Order:

1. **Listener / role / halt** (interceptor). Wrong role → `PERMISSION_DENIED`. Halted → `UNAVAILABLE`.
2. **Resolve** the capability by id. If `expected_capability_digest` or `expected_catalog_digest` is given, compare it (mismatch → `CATALOG_STALE`). *From here
   on, exactly one ActionRecord is written for the attempt.*
3. **Admission.** Anything other than `SERVABLE` → `CAPABILITY_WITHHELD` with `Withheld[]`. `EXCLUDED` is final.
4. **Recompute projections** (level, tier) from the stored effects and branches, and compare them to the lock. A mismatch means corrupt
   state, so the engine halts. Apply the matching `EffectBranch`es for this input. Clamp to the session `authority_ceiling`.
5. **Validate** the input against the *canonical* closed schema (`jsonschema`). Canonicalize path scopes (realpath; for writes,
   the realpath of the parent) and check them against the session `PathScope`, where **deny wins**.
6. **Brakes.** Kill-switch epoch; circuit breaker (M3).
7. **Identity.** Re-check every `TargetIdentity.files` entry: stat tuple, then rehash on change. Mismatch → `TARGET_IDENTITY_CHANGED`,
   `recovery: REDISTILL`. Check backend preconditions → `PRECONDITION_UNMET`. Check confinement availability when
   `required_runtime_isolation = CONFINED`.
8. **Policy.** Hard exclusions → collect **all** matching rules → the strictest wins (deny > ask > allow), regardless of order.
   No match → deny. Each denial carries a typed `DenialReason` that maps to exactly one existing remediation command.
9. **Ask** (M3). Park a `PendingConfirmation` bound to
   `action_digest = sha256(JCS{capability_id, capability_digest, lock_digest, identity_digest, generation, args_commitment,
   policy_digest, dry_run})`. It is single-use with a 120 s TTL. The prompt is engine-authored, and the agent's `stated_reason` and the dry-run preview are labelled.
   `Resume` re-runs steps 1–8, and any digest drift → `TARGET_STATE_CHANGED` and a new ask.
10. **Intent.** For level ≥ WRITE and for INTROSPECT, append an `ActionIntent` and **fsync**. If that fails → `AUDIT_UNAVAILABLE`, and nothing runs.
11. **Dispatch** through `aa-exec` with the deadline, output caps and kill-switch cancel token. Wrap in the `Confiner` profile
    when required.
12. **Post-observe.** Re-run the read-back oracle (WRITE+, slice 2) and hash the state handle. Anything observed beyond the declared effects
    fails the action, **auto-withholds** the capability (`SIDE_EFFECT`), and both are audited.
13. **Record** the `ActionRecord` outcome. Return output with `ContentOrigin TARGET`. Set the session taint bit.

### 5.4 Confirmations, elicitation and grants (ADR 0007)
- Only `DecideConfirmation` on `operator.sock` approves. `aa approve <id>` shows the engine-authored challenge and requires a TTY.
- **MCP elicitation is host-mediated consent**, recorded as `APPROVAL_SOURCE_HOST_ELICITATION`. This fixes the judges' self-approval
  finding against safety-first. `RelayElicitation` is accepted only if:
  1. the calling peer's executable (pid → realpath → sha256) is the engine-pinned `aa` binary, so an arbitrary
     `agent.sock` client cannot call it;
  2. policy lists `HOST_ELICITATION` for the action's level. The default is WRITE only and never DESTRUCTIVE;
  3. the assurance tier permits it. It is off by default in Tier ≥ 1 unless the operator opts in.

  The residual is stated plainly: an agent that spawns the genuine `aa mcp` and acts as its MCP client can answer its own
  elicitation. That is why elicitation never covers DESTRUCTIVE.
- **Grants** (M3) are issued only on `operator.sock`. Each is narrow, expiring and revocable: session × capability glob × `max_tier < DESTRUCTIVE` × TTL ≤ 1 h × max uses.
  Agents can list and revoke grants but never mint them. `aa grant`, `aa grants list`, `aa policy explain` and `aa grants revoke` ship together. A
  test asserts that every grant lifetime and every denial reason has a production issuer and an existing CLI command (REA #545/#552).

### 5.5 Policy file (default deny; strictest match wins)

```toml
# ~/.config/agentic-anything/policy.toml  (operator-owned, 0600/0644; refused if group/other-writable)
format = "aa-policy/1"
default = "deny"

[paths]
write_roots = ["~/Documents/aa-out", "~/src"]        # session workspaces are always writable
deny        = ["~/.ssh", "~/.aws", "~/.config/agentic-anything", "~/Library/Keychains"]

[[rule]]
id = "observe-registered"
effect = "allow"
when = { tier = "observe", target_registered = true }

[[rule]]
id = "write-in-roots"
effect = "allow"
when = { level = "write", targets = ["git", "cli-anything.*"] }   # PathScope still applies

[[rule]]
id = "destructive"
effect = "ask"
when = { level = "destructive" }
approval_sources = ["operator_socket"]                          # never elicitation, never grants

[[rule]]
id = "tainted-exfil"
effect = "ask"
when = { session_tainted = true, any_effect = ["external_side_effect", "executes_foreign_code", "reads_credentials", "writes_clipboard"] }
approval_sources = ["operator_socket"]

[[rule]]
id = "privacy-reads"
effect = "ask"
when = { any_effect = ["reads_screen", "reads_clipboard", "reads_credentials"] }
```

Matching uses structured descriptors only: target, capability glob, level, tier, effect bits, canonical scopes, taint and assurance tier. It
never uses command text. `aa policy explain` runs the same evaluator.

### 5.6 Kill switch, untrusted content and taint
- **Halt sources:**
  - the `Halt` RPC, from either socket;
  - `touch <state>/HALT`;
  - `SIGUSR1`;
  - automatic: the audit chain fails to verify at boot or at runtime.
- **Halt effects:**
  - an atomic epoch is checked at gate entry, before dispatch and on `Resume`;
  - in-flight actions are cancelled, and **only owned groups** are killed;
  - pending confirmations expire.
- The halt state is persisted, and the engine boots halted if it stopped halted. `Unhalt` needs `operator.sock` and the current `halt_epoch`.
- **Untrusted content.**
  - Every target string is `TargetText` with `origin`.
  - MCP descriptions are an engine template plus ≤ 300 chars of vendor text in an explicit `Vendor description (untrusted data): «…»`
    block (tool-poisoning defence).
  - Outputs carry `_meta.provenance.untrusted_content: true`.
  - Nothing harvested can lower an effect.
- **Session taint.** After a session receives target content, actions with `external_side_effect`, `executes_foreign_code`,
  `reads_credentials` or `writes_clipboard` escalate to ask (§5.5).

### 5.7 Hard exclusions (compiled into `aa-core::policy::exclusions`)
These are evaluated before policy. Configuration can add categories but cannot remove them.
- Licence, activation, registration, DRM, anti-cheat, EDR and security-product surfaces are `ADMISSION_STATE_EXCLUDED`. This is decided by name and keyword
  at characterize time and by signer or bundle id at identity time.
- Licence and consent dialogs are never driven. REA's Hopper demo clicker is not ported.
- A vendor that disables its automation surface (for example an Electron fuse) is a hard stop.
- Password managers and banking apps get deny-by-default for input, screen and credential reads.

---

## 6. Effects and characterization (ADR 0002, 0011)

### 6.1 The vector

`manifest.proto` `Effects` has 18 tri-state fields. The **target-facing** fields are `mutates_target`, `writes_filesystem`,
`launches_process`, `accesses_network`, `changes_ui_state`, `sends_input`, `may_discard_data`, `executes_foreign_code`,
`mutates_device`, `external_side_effect`, `attaches_process`, `reads_screen`, `reads_clipboard`, `writes_clipboard`,
`reads_credentials` and `irreversible`. The **control-plane** field is `mutates_session`, which is never in `readOnlyHint`; REA's merged flag made 109/234 tools
non-read-only (`[rea-tool-design-philosophy-and-verification §1.6]`). The **safe-direction** field is `idempotent`, which combines by AND, with UNKNOWN treated as FALSE.

### 6.2 Combination and lowering
- For each field, take the max over *admissible* signals, ordered `UNKNOWN < FALSE < TRUE`. A FALSE is admissible only from:

| Transition | Allowed sources |
|---|---|
| UNKNOWN → TRUE (raise) | anything: harvester native signal, rule table, proposal, lane, operator |
| UNKNOWN → FALSE | `OBSERVED` from a lane whose isolation can see that field (ADR 0011 table); `DECLARED` from an operator-trusted spec; `OPERATOR` via `AssumeEffects` on `operator.sock` (audited per field) |
| TRUE → FALSE | `OPERATOR` with a reason only. Never a lane ("absence of evidence"), never a proposal |

- A FALSE from INFERRED or PROPOSED is recorded and ignored.
- Effects are not inherited down a command tree. Flags and params can only raise effects, through `EffectBranch`es. A branch that has not been resolved,
  where a flag switches authority (for example `gh api -X`), means `MIXED_AUTHORITY`, withheld (`[pilot-pipeline-on-real-targets §What to distill 4]`).
- **Binding-intrinsic execution is not an effect.** Running the identity-pinned executable or interpreter of a `cli_argv` binding with
  `KILL_SCOPE_OWNED_GROUP` is covered by the target registration (`rea:src/contracts/toolEffects.ts:12-18`). Under
  confinement, the exec allow-list is exactly those pinned files. So a child process (soffice, a pager, `sh`) is *denied and
  observed*, which raises `launches_process` TRUE.

### 6.3 Projections (derived; `aa-core::effects`; proptest-monotone)
- **`level`:**
  - UNKNOWN if any target-facing field is UNKNOWN;
  - else DESTRUCTIVE if any of `may_discard_data | irreversible | executes_foreign_code | mutates_device | external_side_effect | attaches_process | reads_credentials` is TRUE;
  - else WRITE if any of `mutates_target | writes_filesystem | launches_process | accesses_network | writes_clipboard` is TRUE;
  - else UI if any of `changes_ui_state | sends_input` is TRUE;
  - else READ.
- **`tier`:**
  - `attaches_process` → ATTACH; instrumentation adapters → INJECT; install/OS settings → MUTATE_SYSTEM;
  - otherwise from the level: DESTRUCTIVE→DESTRUCTIVE, WRITE→WRITE, UI→DRIVE;
  - `launches_process` with nothing else → LAUNCH; READ→OBSERVE;
  - UNKNOWN → `AUTHORITY_TIER_UNKNOWN`, which is evaluated as the highest tier.
- **`hints`** use REA's `annotationsFromEffects` (`rea:src/contracts/toolEffects.ts:765-787`) over target-facing fields:
  - `readOnly` = level READ, with every target field FALSE;
  - `destructive` = level DESTRUCTIVE, or an UNKNOWN in its inputs;
  - `idempotent` = TRUE only;
  - `openWorld` = any of `launches_process | accesses_network | external_side_effect` that is not FALSE.

### 6.4 Rule tables as code (`aa-core::characterize::rules`, each versioned `rule:<name>@n`)
- **`cli-verbs@1`**, raise-only, on the leaf token:
  - `add|new|create|set|modify|rename|apply|save|import|open|insert|update|commit|checkout|merge|rebase|push|pull` → `mutates_target`;
  - `remove|delete|rm|clear|reset|purge|drop|prune|wipe|overwrite` → `may_discard_data`;
  - `send|publish|post|pay|upload|push` → `external_side_effect` + `accesses_network`.

  Read-ish tokens (`list|info|show|get|status|log|presets`) emit **nothing**. They are only a hint in the withheld message ("likely READ; run
  `aa verify`"). Only L3 or the operator lowers a field.
- **`cli-options@1`:**
  - `-o/--output/--out/output_path` → `writes_filesystem` with a `WRITE_PATH` scope;
  - `--force/-f/--yes/--hard/--clobber/--overwrite/-D` → a branch raising `may_discard_data`;
  - `--web/--browser` → a branch raising `launches_process`, and the parameter is excluded by default;
  - `-X/--method` → a branch giving `MIXED_AUTHORITY`.
- **`cli-anything@1`:** the autosave rule (`mutates_target` with a state handle → `writes_filesystem`); `repl` → `INTERACTIVE`; the `{error,type}` convention.
- **`exclusions@1`:** licence/activation/register/drm/serial keywords → `EXCLUDED_BY_POLICY`.
- A rule change shows up as a lock diff, and the lock records the rule version on every signal. The pilot measured table error rates of 5–10% on gh and CDP.
  That is why the tables only raise and why observation does the lowering.

---

## 7. The capability IR and manifest format (ADR 0003)

### 7.1 Identifiers
- A target id matches `[a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*`, for example `git` or `cli-anything.libreoffice`.
- A capability id is `<target>.<noun-path>.<verb>`. It is derived from the native address and keeps `provenance.native_ids`.
- An MCP name is `<mcp_prefix>_<noun>_<verb>`, matching `[a-zA-Z0-9_-]{1,64}`, with a deterministic 6-hex suffix on collision.
- `capability_digest = sha256(JCS(schema, binding, effects, signals, branches, oracles, scopes, dry_run))`. Admission and reviews
  carry over across recompiles only while it is unchanged.

### 7.2 Overlay schema (`target.aa.yaml`, `schema: aa.overlay/v1`; parsed by `aa-core::manifest::overlay`)

| Key | Meaning | Who may write it |
|---|---|---|
| `target`, `display_name`, `mcp_prefix`, `locator` | identity of the target | operator; `aa target add` writes a starter |
| `harvest[]` | harvesters and their options (`commands` include-list, `help_argv`) | operator / proposal (narrowing only) |
| `defaults.binding` | `global_args`, `env`, `deadline_ms`, `max_output_bytes`, `usage_exit_code`, `usage_stderr_marker` | operator |
| `state_handle` | `{flag, param, engine_supplied}` | operator |
| `exclude[]` | `{capability, reason}` | operator / proposal |
| `fixtures[]` | seed recipes (argv / file / capability steps) | operator / proposal |
| `capabilities.<id>.{binding (partial override: command_path, fixed_args, args), fixed_args, cwd_pointer, effects (raise only), branches (raise only), scopes, oracle, examples, core, use_when}` | per-capability curation. `fixed_args`/`cwd_pointer` are shorthands for the binding fields. `cwd_pointer` adds a required `format: path` input property if absent. A new id whose `binding.command_path` equals a harvested native path is a **mode split** (e.g. `git.branch.list` / `git.branch.delete`), and the unsplit native candidate is then withheld `MIXED_AUTHORITY` | operator / proposal (fenced; binding and scope edits are operator-only) |
| `assumed[]` | mirror of `AssumeEffects` audit entries `{field, value, glob, reason, audit_seq}`. Compile accepts an entry only if its `audit_seq` verifies in the chain | **engine only** (from operator.sock) |

### 7.3 Full example: `git` (a real CLI on this Mac, git 2.48.1)

This example also shows the pipeline catching a "read" that writes. The facts below were verified on this Mac on 2026-11-05 in a scratch repo:
- Plain `git status` **rewrote `.git/index`** after a file's mtime changed. `git --no-optional-locks status` did not.
- Unknown flags exit 129.
- `git branch -d -- <missing>` exits 1 with `error: branch '<x>' not found`.
- `git add -- <path>` and `git log … --` accept the `--` separator.

**Overlay** `targets/git/target.aa.yaml`:

```yaml
schema: aa.overlay/v1
target: git
display_name: Git
mcp_prefix: git
locator: { executable: git }              # resolved to /opt/homebrew/Cellar/git/2.48.1/bin/git, sha256 pinned
authorization: "Owner workstation; git is GPL-2.0 free software; automating my own repositories."

harvest:
  - harvester: help-text@1                 # git prints parse-options usage on `<cmd> -h` (exit 129)
    commands: [status, log, branch, add, rev-parse]   # curated include-list; git has ~150 commands
    help_argv: ["{command}", "-h"]

defaults:
  binding:
    # --no-optional-locks: without it, L3 observed `status` rewriting .git/index (SIDE_EFFECT).
    global_args: ["--no-optional-locks", "-c", "core.pager=cat", "-c", "color.ui=never"]
    env: { GIT_CONFIG_NOSYSTEM: "1", GIT_TERMINAL_PROMPT: "0", GIT_OPTIONAL_LOCKS: "0" }
    deadline_ms: 15000
    max_output_bytes: 8388608
    usage_exit_code: 129

fixtures:
  - id: one-commit-repo
    steps:
      - argv: [git, init, -q, repo]
      - file: { path: repo/README, content: "hello\n" }
      - argv: [git, -C, repo, add, README]
      - argv: [git, -C, repo, -c, user.name=aa, -c, user.email=aa@localhost, commit, -qm, init]
      - argv: [git, -C, repo, branch, feature-x]
    ready: git.status
  - id: repo-with-untracked
    steps:
      - include_fixture: one-commit-repo
      - file: { path: repo/NEW.txt, content: "new\n" }
      - file: { path: repo/docs/a.md, content: "a\n" }
    ready: git.status

capabilities:
  git.status:
    fixed_args: ["--porcelain=v2", "--branch"]
    scopes: [ { pointer: /repo, kind: READ_PATH } ]
    cwd_pointer: /repo
    core: true
    examples:
      - { input: { repo: { $root: /repo } }, fixture: one-commit-repo }
      - { input: { repo: { $root: /repo }, untracked_files: "no" }, fixture: one-commit-repo }
  git.log:
    fixed_args: ["--format=%H%x09%an%x09%aI%x09%s"]
    scopes: [ { pointer: /repo, kind: READ_PATH } ]
    cwd_pointer: /repo
    core: true
    examples:
      - { input: { repo: { $root: /repo }, max_count: 5 }, fixture: one-commit-repo }
      - { input: { repo: { $root: /repo }, max_count: 1, author: aa }, fixture: one-commit-repo }
  git.add:
    scopes: [ { pointer: /repo, kind: WRITE_PATH } ]
    cwd_pointer: /repo
    oracle:
      read_back:
        capability: git.status
        args: { repo: { $input: /repo } }
        predicates:
          - { pointer: /lines, op: CONTAINS, value: { $input: /pathspec/0 } }   # "1 A. … <path>"
    examples:
      - { input: { repo: { $root: /repo }, pathspec: [NEW.txt] }, fixture: repo-with-untracked }
      - { input: { repo: { $root: /repo }, pathspec: [docs/] }, fixture: repo-with-untracked }
  git.branch.delete:
    binding: { command_path: [branch], fixed_args: ["--delete"] }   # `git branch` is one native command; split by mode
    branches:
      - { pointer: /force, effects: { irreversible: true } }        # -D deletes unmerged work
    scopes: [ { pointer: /repo, kind: WRITE_PATH } ]
    cwd_pointer: /repo
    oracle:
      negative:
        capability: git.branch.list
        args: { repo: { $input: /repo } }
        predicates: [ { pointer: /lines, op: ABSENT, value: { $input: /name } } ]
        bogus_input: { repo: { $root: /repo }, name: aa-no-such-branch }   # must fail (exit 1)
  git.branch.list:
    binding: { command_path: [branch], fixed_args: ["--list", "--format=%(refname:short)"] }
    scopes: [ { pointer: /repo, kind: READ_PATH } ]
    cwd_pointer: /repo
exclude:
  - { capability: git.branch.edit-description, reason: "launches $EDITOR (interactive)" }
```

**Lock** `targets/git/capabilities.lock.json`. This is canonical proto3-JSON; it is shown pretty-printed with 2 of its 7 capabilities in full and the rest abridged.
Fields at their zero value are omitted by pbjson, so an omitted effect field **means UNKNOWN**.

```json
{
  "schema": "aa.lock/v1",
  "target": {
    "id": "git", "displayName": "Git", "kind": "TARGET_KIND_CLI_TOOL", "mcpPrefix": "git",
    "identity": {
      "files": [ { "role": "executable", "realpath": "/opt/homebrew/Cellar/git/2.48.1/bin/git",
                   "sha256": "<sha256 of the binary>", "versionBanner": "git version 2.48.1" } ],
      "specSha256": "<sha256 of help-text raw artifact>",
      "identityDigest": "<sha256>"
    },
    "license": { "spdx": "GPL-2.0-only", "url": "https://git-scm.com/about/free-and-open-source" },
    "sources": [ { "harvester": "help-text@1", "uri": "argv:git {status,log,branch,add,rev-parse} -h",
                   "sha256": "<cas digest>", "authority": "DISCOVERY_AUTHORITY_INTROSPECT",
                   "harvestedAt": "2026-11-05T22:10:04Z", "actionId": "act_01JC…" } ],
    "status": "TARGET_STATUS_SCAFFOLDED",
    "authorizationStatement": "Owner workstation; git is GPL-2.0 free software; automating my own repositories.",
    "fixtures": [ { "id": "one-commit-repo", "readyCapabilityId": "git.status", "steps": [ "…as overlay…" ] } ]
  },
  "capabilities": [
    {
      "id": "git.status", "targetId": "git", "family": "status",
      "title": { "text": "Show the working tree status", "origin": "CONTENT_ORIGIN_ENGINE" },
      "description": { "text": "Show the working tree status", "source": "help-text:git status -h" },
      "inputSchemaJson": "{\"additionalProperties\":false,\"properties\":{\"ignored\":{\"enum\":[\"traditional\",\"matching\",\"no\"]},\"repo\":{\"format\":\"path\",\"type\":\"string\"},\"untracked_files\":{\"enum\":[\"no\",\"normal\",\"all\"]}},\"required\":[\"repo\"],\"type\":\"object\"}",
      "outputSchemaJson": "{\"properties\":{\"lines\":{\"items\":{\"type\":\"string\"},\"type\":\"array\"},\"text\":{\"type\":\"string\"}},\"type\":\"object\"}",
      "effects": {
        "mutatesTarget": "TRI_FALSE", "writesFilesystem": "TRI_FALSE", "launchesProcess": "TRI_FALSE",
        "accessesNetwork": "TRI_FALSE", "changesUiState": "TRI_FALSE", "sendsInput": "TRI_FALSE",
        "mayDiscardData": "TRI_FALSE", "executesForeignCode": "TRI_FALSE", "mutatesDevice": "TRI_FALSE",
        "externalSideEffect": "TRI_FALSE", "attachesProcess": "TRI_FALSE", "readsScreen": "TRI_FALSE",
        "readsClipboard": "TRI_FALSE", "writesClipboard": "TRI_FALSE", "readsCredentials": "TRI_FALSE",
        "irreversible": "TRI_FALSE", "mutatesSession": "TRI_FALSE", "idempotent": "TRI_TRUE"
      },
      "signals": [
        { "field": "writes_filesystem", "value": "TRI_FALSE", "confidence": "CONFIDENCE_OBSERVED",
          "source": "verify:vr_3f2a…", "isolation": "ISOLATION_CONFINED",
          "rationale": "2 examples; private-root tree diff empty; writes outside root denied and none attempted" },
        { "field": "accesses_network", "value": "TRI_FALSE", "confidence": "CONFIDENCE_OBSERVED",
          "source": "verify:vr_3f2a…", "isolation": "ISOLATION_CONFINED" },
        { "field": "launches_process", "value": "TRI_FALSE", "confidence": "CONFIDENCE_OBSERVED",
          "source": "verify:vr_3f2a…", "isolation": "ISOLATION_CONFINED", "rationale": "exec allow-list = pinned git only" },
        { "field": "changes_ui_state", "value": "TRI_FALSE", "confidence": "CONFIDENCE_OPERATOR",
          "source": "operator:audit_seq=42", "rationale": "headless CLI; no UI surface" },
        { "field": "idempotent", "value": "TRI_TRUE", "confidence": "CONFIDENCE_OBSERVED", "source": "verify:vr_3f2a…",
          "rationale": "identical output on repeat; no state change" }
      ],
      "level": "EFFECT_LEVEL_READ", "tier": "AUTHORITY_TIER_OBSERVE",
      "hints": { "readOnly": true, "idempotent": true },
      "scopes": [ { "pointer": "/repo", "kind": "SCOPE_KIND_READ_PATH" } ],
      "binding": { "cli": {
        "executableRole": "executable",
        "globalArgs": ["--no-optional-locks", "-c", "core.pager=cat", "-c", "color.ui=never"],
        "commandPath": ["status"], "fixedArgs": ["--porcelain=v2", "--branch"],
        "args": [ { "pointer": "/untracked_files", "style": "ARG_STYLE_OPTION", "flag": "--untracked-files" },
                  { "pointer": "/ignored", "style": "ARG_STYLE_OPTION", "flag": "--ignored" } ],
        "cwdPointer": "/repo",
        "output": "OUTPUT_MODE_TEXT_STDOUT", "errors": "ERROR_CONVENTION_EXIT_CODE",
        "deadlineMs": 15000, "maxOutputBytes": "8388608", "killScope": "KILL_SCOPE_OWNED_GROUP",
        "usageExitCode": 129,
        "env": { "GIT_CONFIG_NOSYSTEM": "1", "GIT_OPTIONAL_LOCKS": "0", "GIT_TERMINAL_PROMPT": "0" } } },
      "returnsTargetContent": true, "lifetime": "LIFETIME_ONESHOT",
      "provenance": { "nativeIds": ["git status"], "fieldConfidence": { "input_schema": "CONFIDENCE_INFERRED" } },
      "admission": { "state": "ADMISSION_STATE_SERVABLE",
                     "verificationRecordIds": ["vr_11c0…(L0)", "vr_9b7d…(L1)", "vr_3f2a…(L3)"],
                     "requiredRuntimeIsolation": "ISOLATION_CONFINED" },
      "core": true,
      "projections": { "mcpName": "git_status", "cliPath": "status" },
      "capabilityDigest": "<sha256>"
    },
    {
      "id": "git.add", "targetId": "git", "family": "index",
      "title": { "text": "Add file contents to the index", "origin": "CONTENT_ORIGIN_ENGINE" },
      "inputSchemaJson": "{\"additionalProperties\":false,\"properties\":{\"dry_run\":{\"type\":\"boolean\"},\"force\":{\"type\":\"boolean\"},\"pathspec\":{\"items\":{\"type\":\"string\"},\"minItems\":1,\"type\":\"array\"},\"repo\":{\"format\":\"path\",\"type\":\"string\"}},\"required\":[\"repo\",\"pathspec\"],\"type\":\"object\"}",
      "effects": { "mutatesTarget": "TRI_TRUE", "writesFilesystem": "TRI_TRUE", "launchesProcess": "TRI_FALSE",
                   "accessesNetwork": "TRI_FALSE", "mayDiscardData": "TRI_FALSE", "executesForeignCode": "TRI_FALSE",
                   "externalSideEffect": "TRI_FALSE", "irreversible": "TRI_FALSE", "idempotent": "TRI_TRUE",
                   "…other target fields": "TRI_FALSE (OBSERVED/OPERATOR as for git.status)" },
      "signals": [
        { "field": "mutates_target", "value": "TRI_TRUE", "confidence": "CONFIDENCE_INFERRED", "source": "rule:cli-verbs@1", "rationale": "leaf verb 'add'" },
        { "field": "writes_filesystem", "value": "TRI_TRUE", "confidence": "CONFIDENCE_OBSERVED", "source": "verify:vr_77ae…",
          "isolation": "ISOLATION_CONFINED", "rationale": "repo/.git/index modified" }
      ],
      "branches": [ { "pointer": "/force", "effects": { "mutatesTarget": "TRI_TRUE" },
                      "signals": [ { "field": "mutates_target", "value": "TRI_TRUE", "confidence": "CONFIDENCE_INFERRED", "source": "rule:cli-options@1" } ] } ],
      "level": "EFFECT_LEVEL_WRITE", "tier": "AUTHORITY_TIER_WRITE",
      "hints": { "idempotent": true },
      "scopes": [ { "pointer": "/repo", "kind": "SCOPE_KIND_WRITE_PATH" } ],
      "binding": { "cli": { "executableRole": "executable", "globalArgs": ["--no-optional-locks", "-c", "core.pager=cat", "-c", "color.ui=never"],
        "commandPath": ["add"],
        "args": [ { "pointer": "/force", "style": "ARG_STYLE_FLAG", "flag": "--force" },
                  { "pointer": "/dry_run", "style": "ARG_STYLE_FLAG", "flag": "--dry-run" },
                  { "pointer": "/pathspec", "style": "ARG_STYLE_POSITIONAL", "position": 0 } ],
        "cwdPointer": "/repo", "output": "OUTPUT_MODE_TEXT_STDOUT", "errors": "ERROR_CONVENTION_EXIT_CODE",
        "deadlineMs": 15000, "maxOutputBytes": "8388608", "killScope": "KILL_SCOPE_OWNED_GROUP", "usageExitCode": 129 } },
      "oracles": [ { "readBack": { "capabilityId": "git.status", "argsTemplateJson": "{\"repo\":{\"$input\":\"/repo\"}}",
                     "predicates": [ { "pointer": "/lines", "op": "PREDICATE_OP_CONTAINS", "valueJson": "{\"$input\":\"/pathspec/0\"}" } ] } } ],
      "admission": { "state": "ADMISSION_STATE_WITHHELD",
        "withheld": [ { "reason": "WITHHELD_REASON_NOT_VERIFIED", "hint": "WRITE needs L4 read-back (slice 2): aa verify git --lanes read_back" } ] },
      "projections": { "mcpName": "git_add", "cliPath": "add" },
      "capabilityDigest": "<sha256>"
    },
    { "id": "git.log", "level": "EFFECT_LEVEL_READ", "admission": { "state": "ADMISSION_STATE_SERVABLE" }, "…": "as git.status; args --max-count (integer, ARG_STYLE_OPTION => --max-count=5), --author" },
    { "id": "git.branch.list", "level": "EFFECT_LEVEL_READ", "…": "fixedArgs [--list, --format=%(refname:short)]" },
    { "id": "git.branch.delete", "level": "EFFECT_LEVEL_DESTRUCTIVE", "tier": "AUTHORITY_TIER_DESTRUCTIVE",
      "…": "may_discard_data TRUE (rule:cli-verbs@1 'delete'); branch /force raises irreversible; negative oracle via git.branch.list + bogus-id control; runtime: always ask" },
    { "id": "git.branch", "admission": { "state": "ADMISSION_STATE_WITHHELD",
      "withheld": [ { "reason": "WITHHELD_REASON_MIXED_AUTHORITY", "hint": "native command mixes list/create/delete; served via overlay mode splits git.branch.list / git.branch.delete" } ] } },
    { "id": "git.rev-parse", "admission": { "state": "ADMISSION_STATE_WITHHELD",
      "withheld": [ { "reason": "WITHHELD_REASON_MIXED_AUTHORITY", "hint": "--git-path/--show-toplevel are reads, but --local-env-vars et al. are prose-only; split by mode in the overlay" } ] } }
  ],
  "ruleVersions": { "help-text": "1", "cli-verbs": "1", "cli-options": "1", "exclusions": "1", "compile": "1" },
  "overlaySha256": "<sha256 of target.aa.yaml>",
  "lockDigest": "<sha256(JCS(lock with lockDigest=\"\"))>"
}
```

What happened on the way, as the engine records it (hermetic e2e test `git_status_side_effect_caught`):
1. The first compile had no `--no-optional-locks`.
2. L3 on `git.status` with a fixture whose file mtime moved showed `repo/.git/index MODIFIED`. The result was `findings: [SIDE_EFFECT]`,
   `writes_filesystem` TRUE (OBSERVED), level WRITE, withheld `NO_ORACLE`.
3. The operator added the global arg.
4. The capability digest changed, so admission reset, and L3 then passed.

The CLI-Anything LibreOffice lock (abridged) is in §11.

### 7.4 What is derived and never authored
`level`, `tier`, `hints`, MCP names, CLI flag specs, SKILL.md text, the catalog digest and the capability digest are all derived. A test recomputes them all from
the lock and compares (REA set-equality style).

---

## 8. Discovery harvesters

The `Harvester` trait lives in `aa-core`, with implementations in `aa-harvest` (mvp-first's split):
- `harvest()` is effectful. It runs through `ProcessRunner` and returns `RawHarvest` bytes, which are stored verbatim in the CAS.
- `candidates(&RawHarvest)` is **pure**, so CI replays recorded harvests with no Python, no git and no gh.

Every harvest is an audited pipeline action (`aa.pipeline.harvest.<id>`):
- STATIC harvesters are allowed for any registered target.
- INTROSPECT runs target code. It needs the target's authorization statement, writes intent before dispatch, and runs confined
  (network denied, private HOME/TMPDIR, scrubbed env, `stdin=/dev/null`, deadline).
- PROBE and EXPLORE exist only inside verification lanes.

| Harvester | Probe | Authority | Native effect signal | Milestone |
|---|---|---|---|---|
| `cli-anything-registry@1` | `registry.json` + harness dir + SKILL.md | STATIC | none | **slice 1** |
| `click-info@1` | `python -I aa_click_info.py <entry_point>` → unique root group → `to_info_dict()` | INTROSPECT (imports harness) | none (types only) | **slice 1** |
| `cobra-complete@1` | recursive `X __complete <path…> ""` and `X __complete <path…> --` for flags; `X <path> --help` for descriptions | INTROSPECT (some CLIs dial out, e.g. kubectl: confined) | enum values | **slice 1** |
| `help-text@1` | `X <cmd> -h`/`--help`, verbatim. Parsers: git parse-options, GNU getopt-style, Click/argparse usage | INTROSPECT | keywords (raise only) | **slice 1** |
| `swift-dump-help@1` | `X --experimental-dump-help` | INTROSPECT | none | M1 |
| `usage-kdl@1`, `json-fields@1` | usage spec; `X <cmd> --json` field list | STATIC / INTROSPECT | `effect:` (DECLARED if trusted) | M1 |
| `mcp-tools-list@1` | rmcp client `tools/list` | INTROSPECT | annotations (untrusted, raise only) | M2 |
| `openapi@1` | file or owned loopback URL | STATIC | HTTP method (DECLARED if operator-trusted) | M2 |
| `sdef@1`, `app-intents@1` | `.sdef` (+ access-groups), `extract.actionsdata` (`systemProtocols`) | STATIC | access groups, r/rw, DeleteEntity | M2 |
| `ax-menubar@1` | Swift adapter, enumerate only | INTROSPECT | none | M2 |
| `graphql@1`, `grpc-reflection@1`, `dbus@1` | `__schema`, reflection, `Introspect` | INTROSPECT | query/mutation, property access | M4+ |

**Two self-descriptions per target, diffed** (`[pilot §What to distill 5]`), for example Cobra `__complete` vs `--help`. A disagreement is
recorded as a finding attached to identity.

---

## 9. Verification ladder (ADR 0011)

| Lane | Does | Applies to | Emits | Non-claims |
|---|---|---|---|---|
| L0 static (`aa-core`) | closed schema; refs resolve; every target field decided or withheld with a hint; names unique; binding resolves against a harvested address; oracle refs a servable READ; no unresolved branch | all | pass/fail | runtime behaviour |
| L1 parser (`aa-exec`) | per param a wrong-typed value, plus once an unknown flag `--aa-probe-unknown`. Expect `usage_exit_code` (+ `usage_stderr_marker`) **and** no file change | CLI | `PARSER_MISMATCH` if accepted; detects `no_equals_form` | semantic correctness |
| L2 dry-run (slice 2) | run with `dry_run.argv`. For NO_PERSIST, the state-handle hash is unchanged | supported | pass/fail | preview matches the real run |
| L3 sandbox-diff (`aa-exec`) | build the fixture in a private root (`home/`, `cwd/`, `tmp/`). Run ≥ 2 distinct examples under the Confiner. blake3 tree-diff of `home/`+`cwd/`+state handle. Collect confinement denials. Check process-group survivors | all | OBSERVED signals per ADR 0011's lowering table; `SIDE_EFFECT` if observed ⊄ declared | effects on cloud state; anything above the lane's isolation |
| L4 read-back (slice 2) | oracle on the seed must **fail**; run the write; oracle must **pass**. FileOracle: magic + size. Negative: item absent after, and the bogus-id control fails | WRITE, DESTRUCTIVE | `NO_ORACLE`, `UNCONFIRMED`, `CONTRADICTED`, `UNEXPLAINED_CHANGE` | that a real backend rendered it unless a backend precondition binds it |
| L5 golden / L6 differential (M4) | replay goldens after an upgrade; two surfaces agree | — | drift | |
| L7 property / L8 agent eval (M4+) | schema-generated inputs; routing eval kept separate | — | | "a scripted sequence tests that sequence" |

**Admission matrix** (records bound to the current `lock_digest` + `identity_digest`):

| Level | Required PASSED lanes | Runtime |
|---|---|---|
| UNKNOWN | never | — |
| READ | L0, L1 (CLI), L3 | policy; served under `required_runtime_isolation` |
| UI | L0, L3, an a11y oracle (M2+) | policy (ask by default) |
| WRITE | READ set + L4 (+ L2 if supported) | policy within `write_roots`; idempotency key |
| DESTRUCTIVE | WRITE set (+ negative oracle for deletes) | always ask; never grantable |

Admission resets to WITHHELD on any change in `capability_digest` or `identity_digest`.

**Confinement (`aa-exec::confine`).** On macOS the `Confiner` emits a Seatbelt profile per run (this shape is verified):

```scheme
(version 1)
(allow default)
(deny network-outbound (require-not (remote unix-socket)))
(deny file-write* (require-not (require-any (subpath "<private-root>") (literal "/dev/null") <write-scope subpaths…>)))
(deny process-exec* (require-not (require-any <literal realpaths of pinned executable/interpreter/backends>)))
```

- If a confined run fails **because of** a denial, that is evidence the effect is TRUE. The denial is reported in
  `ProcessReport.confinement_denials`. The overlay can then add the backend to the pinned identity (for example `backend:soffice`), or
  the field stays TRUE.
- `sandbox-exec` is deprecated by Apple. That is a recorded risk, and the trait isolates it.
- On Linux, Landlock lands in M4. Until then Linux lanes run at `ISOLATION_PROCESS`, and the confined-only fields need `AssumeEffects`.

---

## 10. Renderers and front ends (ADR 0010)

- **`aa` CLI (slice 1).** It is a client of `agent.sock`. Operator subcommands use `operator.sock`.
  - Slice-1 commands:
    - `aa daemon` (execs `aad`);
    - `aa status`, `aa halt`, `aa unhalt`;
    - `aa target add <id> --exec <path> --harvester … --authorization …`;
    - `aa import cli-anything <repo> --harness <n> --python <venv-python> --authorization …`;
    - `aa discover <target>` (= harvest + compile);
    - `aa verify <target>`;
    - `aa list [target]`, `aa describe <cap>`;
    - `aa invoke <cap> --input-json <inline|file|->` (alias `aa x`);
    - `aa audit [--last N] [--verify]`.
  - Slice 2 adds `aa run <target> <path…> [flags]` (a dynamic clap tree from `GetExposure(CLI)`, cached by catalog digest), `aa
    compile --check`, `aa distill`, `aa propose context|apply`, `aa mcp`, `aa approve`, `aa doctor`.
  - Output is pretty JSON on a TTY and compact when piped. `--full-output` gives the envelope. Exit codes: 0/1/2/3/4/130/143/141 (§4).
- **`aa mcp`** (slice 2, `aa-cli` feature `mcp`, rmcp 3.8.0 stdio).
  - Profiles: `typed`, the servable core ≤ 30 per target with derived annotations and `_meta.aa`; and `search`, `aa_search`/`aa_describe`/`aa_invoke`,
    which is the default when more than 30 capabilities are servable.
  - Plus `aa_status`, which reports the catalog digest, halt state and counts.
  - Withheld capabilities are not tools; `aa_search` shows them with reasons. There are no approval tools.
  - Elicitation is relayed per §5.4. Without it, the response is `CONFIRMATION_REQUIRED` with "run `aa approve <id>`".
  - `WatchCatalog` drives `tools/list_changed`.
- **SKILL.md** (slice 2, `aa-core::render::skill`) is rendered per target with agentskills.io limits. It contains:
  - the core table with levels;
  - `aa x`/`aa run` fallbacks;
  - a withheld section with reasons;
  - "trust the live catalog";
  - `metadata.aa.lock_digest`/`catalog_digest`.

  Generated sections are marker-delimited so hand-written judgment can sit around them.
- **Catalog digest:** SHA-256 over JCS of the sorted exposure projection, per profile. It is exposed in `GetStatus` and `aa_status`.

---

## 11. CLI-Anything ingestion (ADR 0009)

- **Import is STATIC, from the local checkout.** It parses `registry.json` (79 entries) and the harness dir, hashes the tree, and records Apache-2.0.
  `install_cmd` is stored verbatim and **never run**. The harness SKILL.md is untrusted reference text only.
- **The interpreter is operator-installed**, for example `uv venv … && uv pip install <local harness dir>`. An engine-managed install is an M2
  pipeline action (`aa.pipeline.install`, DESTRUCTIVE, operator confirmation every time) because build backends execute code.
- **Introspect:** `click-info@1` runs confined. Root selection takes the unique group that is not a child of another group,
  otherwise it returns `ambiguous_root`. This handles `main()` wrappers, verified at `libreoffice_cli.py:831`.
- **Map:**
  - `--json` → `global_args`.
  - `--project` → engine-supplied state handle from the session workspace. The overlay may expose it, and it is then always path-scoped.
  - `--dry-run` → `DryRun{NO_PERSIST}`.
  - `{error,type}` → `UPSTREAM_ERROR` with the upstream type as `TargetText`.
  - `repl` and the bare group → `INTERACTIVE`.
  - Autosave rule (§6.4).
  - The binding executes `<venv-python> -I -m cli_anything.<name>`. The package has `__main__.py`, verified. Both are pinned in the
    identity together with `source_tree_sha`.
- **Backend:** `requires: libreoffice` becomes precondition `binary:soffice`. It resolves to a pinned
  `backend:soffice` FileIdentity, which is also put on the exec allow-list. On this Mac `/opt/homebrew/bin/soffice` is a shim and
  `/Applications/LibreOffice.app` is absent (generator-first, observed), so `export.render` for PDF presets → `PRECONDITION_UNMET`. Under
  confinement, an unpinned fallback exec is denied and recorded, not run silently.
- **Expected lock (abridged)** for `cli-anything.libreoffice`:
  - 40 leaf candidates;
  - `document.info`, `writer.list`, `calc.list-sheets`, `calc.get-cell`, `export.presets` → READ servable after L3;
  - `document.new`, `writer.add-paragraph` → WRITE, withheld `NOT_VERIFIED` until L4 (slice 2), then servable with oracle `writer.list` `COUNT_DELTA 1` + `CONTAINS {text: $input/text}`;
  - `*.remove*` → DESTRUCTIVE (ask);
  - `export.render` → WRITE with a `WRITE_PATH` scope on `output_path`, `launches_process` TRUE for non-ODF presets (branch on `/preset`), withheld until a proposal narrows `--preset`;
  - `session.*` and `repl` → excluded or interactive.

  ```json
  { "id": "cli-anything.libreoffice.writer.list", "level": "EFFECT_LEVEL_READ",
    "binding": { "cli": { "executableRole": "interpreter", "interpreterArgs": ["-I", "-m", "cli_anything.libreoffice"],
      "globalArgs": ["--json"], "stateHandleFlag": "--project", "stateHandlePointer": "/project",
      "stateHandleEngineSupplied": true, "commandPath": ["writer", "list"],
      "output": "OUTPUT_MODE_JSON_STDOUT", "errors": "ERROR_CONVENTION_CLI_ANYTHING_JSON",
      "killScope": "KILL_SCOPE_OWNED_GROUP", "usageExitCode": 2, "deadlineMs": 30000 } },
    "admission": { "state": "ADMISSION_STATE_SERVABLE", "requiredRuntimeIsolation": "ISOLATION_CONFINED" } }
  ```
- **Non-claims recorded:** "L4 does not establish that LibreOffice rendered this file" (the GIMP→Pillow lesson).
- `public_registry.json` and `matrix_registry.json` are suggestions only.
- **Any Click CLI** works the same way: `aa target add <id> --python-script <venv>/bin/<entry>`.

---

## 12. Persistence and audit (ADR 0006)

- **Files are the product.** Overlays, locks, verification records and packages are diffable files. The CAS holds immutable blobs.
- **The audit log is the runtime truth.**
  - It is `audit/segment-<n>.jsonl`. Each line is an `AuditEntry` with `seq`, `prev_sha256`, `entry_sha256 = sha256(JCS(entry minus hash))`,
    `principal`, `halt_epoch` and a body (`intent | action | admin`).
  - `audit/HEAD` mirrors the head hash and is updated by atomic rename.
  - There is a single writer task, and fsync happens on intents and on admin events.
  - The log is tamper-*evident*, not tamper-proof.
- **Exactly one ActionRecord per attempt after resolution.**
  - Intents come first for WRITE+ and INTROSPECT.
  - At boot, an intent without an outcome becomes `ActionRecord{OUTCOME_UNKNOWN, EXECUTION_STATE_UNKNOWN, recovered: true}`.
  - If chain verification fails → boot halted (`HALT_SOURCE_AUDIT_VERIFY_FAILED`).
- **Privacy:**
  - `args_commitment = HMAC-SHA256(state/hmac.key, JCS(input))` plus `input_redacted_json`, with `x-aa-secret` params blanked;
  - outputs go to the CAS;
  - stderr excerpts are bounded and redacted, including truncated token prefixes (REA `ProviderDiagnosticRedaction`).
- **Admin events:** registration (with the authorization statement), each assumed field, admission changes, halts, decisions, grants and policy loads.
- **Retention:** records are kept forever. `aa gc` removes CAS blobs that nothing references.
- **Slice 3 / M3:** a SQLite index (rusqlite 0.41.1 bundled, WAL) that is rebuildable from the JSONL, with `BEFORE UPDATE/DELETE RAISE(ABORT)`
  triggers.
- **Drift:** on an identity change the engine refuses Invoke for the target (`TARGET_IDENTITY_CHANGED`, `REDISTILL`). `aa distill`
  re-harvests and only changed capabilities lose admission.

---

## 13. The LLM boundary (slice 2, `aa-core::proposal`)

- **The loop:**
  1. `aa propose context <target>` (`GetProposalContext`) produces a bounded pack. All target text in it is `TargetText`, using spotlighting.
  2. The agent writes a `ManifestProposal`.
  3. `aa propose apply p.json` (`ApplyProposal`, agent role) submits it.
  4. Accepted ops are written into the overlay with `# proposal:<id>` comments for git review.
- **Validator rules.** Each is a unit-tested predicate, and cargo-mutants has a **blocking** score floor on this module:
  1. `base_lock_digest` must equal the current lock.
  2. Every capability id must be in the harvested set.
  3. `RaiseEffectOp` must be TRUE (or FALSE for `idempotent`), and `AddBranchOp` may only raise.
  4. Any key touching admission, grants, `assumed`, identity, preconditions or TRUE→FALSE is a **whole-proposal hard reject**.
  5. `RefineSchemaOp` may only narrow: the schema stays closed, `required` only grows, `enum` only shrinks, bounds only tighten, no widening,
     and unknown keywords are rejected. Any narrowing resets verification.
  6. Oracles must be declarative, and they stay unverified until L4 passes.
  7. Examples must validate and only ever run in lanes.
  8. Description budgets: 64, 1024 and 200 characters, no control characters, `CONTENT_ORIGIN_PROPOSAL`.
  9. Size limits: 1 MiB and 2,000 ops.
- **Hostile-proposal test table:** lower an effect; `additionalProperties: true`; widen a type; drop a `required`; invent an id; stale base;
  an oracle on a withheld read; a proposal containing `assumed`; an injected "ignore previous" description, which is stored as data and flagged.

---

## 14. Process supervision (ADR 0012, `aa-exec`)

- **Spawn:**
  - a new process group via `process-wrap` 10.0.1;
  - `stdin=/dev/null`;
  - `env_clear()` plus an allow-list (`PATH`, `LANG`, `LC_*`, binding `env`);
  - private 0700 root `{home,cwd,tmp}` from `tempfile`;
  - `AA_RUN_ID` set last, as an ownership marker and not a credential;
  - never a shell;
  - optional Confiner wrapping.
- **Deadline:** absolute. Then SIGTERM → 250 ms → SIGKILL, **of the owned group only**. A timed-out WRITE+ is `OUTCOME_UNKNOWN`.
- **Output:** stdout is capped at `max_output_bytes`. Overflow is drained and reported, never retained, and becomes `RESOURCE_CONSTRAINT`, not silent
  truncation. stderr keeps a head+tail excerpt and is redacted.
- **After exit:** survivors in the group give `CLEANUP_INCOMPLETE` plus cleanup debt, which blocks new dispatch to that target until a retry succeeds.
- **Drop guard:** the group is killed when the RPC future drops.
- **Unowned processes:** `open -a`-style LaunchServices children are recorded in `unowned_process_notes` and never signalled.
- **Argv hardening:**
  - options are always `--opt=value`, unless L1 proved `no_equals_form`. In that case values starting with `-` are refused;
  - positionals come after `--`;
  - argv is built from the validated input object only.
- Full PID-reuse-proof ownership verification of *foreign* processes is the REA kernel port, and is deferred to attach adapters (M2–M3).

---

## 15. Roadmap (mapped onto PLAN.md milestones M0–M4)

| Milestone | PLAN.md theme | Content here | Exit criterion |
|---|---|---|---|
| **M0** Decisions | ADRs; WSL2→Windows spike; licence review | this document + ADRs 0001–0013; `proto/` lint-clean; `build-plan.md`; licence review of the CLI-Anything harnesses we execute (Apache-2.0 NOTICE) and of LibreOffice (MPL-2.0); WSL2/Windows spike notes | judged; WP0 merged |
| **M1** Core loop (slice 1, then slice 2) | MCP skeleton; capability registry; adapter contract; authority declaration | **Slice 1** (build-plan): 7 crates; `cli_argv`; harvesters cli-anything/click/cobra/help-text; compile; L0/L1/L3 + Seatbelt; READ admission; gate with policy allow/deny + path scope + identity; JSONL audit + intent + boot-halt; Halt/Unhalt; `aa` over two UDS listeners; e2e on `git` (+ `gh` if present) and a Click fixture. **Slice 2:** L4 read-back + WRITE admission; L2; renderers (mcp-typed/search, skill-md, cli-tree); `aa mcp` (rmcp); `aa run`; `ApplyProposal` fence + `skills/aa-distill`; `compile/package --check` in CI; one-shot `AdapterService` framing | `git`, `gh` (read core), a Click harness and `cli-anything.libreoffice` reach READ-servable in < 10 min each (measured); `writer.add-paragraph` admitted by L4; SKILL.md + `tools/list` set-equal with the lock |
| **M2** Adapters | a11y; AppleScript/COM; CDP/Electron; PTY/CLI wrap; plugin bridge | long-lived `AdapterService` over UDS + Python/Swift kits + conformance; `mcp` and `http` bindings; harvesters `mcp-tools-list`, `openapi`, `sdef`, `app-intents`, `ax-menubar`, `swift-dump-help`; `Lifetime.SESSION`; CDP adapter for owned Electron/Node targets; engine-managed harness install as a gated pipeline action; UI level with an a11y oracle | Music READ via sdef with TCC as a `needs_human` precondition; one CDP write admitted by read-back; one in-host bridge (e.g. Blender Python) with a write admitted |
| **M3** Authority & audit depth | action records + audit; confirm-before-irreversible; allow/deny lists; kill switch | confirmations + `Resume` + `DecideConfirmation`; `RelayElicitation` with verified relay + a CI lane with a real pinned MCP client; grants lifecycle (issue/list/explain/revoke as one unit); session taint; circuit breaker; HALT sentinel + SIGUSR1; SQLite index with triggers; denial-reason → remediation test | destructive actions need a human on `operator.sock`; "issue → use → revoke" e2e; tamper test boots halted |
| **M4** Hardening & distribution | dependency pinning/npx trust; setup flow; verification lanes; docs | L5 golden + L6 differential; drift re-distill re-admits only unchanged capabilities; Linux Landlock Confiner; loopback TCP + Windows named pipes; Tier-1 setup via `aa doctor` (separate agent uid); pinned installs for MCP servers/npx; `server.json`/plugin packaging; L8 agent-eval lane | upgrading a harness re-admits only unchanged capabilities; Tier 1 documented and tested |

Explicitly deferred:
- code-mode exposure;
- EXPLORE-authority UI exploration (needs per-OS VM lanes, CRITIC P4);
- inject/dynamic instrumentation (own software only);
- streamable-HTTP MCP;
- vision fallback;
- multi-step recipes (`matrix_registry.json`);
- signed audit anchors.

---

## 16. Judge-raised flaws: what was fixed and how

| # | Flaw (judge) | Fixed? | How |
|---|---|---|---|
| 1 | generator-first: `ResolveApprovalRequest.channel` is client-asserted on a single socket, so any client can self-approve (all 3 judges) | **Fixed** | Two listeners. Role comes from the listener and is checked against the compiled `RPC_POLICY` table. `DecideConfirmation`, `IssueGrant`, `Unhalt` and `AssumeEffects` are operator-only, and there is no channel field anywhere (§5.1, ADR 0002/0005). A test asserts every operator RPC returns `PERMISSION_DENIED` on `agent.sock`. |
| 2 | generator-first: `SetKillSwitch(engaged=false)` open to any caller | **Fixed** | `Halt` is open to anyone. `Unhalt` is operator-only and needs the current `halt_epoch`. Halt state is persisted, and the engine boots halted on chain failure (§5.6). |
| 3 | generator-first: blanket `--accept-assumptions` lowers network/launch/foreign-code target-wide from the agent channel | **Fixed** | `AssumeEffects` is operator-only and per field. One audited AdminEvent per field. Compile accepts `assumed[]` only with a verifying `audit_seq`, and it is never proposable (§7.2). With Seatbelt confinement most such assumptions are unnecessary on macOS. |
| 4 | generator-first: `--install private-venv` runs `uv pip install` (build backend code) without a gate | **Fixed** | The slice never installs; the operator installs the venv. M2 adds `aa.pipeline.install` as a DESTRUCTIVE pipeline action that needs an operator confirmation every time (ADR 0009). |
| 5 | all: L3 at `ISOLATION_PROCESS` on macOS cannot see writes outside the root, so a false READ is possible | **Fixed** | Seatbelt `Confiner` (verified on this Mac). Lowering `writes_filesystem` (anywhere), `accesses_network` and `launches_process` requires `ISOLATION_CONFINED`. Capabilities are **served under the same confinement**. Path- or output-typed params are never lowered (ADR 0011). |
| 6 | generator-first: `AdmissionState` zero is UNSPECIFIED | **Fixed** | `ADMISSION_STATE_WITHHELD = 0`, along with fail-safe zeros on 15 safety enums. `ENUM_ZERO_VALUE_SUFFIX` is disabled deliberately, and a proptest guards it (ADR 0002). |
| 7 | safety-first: RelayElicitation self-approval by any `agent.sock` client | **Fixed (with stated residual)** | The relay must be the verified pinned `aa` binary (peer pid → exe sha256). Policy must list HOST_ELICITATION for the level (WRITE only, never DESTRUCTIVE). It is off in Tier ≥ 1 by default. Residual: an agent that runs the genuine `aa mcp` as its own client can answer elicitation, so it never covers DESTRUCTIVE (§5.4). |
| 8 | safety-first: `launches_process` counts the target CLI itself, plus source-grep floors, so nothing is ever read-only | **Fixed** | Binding-intrinsic carve-out (cli_argv + OWNED_GROUP + pinned exe only), with the exec allow-list enforcing it. Grep floors are rejected in favour of observation under confinement (§6.2). |
| 9 | safety-first: 18 crates / 14 in one day; full REA process port | **Fixed** | 7 crates. A focused `aa-exec` kernel of about 1–1.5k lines. The REA ownership port is deferred to attach adapters (ADR 0012/0013). |
| 10 | safety-first: OBSERVE admission needs L0+L1+L3 but the slice built only L0 | **Fixed** | Slice 1 builds L0, L1 and L3, and the acceptance runs the real admission path with no operator override. |
| 11 | safety-first: first-time introspection asks, repeating REA's #555 friction | **Fixed** | INTROSPECT is a gated, audited pipeline action. It is allowed without a per-run ask when the target was registered with an authorization statement, and refused otherwise (judge graft). |
| 12 | mvp-first: READ enablement is bare operator assertion, and nothing mutating before M3 | **Fixed** | READ is admitted by L3 evidence. WRITE is admitted by L4 in slice 2 (M1). Operator assertions exist only per field on `operator.sock`. |
| 13 | mvp-first: `ReviewCapability` lowering reachable from the agent socket with a client-side TTY check | **Fixed** | Every lowering path is either lane evidence (engine-run) or `AssumeEffects` on `operator.sock` (§6.2). |
| 14 | mvp-first: agent-supplied absolute `project_path` unscoped; harness opens it before any subcommand | **Fixed** | `--project` is engine-supplied from the 0700 session workspace by default. If the overlay exposes it, it is a `STATE_HANDLE` scope canonicalized and checked against PathScope (deny wins) (§11). |
| 15 | mvp-first: introspection runs harness code with no gating | **Fixed** | As #11, plus confinement and intent-before-dispatch. |
| 16 | mvp-first: MCP descriptions are raw target help, no cap | **Fixed** | `TargetText` everywhere. The MCP template has ≤ 300 vendor chars in an untrusted block (§5.6). |
| 17 | generator-first: one-day slice too wide (L0–L4, MCP, renderers) | **Fixed** | Slice 1 is L0/L1/L3 + READ + `aa` + audit. L4, MCP and renderers move to slice 2 (judge graft). |
| 18 | generator-first: root-group heuristic "most commands" | **Fixed** | A unique non-child group, else `ambiguous_root` (mvp-first rule). |
| 19 | safety-first: pbjson↔prost 0.14 marked UNVERIFIED; hyper-util/tower connector | **Resolved** | Verified by a scratch build. Native `unix://` endpoint (ADR 0001/0005). |
| 20 | all: single-uid approvals are not a boundary | **Stated, not "fixed"** | Assurance tiers are computed from proofs and recorded per decision. The Tier 1 setup lands in M4 (§5.2). |

---

## 17. Risks and open questions

1. **`sandbox-exec` deprecation.** Apple may remove it. *Mitigation:* the `Confiner` trait, with a fallback to `ISOLATION_PROCESS` plus per-field
   operator assumptions. Watch for Endpoint Security or app-sandbox alternatives (CRITIC P4 is still open).
2. **Over-conservatism starves the catalog.** *KPI:* "% servable after `aa distill` with no overlay edits", per harvester. Every withheld entry
   carries a specific hint.
3. **"Minutes" vs audited.** The pilot measured about 300 judgments for gh. Status and counters make the remaining work visible: unknown bits, unresolved branches,
   writes without an oracle.
4. **Host behaviour around annotations and elicitation is unverified** (CRITIC P3). The `aa mcp` design does not depend on hosts honouring
   `readOnlyHint`. The engine gate is authoritative.
5. **Two schema languages** (proto + JSON Schema). Narrowing works only over a supported subset, and the rest is rejected.
6. **Text-output oracles** (git) use line predicates over `{text, lines}`. This is weaker than typed JSON oracles. Prefer `--porcelain`/`--format` fixed args.
7. **GUI targets** need VM or user-account lanes for EXPLORE and for UI-level L3. These are deferred.
8. **Open:**
   - the specifier schema for sdef;
   - the App Intents type codes;
   - whether per-document vs per-app keys should be used for session grants;
   - how to compute a digest for GUI actions that depend on live UI state (include the pre-observation hash?).

   See `[pilot §Open questions]` and `[rea-removed-permission-system-archaeology §Open questions]`.
