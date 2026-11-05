# Proposal: generator-first agentic-anything (Rust gRPC engine)

- **Status:** draft proposal, one of several independent designs. To be judged and then merged into `docs/design/architecture.md`.
- **Lens:** generator/distillation-first. The goal is to make "add a new target app in minutes" true, while keeping "trusted to mutate" honest.
- **Date:** 2026-11-05. Toolchain checked: rustc 1.99.1, protoc 36.1 (libprotoc), buf 1.75.0.
- **Hard constraints honoured:**
  - The core is a Rust gRPC engine built on tonic and prost.
  - `aa` CLI, the MCP server and generated Agent Skills are clients of the engine.
  - CLI-Anything harnesses are one binding source among several.
  - Excluded: DRM or licence defeat, protected-asset extraction, and evading security controls.
- **Verified while writing:**
  - Every `.proto` block in §6 was extracted and passed `protoc`, `buf lint` (STANDARD), and a real `cargo build` with `tonic-prost-build 0.14.9` + `pbjson-build 0.9.0` (scratch crate, outside this repo).
  - The CLI-Anything facts in §10 come from running the LibreOffice harness on this machine in a scratch venv.

Research is cited as `[report §n]`, where the report is a file in `docs/research/`. REA source is cited as `rea:path:line` against `/Users/nhanclie/rea` at `3484432fb`.

---

## 0. TL;DR (the opinions)

1. **The product is the manifest, and the engine compiles and runs it.** Each target is a directory under version control:
   - a small curated overlay, `target.aa.yaml`, written by a human;
   - a deterministic compiled lock, `capabilities.lock.json`;
   - verbatim harvest artifacts in a content-addressed store;
   - verification records.
   
   Every front end (MCP, CLI, SKILL.md) is a rendering of the lock. Nothing is hand-written per capability. This answers the cost REA pays: about 12 files per tool and 7–10 shared seams per target (`[rea-capability-registry §Summary]`, `[rea-adding-a-new-target-walkthrough §Summary]`).
2. **The engine stays deterministic, and there is no LLM inside it.** LLM output enters through one RPC, `DistillService.ApplyProposal`. It takes a typed `ManifestProposal` whose ops can only rename, describe, group, raise effects, narrow schemas, pair oracles, add test examples, or exclude. Every op is validated mechanically. Nothing an LLM writes can lower an effect, widen a schema, enable a tool or grant authority (`[survey-capability-discovery-and-distillation §5]`).
3. **Effects use REA's boolean vector, made tri-state.** Missing means `UNKNOWN`, which policy treats as `TRUE`. Signals combine with max. Name heuristics and LLMs may only raise. Lowering needs `OBSERVED` lane evidence, a trusted `DECLARED` spec, or an attributed `OPERATOR` overlay entry. MCP hints are derived mechanically with REA's formula, and target effects are split from session effects (`rea:src/contracts/toolEffects.ts:2-22,765-787`; `[rea-tool-design-philosophy-and-verification §1.6]`).
4. **The verification ladder is code, and admission depends on the effect level.**
   - A READ capability is servable after the static, parser and sandbox-diff lanes.
   - A WRITE capability also needs a read-back oracle that fails on the seed and passes after the write.
   - DESTRUCTIVE capabilities additionally always ask at runtime.
   - UNKNOWN is never servable.

   This generalizes REA's confirmed-write admission from HAR traffic to every source (`[rea-desktop-applications-and-agent-environments §8.2]`, `rea:docs/agent-environments.md:175-186`).
5. **Generic bindings run in-process; foreign runtimes run out-of-process.**
   - The `cli-argv`, `mcp-tool`, `http` and `grpc` bindings are generic Rust executors driven by manifest data. A new target of those shapes needs zero code.
   - Anything that needs Python, Swift, Java or a host-app bridge runs out-of-process. It speaks `aa.adapter.v1` in one of two framings, both generated from the same `.proto`: gRPC over a 0600 Unix socket (long-lived), or a one-shot exchange of proto3-JSON over stdin/stdout.
6. **CLI-Anything is ingested, never trusted.**
   - We import from a local checkout. We introspect the real Click tree with `to_info_dict()` through a one-shot Python harvester. Click 8.5.0 yielded 40 typed leaf commands for LibreOffice on this machine.
   - We map CLI-Anything's `--json`, `--project` and `--dry-run` conventions onto the IR.
   - We **never** run `registry.json` `install_cmd` strings (`[survey-agent-interface-generators §2.7]`).
7. **Local security:**
   - The default transport is a UDS in a 0700 runtime dir, with socket mode 0600 and a peer-uid check through tonic's `UdsConnectInfo`.
   - Loopback TCP is opt-in and requires a bearer token stored in a 0600 file.
   - Nothing listens off-loopback in v1.
   - We say plainly that with a single uid, grants and approvals are guardrails, not a security boundary. A two-uid mode is the planned real boundary.
8. **The first slice** imports the CLI-Anything LibreOffice harness and makes it reachable through both `aa mcp` and `aa run`:
   - reads become servable after observed sandbox diffs;
   - `writer.add-paragraph` becomes servable only after its read-back oracle passes against `writer list`;
   - every call is recorded in a hash-chained SQLite action log.
   
   A Click fixture shaped like a CLI-Anything harness keeps CI hermetic.

---

## 1. Thesis

Software already describes itself, in machine-readable forms:
- Cobra `__complete`;
- Click `to_info_dict`;
- Swift ArgumentParser `--experimental-dump-help`;
- OpenAPI, GraphQL `__schema`, gRPC reflection;
- `.sdef`, App Intents `extract.actionsdata`, D-Bus `Introspect`;
- MCP `tools/list`.

These descriptions are mostly **unclassified**. They carry names and shapes but almost never effects (`[survey-capability-discovery-and-distillation §1.1-1.2]`).

The two existing families of agent-interface generators fail in opposite directions:
- **Deterministic projectors** (FastMCP `from_openapi`, `usage mcp`) are safe but shallow, and they mirror endpoints 1:1.
- **LLM synthesizers** (CLI-Anything, WALT, SkillWeaver) have reach but no effect model and no executable conformance checks. CLI-Anything's 52-check `validate` is a prompt, and its own GIMP harness breaks its rule #1 (`[survey-agent-interface-generators §2.5]`).

agentic-anything therefore should be a **compiler with a type system for authority**:

```
harvest (deterministic, authority-ranked)
  -> characterize (deterministic signal tables; missing = unknown)
  -> synthesize   (LLM *proposals* only, fenced by validators)
  -> verify       (lanes as code; admission by effect level)
  -> package      (renderers; catalog digest)
```

Time-to-first-target ("minutes") comes from four things:
- a harvester for the target's self-description format;
- the generic in-process bindings;
- renderers that need no per-target code;
- an overlay small enough to write by hand or to accept from an LLM proposal.

Trust ("may this mutate my document?") comes from three things:
- verification records bound to a lock digest and target identity;
- the admission matrix;
- the policy engine.

The design keeps these two clocks visibly separate. REA's own data shows that hardening a target costs 3–5× its first commit (`[rea-adding-a-new-target-walkthrough §Summary.3]`). Every target therefore carries `status: scaffolded | live-verified | hardened`, and every capability carries `admission: candidate | withheld{reasons} | servable`.

---

## 2. What "a new target in minutes" means

The table uses `gh` (a Cobra CLI) as the example, so it describes the post-M1 state. The slice (§15) covers the CLI-Anything path.

| Step | Command | Wall time (expected) | Output | Trust state |
|---|---|---|---|---|
| Declare | `aa target add gh --exec gh --authorization "..."` | seconds | `targets/gh/target.aa.yaml` | — |
| Harvest | `aa harvest gh` (Cobra `__complete`, scrubbed env) | seconds–1 min | raw artifacts in CAS, candidates | `candidate` |
| Characterize + compile | `aa compile gh` | < 1 s | `capabilities.lock.json` with effects, levels, hints | mostly `withheld: effects_unknown` |
| Propose (optional) | an agent runs `aa propose context gh`, then `aa propose apply p.json` | 1–5 min of agent time | overlay ops with provenance | still `withheld` |
| Verify reads | `aa verify gh --lanes static,parser,sandbox` | 1–3 min | verification records | reads become `servable` |
| Package | `aa package gh` | < 1 s | MCP tool list, SKILL.md, CLI tree | live in `aa mcp` and `aa run` |
| Writes | add a fixture + oracles, then `aa verify --lanes read_back` | 10–60 min (human review) | records | writes become `servable` |
| Harden | live lanes on new versions, goldens, differential | days (REA tail) | `status: hardened` | — |

`aa distill <target>` runs harvest → compile → verify → package with defaults.

"Minutes" is honest for the first six rows. The last two are where REA spent its tail, and we expose them as states rather than hiding them (`[rea-adding-a-new-target-walkthrough §What to distill.10]`).

---

## 3. The target workspace and manifest format

### 3.1 On-disk layout

```
$AA_HOME (default ~/.local/share/agentic-anything; or `aa --workspace ./aa` for project-local, committable)
  targets/<target-id>/
    target.aa.yaml               # OVERLAY: human-curated (and proposal-accepted) intent; small; committed
    capabilities.lock.json       # LOCK: compiled IR (proto3-JSON, RFC 8785 canonical); committed; lock_digest
    harvest/index.json           # SourceRefs -> CAS digests (what was harvested, how, at what authority)
    verification/<cap-id>/vr_<sha>.json   # VerificationRecord per lane run (bound to lock_digest)
    packages/
      mcp/tools.typed.json       # exact MCP projection (catalog_digest)
      skill/aa-<slug>/SKILL.md   # Agent Skill (+ references/<family>.md)
      cli/tree.json              # CLI projection
    proposals/<proposal-id>.json # accepted/rejected proposals, kept for review
  cas/sha256/ab/cdef...          # immutable raw harvest artifacts, large outputs, observations
  envs/<target-id>/              # engine-owned private venvs (CLI-Anything private installs)
  state.db                       # SQLite (WAL): action log hash chain, approvals, sessions, harvest runs
  config/policy.toml             # grants (or ~/.config/agentic-anything/policy.toml)
runtime dir: $XDG_RUNTIME_DIR/agentic-anything/ (Linux) | $TMPDIR/agentic-anything/ (macOS)
  engine.sock  engine.lock  [engine.token endpoint.json if TCP]  adapters/<run-id>/
```

Why overlay + lock, the Cargo.toml/Cargo.lock split:
- The lock is the **product**. It is reviewable in a git diff, and digest-addressed so verification records can bind to it.
- The overlay holds **only what a machine could not decide**: exclusions, state-handle conventions, oracle pairings, effect raises, core-set selection, operator assumptions and fixtures.
- This is the OpenAPI Overlays / `x-speakeasy-mcp` idea: keep curation outside the upstream spec (`[survey-agent-interface-generators §9 Adopt]`).
- Recompiling is a pure function: `lock = compile(harvest artifacts, overlay, rule-table versions)`. `aa compile --check` fails CI on drift, as REA's generators do with `--check` (`[rea-cli-surface-and-agent-packaging §What to distill.11]`).

### 3.2 Overlay example (the LibreOffice harness, abridged)

```yaml
schema: aa.overlay/v1
target: cli-anything.libreoffice
mcp_prefix: libreoffice
authorization: "Owner's workstation. LibreOffice MPL-2.0; harness Apache-2.0 (HKUDS/CLI-Anything)."
source:
  cli_anything: { repo: /Users/nhanclie/CLI-Anything, harness: libreoffice, install: private-venv }

# Operator assumptions: unobservable at ISOLATION_PROCESS, so a human must assert them.
# Recorded as CONFIDENCE_OPERATOR signals with the local user as author. Never proposable.
assume:
  accesses_network: false
  launches_process: false      # beyond the binding itself; export.render overrides below
  executes_foreign_code: false

state_handle: { flag: --project, param: project, kind: STATE_HANDLE }   # CLI-Anything convention
exclude:
  - { path: repl, reason: "interactive REPL; never invoked headless" }
  - { path: "session.*", reason: "undo stack is not persisted across one-shot invocations (observed undo_count=0)" }

fixtures:
  - id: empty-writer
    steps: [ { capability: document.new, input: { type: writer, name: Seed, output: { $root: /p.json } } } ]
    ready: document.info

capabilities:
  writer.add-paragraph:
    examples: [ { input: { project: { $root: /p.json }, text: "aa-probe-1" }, fixture: empty-writer } ]
    oracle:
      read_back:
        capability: writer.list
        args: { project: { $input: /project } }
        predicates:
          - { pointer: "", op: COUNT_DELTA, value: 1 }
          - { pointer: "", op: CONTAINS, value: { text: { $input: /text } } }
  writer.remove:
    effects: { may_discard_data: true }            # raise only
    oracle: { read_back: { capability: writer.list, args: { project: { $input: /project } },
              predicates: [ { pointer: "", op: COUNT_DELTA, value: -1 } ] } }
  export.render:
    effects: { launches_process: true }            # soffice for non-ODF presets
    oracle: { file: { path: /output_path, magic: "PK\u0003\u0004", min_size: 1000 } }   # odt/ods/odp
core: [document.new, document.info, writer.add-paragraph, writer.add-heading, writer.list, export.render]
```

### 3.3 Lock (one capability, abridged proto3-JSON)

```json
{
  "id": "cli-anything.libreoffice.writer.add-paragraph",
  "family": "writer",
  "inputSchemaJson": "{\"additionalProperties\":false,\"properties\":{\"alignment\":{\"enum\":[\"left\",\"center\",\"right\",\"justify\"]},\"bold\":{\"type\":\"boolean\"},\"font_size\":{\"type\":\"string\"},\"italic\":{\"type\":\"boolean\"},\"position\":{\"type\":\"integer\"},\"project\":{\"format\":\"path\",\"type\":\"string\"},\"text\":{\"default\":\"\",\"type\":\"string\"}},\"type\":\"object\"}",
  "effects": {
    "mutatesTarget": "TRI_TRUE", "writesFilesystem": "TRI_TRUE", "launchesProcess": "TRI_FALSE",
    "accessesNetwork": "TRI_FALSE", "mayDiscardData": "TRI_FALSE", "idempotent": "TRI_FALSE",
    "signals": [
      { "field": "mutates_target", "value": "TRI_TRUE", "confidence": "CONFIDENCE_INFERRED", "source": "rule:cli-verbs@1", "rationale": "leaf verb 'add'" },
      { "field": "writes_filesystem", "value": "TRI_TRUE", "confidence": "CONFIDENCE_INFERRED", "source": "rule:cli-anything-autosave@1" },
      { "field": "may_discard_data", "value": "TRI_FALSE", "confidence": "CONFIDENCE_OBSERVED", "source": "verify:vr_7c1e..." },
      { "field": "accesses_network", "value": "TRI_FALSE", "confidence": "CONFIDENCE_OPERATOR", "source": "overlay:assume" }
    ]
  },
  "level": "EFFECT_LEVEL_WRITE",
  "hints": { "readOnly": false, "destructive": false, "idempotent": false, "openWorld": false },
  "binding": { "cli": { "argvPrefix": ["--json"], "commandPath": ["writer", "add-paragraph"],
               "stateHandlePointer": "/project", "stateHandleFlag": "--project",
               "args": [ { "pointer": "/text", "style": "ARG_STYLE_OPTION", "flag": "--text" } ],
               "output": "OUTPUT_MODE_JSON_STDOUT", "errors": "ERROR_CONVENTION_CLI_ANYTHING_JSON",
               "deadlineMs": 30000, "maxOutputBytes": "8388608" } },
  "dryRun": { "supported": true, "argv": ["--dry-run"], "kind": "DRY_RUN_KIND_NO_PERSIST" },
  "admission": "ADMISSION_STATE_SERVABLE",
  "projections": { "mcpName": "libreoffice_writer_add_paragraph", "cliPath": "writer add-paragraph" }
}
```

---

## 4. IR semantics (what the engine enforces)

### 4.1 Identifiers
- A target id matches `[a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*`, for example `gh` or `cli-anything.libreoffice`.
- A capability id is `<target>.<noun-path>.<verb>`. It is derived from the native address (Click path, Cobra path, `operationId`, sdef code), and the native ids are kept in `provenance.native_ids` (`[survey-capability-discovery-and-distillation §4.4]`).
- The MCP name is derived as `<mcp_prefix>_<noun>_<verb>`, restricted to `[a-zA-Z0-9_-]`, at most 64 characters. Collisions get a deterministic 6-hex suffix (from FastMCP's truncation rule).
- A rename is a proposal op that keeps the native id.

### 4.2 Effects lattice (code in `aa-ir::effects`)
- **Combination.** For each field, the engine takes the max over all *admissible* signals, ordered `UNKNOWN < FALSE < TRUE`. A `FALSE` signal is admissible only from the sources listed in the table below; a `FALSE` from an `INFERRED` or `PROPOSED` source is recorded but ignored. `idempotent` is the exception: it combines with AND, and `UNKNOWN` counts as `FALSE`. A field with no admissible signal is `UNKNOWN`. Effects are not inherited down a command tree, and flags and parameters can only raise them, following the usage spec (`[survey-capability-discovery-and-distillation §3.2 Tier 0]`).
- **Who may move a field:**

| Transition | Allowed sources |
|---|---|
| `UNKNOWN → TRUE` (raise) | any: harvester native signal, rule table, proposal, lane observation, operator |
| `UNKNOWN → FALSE` | `DECLARED` from a spec the operator marked trusted; `OBSERVED` from a lane whose isolation can observe that field; `OPERATOR` (overlay `assume` or per-capability, attributed) |
| `TRUE → FALSE` | `OPERATOR` with `override_reason` only. Never a proposal. Never a lane: "absence of evidence is not evidence" (`[survey-capability-discovery-and-distillation §4.3 rule 4]`) |

- **Binding-intrinsic execution is not an effect.** Running the binding's identity-pinned executable in an owned, deadline-bounded process is covered by the target grant. Without this rule every CLI tool is `launchesProcess` and nothing is read-only. It mirrors REA's carve-out for "REA's own pinned helpers or operator-configured analysis engines" (`rea:src/contracts/toolEffects.ts:12-18`).
- **Projections** (derived, never authored):
  - `level`:
    - `DESTRUCTIVE` if any of `may_discard_data | executes_foreign_code | mutates_device | external_side_effect` is TRUE;
    - else `WRITE` if any of `mutates_target | writes_filesystem | launches_process` is TRUE;
    - else `UI` if `changes_ui_state | sends_input` is TRUE;
    - else `READ` if every target-facing field is FALSE;
    - `UNKNOWN` if any target-facing field is still UNKNOWN.
  - `hints` use REA's formula. `readOnly` is computed from **target-facing** fields only, so session recording does not make every observation non-read-only. REA found 109 of 234 tools non-read-only for that reason alone (`[rea-tool-design-philosophy-and-verification §1.6, §7.4]`). An UNKNOWN in `destructive` or `openWorld` inputs counts as TRUE.
  - `authority.tier` follows from the level and from binding kind (UI drive → DRIVE; attach-style adapters → ATTACH).
- **Untrusted text.** Descriptions, enum labels and help text harvested from the target set `description_untrusted`. Keyword rules may raise effects from that text, and nothing in it can lower an effect (`[survey-capability-discovery-and-distillation §4.3 rule 5]`). `returns_target_content` defaults to true; this is REA's allowlist inverted (`rea:src/contracts/toolEffects.ts:789-816`).

### 4.3 Admission matrix (code in `aa-verify::admission`)

| Level | Required PASSED lanes (records bound to the current `lock_digest` and target identity) | Runtime |
|---|---|---|
| UNKNOWN | never servable (`withheld: effects_unknown`, with a hint naming the undecided fields) | — |
| READ | L0 static, L1 parser (CLI bindings), L3 sandbox-diff | policy allow |
| UI | L0, L3 + an a11y oracle (M3) | policy allow |
| WRITE | L0, L1, L3, **L4 read-back** (+ L2 if `dry_run.supported`) | policy allow; idempotency key required |
| DESTRUCTIVE | the WRITE set | **always ask** (confirmation challenge with dry-run preview when available) |

Any of these resets admission to `candidate`:
- a lock change that touches a capability's schema, binding, effects or oracle;
- a change in target identity.

This is REA's catalog-identity discipline applied per capability (`[rea-capability-registry §Summary]`, `rea:src/catalogIdentity.ts:43-56`).

---

## 5. Cargo workspace layout

```
Cargo.toml              # [workspace] resolver = "3"; edition = "2024"; rust-version = "1.88"
rust-toolchain.toml     # channel = "1.99.1"
buf.yaml                # lint STANDARD, breaking WIRE_JSON (JSON is an on-disk format, so JSON-compat matters)
proto/aa/ir/v1/{ir,record}.proto  proto/aa/engine/v1/engine.proto  proto/aa/adapter/v1/adapter.proto
crates/<see table>
adapters/python/aa_adapter_kit/   adapters/python/aa_click_harvest.py   adapters/swift/ (M3)
targets/                # committed examples: fixture-notes, cli-anything.libreoffice
skills/aa-distill/      # hand-authored skill teaching an agent the propose loop
```

Crate versions were checked with `cargo search` / `cargo info` on 2026-11-05. `rmcp 3.8.0` and `tonic-prost-build 0.14.9` both declare `rust-version 1.88`.

| Crate | Responsibility | Key dependencies (version) |
|---|---|---|
| `aa-proto` | Generated types only. `build.rs` runs `tonic_prost_build::configure().file_descriptor_set_path(..).compile_well_known_types(true).extern_path(".google.protobuf","::pbjson_types").compile_protos(..)`, then `pbjson_build` for serde. No logic. (This exact recipe compiled the §6 protos.) | tonic 0.14.9, tonic-prost 0.14.9, prost 0.14.4, prost-types 0.14.4, pbjson 0.9.0, pbjson-types 0.9.0; build: tonic-prost-build 0.14.9, pbjson-build 0.9.0 |
| `aa-ir` | IR semantics: id grammar, RFC 8785 canonical JSON and digests, effects lattice and projections, closed-schema checks, overlay parse (YAML) and lock I/O, admission matrix types | serde 1.0.229, serde_json 1.0.154, serde_jcs 0.2.0, sha2 0.11.1, jsonschema 0.60.2, jsonptr 0.8.2, serde_yaml_ng 0.10.0 (swappable), thiserror 2.0.21 |
| `aa-harvest` | `Harvester` trait; CAS writes; built-in harvesters behind cargo features (`cli-anything`, `click`, `cobra`, `swift-dump-help`, `help-text`, `usage-kdl`, `mcp`, `openapi`, `graphql`, `grpc`, `sdef`, `app-intents`, `dbus`) | aa-exec, kdl 6.7.1, rmcp 3.8.0 (`client`,`transport-child-process`), openapiv3 2.2.0 / oas3 0.22.0, graphql-parser 0.4.1, tonic-reflection 0.14.9 + prost-reflect 0.16.5, roxmltree 0.21.2, zbus 6.2.1 (linux) |
| `aa-characterize` | Candidates → capabilities: schema projection per source, **signal tables as code** (one module per source, versioned `rule:<name>@n`), verb/option keyword raise rules, oracle-pairing heuristics, naming, core-set defaults | aa-ir, regex 1.13.1, aho-corasick 1.1.5 |
| `aa-proposal` | The LLM fence: validates `ManifestProposal` ops (§9.3), schema-narrowing checker, overlay patching with provenance | aa-ir, json-patch 5.0.0 |
| `aa-exec` | Process supervision kernel: owned spawn in its own process group, run-token env marker, absolute deadlines, bounded stdout/stderr, private runtime roots (cwd/HOME/TMPDIR), env scrubbing allowlist, poison-on-timeout, cleanup report (`[rea-engine-bridges-and-process-supervision §What to distill.3-4,9]`) | tokio 1.54.1, process-wrap 10.0.1, rustix 1.1.6, tempfile 3.27.0, which 8.0.6 |
| `aa-bind` | In-process binding executors behind one `Binding` trait: `cli-argv` (slice), `mcp-tool` (M1), `http` (M1), `grpc` dynamic (M5), `adapter` (delegates to aa-adapter-host) | aa-exec, rmcp 3.8.0, reqwest 0.13.5, prost-reflect 0.16.5 |
| `aa-adapter-host` | Out-of-process adapter lifecycle: private dir, 0600 descriptor handoff, one-shot runner (JSONL framing, reserved exit codes), UDS gRPC client, handshake comparison, receipts, conformance suite | aa-exec, aa-proto, tonic 0.14.9, hyper-util 0.1.21, tower 0.5.3, rand 0.10.3 |
| `aa-verify` | Lanes L0–L8 as code, file-tree snapshot/diff (blake3), oracle predicate evaluation, fixture builder, `VerificationRecord` writer, admission computation | aa-bind, walkdir 2.5.0, blake3 1.8.7, similar 3.2.1 |
| `aa-policy` | Grants (`policy.toml`), deny > ask > allow over structured descriptors, path/host scope canonicalization, approval challenges and single-use HMAC tokens, kill switch, circuit breaker | toml 1.2.0, globset 0.4.21, hmac 0.13.0, subtle 2.6.1, zeroize 1.9.1 |
| `aa-store` | Workspace FS layout, CAS, SQLite (action-log hash chain, approvals, sessions, harvest runs), single-writer task | rusqlite 0.41.1 (`bundled`), ulid 3.0.0, fd-lock 4.0.4, etcetera 0.11.0 |
| `aa-render` | Renderers from the lock: `mcp-typed`, `mcp-search`, `cli-tree`, `skill-md`, `usage-kdl`, `server-json`; catalog digest; `--check` drift | minijinja 3.0.5, kdl 6.7.1 |
| `aa-engine` | tonic service impls and the composition root; UDS/TCP/in-process listeners; peer-uid interceptor; tonic-health and tonic-reflection on the engine itself (so `buf curl` works, and the gRPC harvester is dogfooded against `aad`) | all above, tonic-health 0.14.9, tonic-reflection 0.14.9, tokio-stream 0.1.19, tracing 0.1.44 |
| `aa-client` | Typed client: connect over UDS, TCP+token, or an in-process `tokio::io::duplex`; autostart `aa daemon` with a lock-guarded race | aa-proto, tonic, hyper-util |
| `aa-mcp` | rmcp `ServerHandler` with manual `list_tools`/`call_tool` from `GetExposure`/`Invoke`; elicitation bridge | rmcp 3.8.0 (`server`,`transport-io`,`elicitation`), aa-client |
| `aa-cli` | The single `aa` binary: engine-ops subcommands, dynamic `aa run` tree, `aa daemon`, `aa mcp` | clap 4.6.7, clap_complete 4.6.11, aa-client, aa-mcp, aa-engine (only for `daemon`/`--embedded`), tracing-subscriber 0.3.23, anyhow 1.0.104 |
| `aa-testkit` | Fixture targets (a Click app shaped like a CLI-Anything harness; later a clap app), recorded raw harvest fixtures, an MCP-client test driver | assert_cmd 2.2.2, predicates 3.1.4, insta 1.49.0, proptest 1.12.0, wiremock 0.6.5 |

Dev tooling: cargo-nextest 0.9.151, cargo-mutants 27.1.0, buf 1.75.0.

**Dependency rule**, borrowed from REA's inward-flowing layers:
- `aa-proto ← aa-ir ← {harvest, characterize, proposal, policy, store, render}`.
- `aa-exec` has no aa deps.
- `aa-bind ← aa-exec`.
- `aa-verify ← aa-bind`.
- `aa-engine` is the only crate that sees everything.
- `aa-client`, `aa-mcp` and the non-daemon half of `aa-cli` depend on `aa-proto` only, never on engine internals.

A test parses `cargo metadata` and fails on violations. This is the Rust equivalent of REA's module-boundary lint (`[rea-capability-registry §What to distill]`).

---

## 6. The gRPC API (draft `.proto`, validated)

Conventions:
- **Domain errors are values.** Responses carry `aa.ir.v1.ErrorProjection`. gRPC status is reserved for transport and authentication failures (`UNAUTHENTICATED`, `PERMISSION_DENIED` on a peer-uid mismatch, `UNAVAILABLE`, `UNIMPLEMENTED`, `INTERNAL`). REA does the same: commands return error projections as values, and one classifier sets the exit status (`[rea-cli-surface-and-agent-packaging §Summary]`).
- **JSON.** Every message is serde-able through pbjson. Lock files, `aa --json`, MCP `_meta` and the one-shot adapter framing all use proto3-JSON. Digests are SHA-256 over JCS(pbjson output).
- **Streaming** is used only where progress matters: `Harvest`, `Verify`, `Invoke`. The terminal event is always one of `result`, `error` or `confirmation`.
- **Unimplemented RPCs** return `UNIMPLEMENTED` in early milestones. The surface is fixed now so that clients and adapters can be written against it.

### 6.1 `aa/ir/v1/ir.proto`: Target, Capability, Effects, Authority, Binding, Oracle

```proto
syntax = "proto3";

// The capability IR: the one source every renderer, policy check and
// verification lane reads. On disk it is proto3-JSON (canonical, RFC 8785).
package aa.ir.v1;

import "google/protobuf/timestamp.proto";

// ---------- shared scalars ----------

// Every effect field is tri-state. A missing field is UNKNOWN, and policy
// treats UNKNOWN as TRUE ("absence = ask", usage spec; REA "unknown, not absence").
enum Tri {
  TRI_UNSPECIFIED = 0; // == unknown
  TRI_FALSE = 1;
  TRI_TRUE = 2;
}

// How a field value is known. Ordered from strongest to weakest for lowering.
enum Confidence {
  CONFIDENCE_UNSPECIFIED = 0;
  CONFIDENCE_OBSERVED = 1; // seen in a verification lane (sandbox diff, read-back)
  CONFIDENCE_DECLARED = 2; // machine-readable self-description (typed spec)
  CONFIDENCE_OPERATOR = 3; // a human overlay edit, attributed
  CONFIDENCE_DERIVED = 4; // computed from other fields (projections)
  CONFIDENCE_INFERRED = 5; // heuristics: keywords, help-text regex, verb tables
  CONFIDENCE_PROPOSED = 6; // LLM proposal; can only raise
}

enum DiscoveryAuthority {
  DISCOVERY_AUTHORITY_UNSPECIFIED = 0;
  DISCOVERY_AUTHORITY_STATIC = 1; // read files only
  DISCOVERY_AUTHORITY_INTROSPECT = 2; // call self-description; may run target code
  DISCOVERY_AUTHORITY_PROBE = 3; // invoke with test args, sandbox lane only
  DISCOVERY_AUTHORITY_EXPLORE = 4; // drive UI / write state, sandbox lane only
}

// Invocation authority lattice (projection of effects + binding; never hand-set).
enum AuthorityTier {
  AUTHORITY_TIER_UNSPECIFIED = 0; // == unknown, treated as highest
  AUTHORITY_TIER_OBSERVE = 1;
  AUTHORITY_TIER_LAUNCH = 2;
  AUTHORITY_TIER_DRIVE = 3; // UI input into an allow-listed window
  AUTHORITY_TIER_WRITE = 4; // additive/overwriting change inside declared scope
  AUTHORITY_TIER_DESTRUCTIVE = 5; // discard, external side effect, foreign code
  AUTHORITY_TIER_ATTACH = 6; // debugger / hooks into a running process
  AUTHORITY_TIER_MUTATE_SYSTEM = 7; // install, registry, OS settings
}

enum EffectLevel {
  EFFECT_LEVEL_UNSPECIFIED = 0; // == unknown
  EFFECT_LEVEL_READ = 1;
  EFFECT_LEVEL_UI = 2;
  EFFECT_LEVEL_WRITE = 3;
  EFFECT_LEVEL_DESTRUCTIVE = 4;
}

enum Isolation {
  ISOLATION_UNSPECIFIED = 0; // not proven
  ISOLATION_NONE = 1;
  ISOLATION_PROCESS = 2; // private cwd/HOME/TMP, scrubbed env, same uid
  ISOLATION_CONTAINER = 3;
  ISOLATION_VIRTUAL_MACHINE = 4;
}

// ---------- effects ----------

// REA ToolEffects (src/contracts/toolEffects.ts:2-22), split target vs session,
// extended for driving. Running the binding's own identity-pinned executable in
// an owned, deadline-bounded process is NOT an effect; the target grant covers it.
message Effects {
  // target-facing (drive readOnlyHint)
  Tri mutates_target = 1;
  Tri writes_filesystem = 2; // outside the engine's private runtime root
  Tri launches_process = 3; // beyond the binding's own invocation
  Tri accesses_network = 4;
  Tri changes_ui_state = 5;
  Tri may_discard_data = 6;
  Tri executes_foreign_code = 7; // code outside the declared binding: scripts, macros, plugins
  Tri mutates_device = 8;
  Tri sends_input = 9; // synthetic keyboard/mouse/AX actions
  Tri reads_screen = 10;
  Tri reads_clipboard = 11;
  Tri reads_credentials = 12;
  Tri external_side_effect = 13; // send, publish, pay: irreversible outside the host
  // control-plane-facing (exposed in _meta, never in readOnlyHint)
  Tri mutates_session = 14;
  // "safe" direction is TRUE; combination is AND, unknown -> FALSE
  Tri idempotent = 15;
  // Every signal that contributed, so the lock is auditable.
  repeated EffectSignal signals = 16;
}

message EffectSignal {
  string field = 1; // Effects field name, e.g. "may_discard_data"
  Tri value = 2;
  Confidence confidence = 3;
  string source = 4; // "harvest:click@1", "rule:verb-table@3", "overlay", "proposal:<id>", "verify:<record_id>"
  string rationale = 5;
  string evidence_id = 6; // optional, ev_<sha256>
}

// Derived, never stored by hand. Formula in aa-ir::effects (REA annotationsFromEffects).
message McpHints {
  bool read_only = 1;
  bool destructive = 2;
  bool idempotent = 3;
  bool open_world = 4;
}

// ---------- target ----------

enum TargetKind {
  TARGET_KIND_UNSPECIFIED = 0;
  TARGET_KIND_CLI_TOOL = 1;
  TARGET_KIND_INSTALLED_APP = 2;
  TARGET_KIND_RUNNING_WINDOW = 3;
  TARGET_KIND_SERVICE_API = 4;
  TARGET_KIND_INSTALL_DIR = 5;
}

// REA walkthrough: "scaffolded | live-verified | hardened".
enum TargetStatus {
  TARGET_STATUS_UNSPECIFIED = 0;
  TARGET_STATUS_SCAFFOLDED = 1;
  TARGET_STATUS_LIVE_VERIFIED = 2;
  TARGET_STATUS_HARDENED = 3;
}

message TargetIdentity {
  string executable_path = 1; // realpath
  string executable_sha256 = 2;
  string version_banner = 3;
  string bundle_id = 4;
  string origin = 5; // service base URL (loopback/owned only)
  string spec_sha256 = 6; // digest of the harvested self-description
  string source_tree_sha = 7; // e.g. git tree of a CLI-Anything harness dir
  string interpreter = 8; // e.g. venv python realpath for Python CLIs
}

message License {
  string spdx = 1;
  string url = 2;
  bool bring_your_own = 3;
  string notes = 4;
}

message Precondition {
  string id = 1; // "binary:soffice", "file:project", "running:false"
  string description = 2;
  Confidence confidence = 3;
  string check_capability_id = 4; // optional read capability that tests it
}

message SourceRef {
  string harvester = 1; // "cli-anything-registry@1", "click-info@1"
  string uri = 2;
  string sha256 = 3; // digest of the verbatim raw artifact in harvest/
  DiscoveryAuthority authority = 4;
  google.protobuf.Timestamp harvested_at = 5;
}

message Target {
  string id = 1; // grammar: [a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*
  string display_name = 2;
  TargetKind kind = 3;
  TargetIdentity identity = 4;
  License license = 5;
  repeated Precondition preconditions = 6;
  repeated SourceRef sources = 7;
  TargetStatus status = 8;
  string catalog_digest = 9; // sha256 over the rendered exposure (catalog identity)
  string lock_digest = 10; // sha256 over capabilities.lock.json
  repeated string families = 11;
  // Hard exclusion acknowledgement: owner asserts ownership/authorization.
  string authorization_statement = 12;
  repeated Fixture fixtures = 13; // seed-state recipes for sandbox lanes
  string mcp_prefix = 14; // short alias used in rendered tool names, e.g. "libreoffice"
}

// A seed state built inside the lane's private root before each verification run.
// Steps may use not-yet-servable capabilities: they only ever run in the sandbox.
message Fixture {
  string id = 1;
  repeated FixtureStep steps = 2;
  string ready_capability_id = 3; // read that must succeed on the built seed
}

message FixtureStep {
  string capability_id = 1;
  string input_json = 2; // may use {"$root": "/p.json"} for paths in the private root
}

message Example {
  string input_json = 1;
  string fixture_id = 2;
  string source = 3; // "overlay", "proposal:<id>", "harvest"
}

// ---------- capability ----------

enum Lifetime {
  LIFETIME_UNSPECIFIED = 0;
  LIFETIME_ONESHOT = 1;
  LIFETIME_SESSION = 2; // requires session_id
  LIFETIME_STREAM = 3; // start/poll/cancel shape
}

enum AdmissionState {
  ADMISSION_STATE_UNSPECIFIED = 0;
  ADMISSION_STATE_CANDIDATE = 1; // harvested, not characterized
  ADMISSION_STATE_WITHHELD = 2; // characterized but a gate failed (see reasons)
  ADMISSION_STATE_SERVABLE = 3; // ladder requirement for its level met
}

// REA agent-environments withheld reasons (docs/agent-environments.md:175-186), generalized.
enum WithheldReason {
  WITHHELD_REASON_UNSPECIFIED = 0;
  WITHHELD_REASON_EFFECTS_UNKNOWN = 1;
  WITHHELD_REASON_NO_ORACLE = 2;
  WITHHELD_REASON_UNCONFIRMED = 3;
  WITHHELD_REASON_CONTRADICTED = 4;
  WITHHELD_REASON_SIDE_EFFECT = 5; // observed effect not declared
  WITHHELD_REASON_UNEXPLAINED_CHANGE = 6;
  WITHHELD_REASON_NO_OBSERVATION = 7;
  WITHHELD_REASON_SCHEMA_INVALID = 8;
  WITHHELD_REASON_PARSER_MISMATCH = 9;
  WITHHELD_REASON_PRECONDITION_UNMET = 10; // e.g. backend binary missing
  WITHHELD_REASON_EXCLUDED_BY_POLICY = 11; // licence/consent/DRM surfaces
  WITHHELD_REASON_INTERACTIVE = 12; // REPL/TTY-only, cannot run headless
}

message Withheld {
  WithheldReason reason = 1;
  string hint = 2; // what to record/do next; never changes what is served
  string evidence_id = 3;
}

message Provenance {
  repeated string source_refs = 1; // SourceRef.sha256 values
  repeated string native_ids = 2; // vendor ids: click path, operationId, sdef code
  map<string, Confidence> field_confidence = 3; // "input_schema" -> DECLARED
  repeated string proposal_ids = 4; // accepted ManifestProposal ops that touched it
}

message Authority {
  AuthorityTier tier = 1; // projected
  repeated ScopeParam scopes = 2; // which params designate touched resources
}

message ScopeParam {
  string pointer = 1; // JSON pointer into input, e.g. "/output"
  ScopeKind kind = 2;
}

enum ScopeKind {
  SCOPE_KIND_UNSPECIFIED = 0;
  SCOPE_KIND_READ_PATH = 1;
  SCOPE_KIND_WRITE_PATH = 2;
  SCOPE_KIND_HOST = 3;
  SCOPE_KIND_WINDOW = 4;
  SCOPE_KIND_STATE_HANDLE = 5; // e.g. CLI-Anything --project file
}

message DryRun {
  bool supported = 1;
  // How the binding expresses it, e.g. extra argv ["--dry-run"].
  repeated string argv = 2;
  // NO_PERSIST: computes but does not save (CLI-Anything); PREVIEW: reports plan (-WhatIf).
  DryRunKind kind = 3;
}

enum DryRunKind {
  DRY_RUN_KIND_UNSPECIFIED = 0;
  DRY_RUN_KIND_PREVIEW = 1;
  DRY_RUN_KIND_NO_PERSIST = 2;
}

message Capability {
  string id = 1; // "<target>.<noun>.<verb>", e.g. "cli-anything.libreoffice.writer.add-paragraph"
  string target_id = 2;
  string title = 3;
  string description = 4;
  bool description_untrusted = 5; // text came from the target
  string family = 6;
  string input_schema_json = 7; // closed JSON Schema 2020-12, canonical JSON
  string output_schema_json = 8; // "" = unknown
  Effects effects = 9;
  EffectLevel level = 10; // derived
  McpHints hints = 11; // derived
  Authority authority = 12; // derived
  Binding binding = 13;
  repeated Oracle oracles = 14;
  DryRun dry_run = 15;
  bool returns_target_content = 16; // default true (REA TARGET_CONTENT_FREE_TOOLS inverse)
  Lifetime lifetime = 17;
  repeated Precondition preconditions = 18;
  Provenance provenance = 19;
  AdmissionState admission = 20;
  repeated Withheld withheld = 21;
  repeated string verification_record_ids = 22;
  bool core = 23; // in the curated core set (<= 30 per target)
  Projections projections = 24;
  repeated Example examples = 25; // verification inputs; >= 2 distinct per lane run
}

message Projections {
  string mcp_name = 1; // ^[a-zA-Z0-9_-]{1,64}$, derived, collision-suffixed
  bool mcp_omit = 2;
  string cli_path = 3; // "writer add-paragraph"
  string cli_omit_reason = 4; // typed omission instead of a test-side list (REA)
  string skill_use_when = 5;
}

// ---------- bindings ----------

message Binding {
  oneof kind {
    CliArgvBinding cli = 1;
    McpToolBinding mcp = 2;
    HttpOperationBinding http = 3;
    GrpcMethodBinding grpc = 4;
    AdapterOperationBinding adapter = 5;
  }
}

enum OutputMode {
  OUTPUT_MODE_UNSPECIFIED = 0;
  OUTPUT_MODE_JSON_STDOUT = 1;
  OUTPUT_MODE_TEXT_STDOUT = 2;
  OUTPUT_MODE_NONE = 3;
}

enum ErrorConvention {
  ERROR_CONVENTION_UNSPECIFIED = 0;
  ERROR_CONVENTION_EXIT_CODE = 1; // nonzero exit = failure, stderr excerpt
  ERROR_CONVENTION_CLI_ANYTHING_JSON = 2; // {"error": str, "type": str} + exit 1
}

enum ArgStyle {
  ARG_STYLE_UNSPECIFIED = 0;
  ARG_STYLE_OPTION = 1; // --flag value
  ARG_STYLE_FLAG = 2; // --flag when true
  ARG_STYLE_REPEATED_OPTION = 3; // --flag a --flag b
  ARG_STYLE_POSITIONAL = 4; // emitted after "--"
}

message ArgMapping {
  string pointer = 1; // JSON pointer into input object
  ArgStyle style = 2;
  string flag = 3; // "--text"
  int32 position = 4;
}

// Never a shell. argv is built from typed input; positionals follow "--".
message CliArgvBinding {
  repeated string argv_prefix = 1; // after the executable, e.g. ["--json"]
  repeated string command_path = 2; // ["writer", "add-paragraph"]
  repeated ArgMapping args = 3;
  OutputMode output = 4;
  ErrorConvention errors = 5;
  string state_handle_pointer = 6; // input pointer bound to e.g. "--project"
  string state_handle_flag = 7;
  uint32 deadline_ms = 8;
  uint64 max_output_bytes = 9;
}

message McpToolBinding {
  string server_ref = 1; // Target-level server launch spec id
  string tool_name = 2;
}

message HttpOperationBinding {
  string method = 1;
  string path_template = 2;
  string operation_id = 3;
}

message GrpcMethodBinding {
  string full_method = 1; // "/pkg.Service/Method"
  bool server_streaming = 2;
}

message AdapterOperationBinding {
  string adapter_id = 1;
  string operation = 2;
}

// ---------- oracles (how a write is confirmed) ----------

message Oracle {
  oneof kind {
    ReadBackOracle read_back = 1;
    FileOracle file = 2;
  }
}

enum PredicateOp {
  PREDICATE_OP_UNSPECIFIED = 0;
  PREDICATE_OP_EXISTS = 1;
  PREDICATE_OP_EQUALS = 2;
  PREDICATE_OP_CONTAINS = 3;
  PREDICATE_OP_COUNT_DELTA = 4; // array length after - before == value
  PREDICATE_OP_ABSENT = 5;
}

// Declarative, no regex (REA verifier admission rules). Must fail on the seed
// state and pass after the write, or the oracle itself is not admitted.
message Predicate {
  string pointer = 1; // into the read capability's output
  PredicateOp op = 2;
  string value_json = 3; // may reference input: {"$input": "/text"}
}

message ReadBackOracle {
  string capability_id = 1; // must be a SERVABLE read capability
  string args_template_json = 2; // {"project": {"$input": "/project"}}
  repeated Predicate predicates = 3;
}

// CLI-Anything output verification doctrine: magic bytes + size.
message FileOracle {
  string path_pointer = 1;
  bytes magic_prefix = 2;
  uint64 min_size = 3;
}
```

### 6.2 `aa/ir/v1/record.proto`: errors, Evidence, ActionRecord, VerificationRecord, proposals

```proto
syntax = "proto3";

// Evidence, action records, verification records, errors, LLM proposals.
package aa.ir.v1;

import "aa/ir/v1/ir.proto";
import "google/protobuf/timestamp.proto";

// ---------- errors (REA error envelope, src/contracts/errorSchemas.ts) ----------

enum ErrorCategory {
  ERROR_CATEGORY_UNSPECIFIED = 0;
  ERROR_CATEGORY_INVALID_INPUT = 1;
  ERROR_CATEGORY_UNSUPPORTED_TARGET = 2;
  ERROR_CATEGORY_INTEGRITY_MISMATCH = 3;
  ERROR_CATEGORY_TRUNCATED = 4;
  ERROR_CATEGORY_CANCELLED = 5;
  ERROR_CATEGORY_TIMEOUT = 6;
  ERROR_CATEGORY_RESOURCE_CONSTRAINT = 7;
  ERROR_CATEGORY_UNAVAILABLE = 8;
  ERROR_CATEGORY_EXECUTION_FAILURE = 9;
  ERROR_CATEGORY_AUTHORIZATION = 10; // new: policy / confirmation
}

enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  ERROR_CODE_INVALID_REQUEST = 1; // malformed: schema rejected
  ERROR_CODE_UNRESOLVED_SELECTOR = 2; // valid but the target cannot resolve it
  ERROR_CODE_CAPABILITY_UNAVAILABLE = 3;
  ERROR_CODE_CAPABILITY_WITHHELD = 4;
  ERROR_CODE_TARGET_UNAVAILABLE = 5;
  ERROR_CODE_TARGET_IDENTITY_CHANGED = 6; // exe sha / version drifted since lock
  ERROR_CODE_CATALOG_STALE = 7; // expected_catalog_digest mismatch
  ERROR_CODE_POLICY_DENIED = 8;
  ERROR_CODE_CONFIRMATION_REQUIRED = 9;
  ERROR_CODE_ACTION_UNCONFIRMED = 10; // ran, but post-condition not observed
  ERROR_CODE_EXECUTION_STATE_UNKNOWN = 11; // timeout/poison: may or may not have happened
  ERROR_CODE_UPSTREAM_ERROR = 12; // target reported failure; upstream type preserved
  ERROR_CODE_TIMEOUT = 13;
  ERROR_CODE_CANCELLED = 14;
  ERROR_CODE_RESOURCE_CONSTRAINT = 15;
  ERROR_CODE_CLEANUP_INCOMPLETE = 16;
  ERROR_CODE_ADAPTER_PROTOCOL = 17;
  ERROR_CODE_INTERNAL = 18;
}

enum RecoveryKind {
  RECOVERY_KIND_UNSPECIFIED = 0;
  RECOVERY_KIND_RETRY = 1;
  RECOVERY_KIND_RETRY_CLEANUP = 2;
  RECOVERY_KIND_REOBSERVE_TARGET = 3;
  RECOVERY_KIND_REQUEST_CONFIRMATION = 4;
  RECOVERY_KIND_CHECK_STATUS_BEFORE_RETRY = 5; // receipts for non-idempotent ops
  RECOVERY_KIND_REDISTILL = 6; // target drifted; rerun harvest/verify
}

message Recovery {
  RecoveryKind kind = 1;
  string operation = 2; // RPC or capability to call
  string arguments_json = 3;
}

// Domain errors travel as values inside responses, never as gRPC status.
message ErrorProjection {
  ErrorCode code = 1;
  ErrorCategory category = 2;
  string message = 3;
  bool retryable = 4;
  string remediation_action = 5; // required, non-empty
  Recovery recovery = 6;
  string details_json = 7; // typed per code; e.g. upstream {"type":"IndexError"}
  bool message_untrusted = 8; // contains target-originated text
}

// ---------- evidence ----------

message Evidence {
  string evidence_id = 1; // "ev_" + sha256(JCS(record minus id))
  string predicate_type = 2; // "aa.harvest", "aa.observation", "aa.action", "aa.verification"
  string operation = 3;
  string target_id = 4;
  TargetIdentity target_identity = 5;
  Confidence confidence = 6;
  Isolation isolation = 7;
  string normalized_result_json = 8;
  string raw_blob_sha256 = 9; // large raw payloads live in the CAS
  repeated string limitations = 10;
  repeated string non_claims = 11; // required: what this record does NOT establish
  repeated string evidence_links = 12;
  google.protobuf.Timestamp observed_at = 13;
}

// ---------- actions ----------

enum Verdict {
  VERDICT_UNSPECIFIED = 0;
  VERDICT_ALLOW = 1;
  VERDICT_ASK = 2;
  VERDICT_DENY = 3;
}

enum ApprovalChannel {
  APPROVAL_CHANNEL_UNSPECIFIED = 0;
  APPROVAL_CHANNEL_STANDING_GRANT = 1; // policy file rule
  APPROVAL_CHANNEL_MCP_ELICITATION = 2;
  APPROVAL_CHANNEL_OPERATOR_TTY = 3; // `aa approve` on an interactive terminal
}

message PolicyDecision {
  Verdict verdict = 1;
  string rule_id = 2; // which allow/deny/ask rule matched
  string grant_id = 3;
  string approval_id = 4;
  ApprovalChannel channel = 5;
  repeated string reasons = 6;
}

enum Outcome {
  OUTCOME_UNSPECIFIED = 0;
  OUTCOME_COMPLETED = 1;
  OUTCOME_FAILED = 2; // target reported failure; effects may still exist
  OUTCOME_REFUSED = 3; // engine refused before dispatch
  OUTCOME_NOT_DISPATCHED = 4; // denied / awaiting confirmation; nothing ran
  OUTCOME_UNKNOWN = 5; // deadline/poison; effects unknown
}

enum FileChange {
  FILE_CHANGE_UNSPECIFIED = 0;
  FILE_CHANGE_CREATED = 1;
  FILE_CHANGE_MODIFIED = 2;
  FILE_CHANGE_DELETED = 3;
  FILE_CHANGE_UNCHANGED = 4;
}

message ObservedFileEffect {
  string path = 1; // inside a watched root
  FileChange change = 2;
  string before_sha256 = 3;
  string after_sha256 = 4;
}

message ProcessReport {
  int32 exit_code = 1;
  int32 signal = 2;
  uint64 stdout_bytes = 3;
  uint64 stderr_bytes = 4;
  string stderr_excerpt = 5; // bounded, redacted, untrusted
  bool output_truncated = 6;
  uint32 wall_ms = 7;
  string cleanup = 8; // "complete" | "incomplete" | "unverified"
}

// Append-only, hash-chained in SQLite. One per Invoke, including refusals.
message ActionRecord {
  string action_id = 1; // ULID
  uint64 seq = 2;
  string prev_record_sha256 = 3;
  string record_sha256 = 4; // sha256(JCS(this with record_sha256 = ""))
  string capability_id = 5;
  string catalog_digest = 6;
  string lock_digest = 7;
  TargetIdentity target_identity = 8; // as re-verified just before dispatch
  string input_sha256 = 9;
  string input_redacted_json = 10;
  bool dry_run = 11;
  string idempotency_key = 12;
  Effects declared_effects = 13;
  EffectLevel level = 14;
  PolicyDecision decision = 15;
  Isolation isolation = 16;
  string front_end = 17; // "aa-cli/0.1", "aa-mcp/0.1 client=claude-code"
  repeated string pre_observation_ids = 18;
  repeated string post_observation_ids = 19;
  repeated ObservedFileEffect observed_files = 20;
  repeated string undeclared_effects = 21; // observed but not declared -> flagged
  Outcome outcome = 22;
  ProcessReport process = 23;
  ErrorProjection error = 24;
  repeated string residual_unknowns = 25;
  google.protobuf.Timestamp requested_at = 26;
  google.protobuf.Timestamp dispatched_at = 27;
  google.protobuf.Timestamp completed_at = 28;
}

// ---------- verification ----------

enum Lane {
  LANE_UNSPECIFIED = 0;
  LANE_STATIC = 1; // L0 schema closed, refs resolve, effect audit complete, names unique
  LANE_PARSER = 2; // L1 unknown flag / wrong type -> usage error
  LANE_DRY_RUN = 3; // L2 native dry run behaves
  LANE_SANDBOX_DIFF = 4; // L3 observed effects subset of declared
  LANE_READ_BACK = 5; // L4 oracle fails on seed, passes after
  LANE_GOLDEN = 6; // L5 replay recorded goldens
  LANE_DIFFERENTIAL = 7; // L6 two surfaces agree
  LANE_PROPERTY = 8; // L7 generated inputs
  LANE_AGENT = 9; // L8 agent routing eval (separate lane)
}

enum LaneStatus {
  LANE_STATUS_UNSPECIFIED = 0;
  LANE_STATUS_PASSED = 1;
  LANE_STATUS_FAILED = 2;
  LANE_STATUS_SKIPPED = 3; // never counts as coverage
  LANE_STATUS_ERROR = 4; // harness failure, e.g. backend unreachable
}

message VerificationRecord {
  string record_id = 1; // "vr_" + sha256
  string capability_id = 2;
  Lane lane = 3;
  LaneStatus status = 4;
  string skip_reason = 5;
  string lock_digest = 6;
  TargetIdentity target_identity = 7;
  Isolation isolation = 8;
  string host = 9; // os/arch
  string engine_version = 10;
  repeated string action_ids = 11; // the invocations that produced it
  repeated string evidence_ids = 12;
  repeated string non_claims = 13; // required
  repeated EffectSignal effect_signals = 14; // OBSERVED signals this lane emits
  google.protobuf.Timestamp ran_at = 15;
}

// ---------- the LLM boundary ----------

// An LLM (or a human) never edits the lock. It submits a proposal; the engine
// validates every op deterministically and records accepted ops with provenance.
message ManifestProposal {
  string proposal_id = 1;
  string author = 2; // "claude-opus-5.5 via aa-mcp", "human:<login>"
  string target_id = 3;
  string base_lock_digest = 4; // must equal current lock, else rejected (no blind merges)
  repeated ProposalOp ops = 5;
}

message ProposalOp {
  oneof op {
    RenameOp rename = 1;
    DescribeOp describe = 2;
    FamilyOp family = 3;
    CoreSetOp core = 4;
    RaiseEffectOp raise_effect = 5;
    PairOracleOp pair_oracle = 6;
    ExcludeOp exclude = 7;
    RefineSchemaOp refine_schema = 8;
    AddExampleOp add_example = 9;
  }
  string rationale = 20;
}

message RenameOp {
  string capability_id = 1;
  string new_id = 2;
}

message DescribeOp {
  string capability_id = 1;
  string title = 2;
  string description = 3;
  string use_when = 4;
}

message FamilyOp {
  string capability_id = 1;
  string family = 2;
}

message CoreSetOp {
  repeated string capability_ids = 1;
}

message RaiseEffectOp {
  string capability_id = 1;
  string field = 2;
  Tri value = 3; // TRUE only (FALSE for idempotent); anything else is rejected
}

message PairOracleOp {
  string capability_id = 1;
  Oracle oracle = 2;
}

message ExcludeOp {
  string capability_id = 1;
  string reason = 2;
}

// May only narrow: add enum constraints, descriptions, formats; never widen or open.
message RefineSchemaOp {
  string capability_id = 1;
  string json_patch = 2; // RFC 6902 over input_schema_json
}

// Test inputs are harmless data: they must validate against the canonical schema
// and only ever execute inside a verification lane's sandbox.
message AddExampleOp {
  string capability_id = 1;
  Example example = 2;
}
```

### 6.3 `aa/engine/v1/engine.proto`: engine services

```proto
syntax = "proto3";

// The engine API (aad). Every front end (aa CLI, aa-mcp, skills, CI) is a client.
package aa.engine.v1;

import "aa/ir/v1/ir.proto";
import "aa/ir/v1/record.proto";

// ===== EngineService: liveness and identity =====
service EngineService {
  rpc GetStatus(GetStatusRequest) returns (GetStatusResponse);
  rpc Shutdown(ShutdownRequest) returns (ShutdownResponse);
}

message GetStatusRequest {}

message GetStatusResponse {
  string engine_version = 1;
  string protocol_version = 2; // "aa.engine.v1"
  string workspace_root = 3;
  uint32 target_count = 4;
  uint32 servable_capability_count = 5;
  string listen = 6; // "unix:///.../engine.sock"
  bool kill_switch_engaged = 7;
}

message ShutdownRequest {
  bool drain = 1;
}

message ShutdownResponse {}

// ===== TargetService: create/import targets =====
service TargetService {
  rpc ListTargets(ListTargetsRequest) returns (ListTargetsResponse);
  rpc GetTarget(GetTargetRequest) returns (GetTargetResponse);
  // Create a target from a locator; no execution happens here (STATIC authority).
  rpc CreateTarget(CreateTargetRequest) returns (CreateTargetResponse);
  // Read registry.json + harness dirs from a local CLI-Anything checkout.
  rpc ImportCliAnything(ImportCliAnythingRequest) returns (ImportCliAnythingResponse);
}

message ListTargetsRequest {}

message ListTargetsResponse {
  repeated aa.ir.v1.Target targets = 1;
}

message GetTargetRequest {
  string target_id = 1;
}

message GetTargetResponse {
  aa.ir.v1.Target target = 1;
}

message TargetLocator {
  oneof locator {
    string executable = 1; // name on PATH or absolute path
    string bundle_path = 2; // .app
    string service_origin = 3; // http://127.0.0.1:8080 (loopback/owned only)
    string mcp_command_json = 4; // ["npx","-y","pkg@1.2.3"] stdio MCP server
    string openapi_path = 5;
  }
}

message CreateTargetRequest {
  string target_id = 1;
  TargetLocator locator = 2;
  string authorization_statement = 3; // required, recorded verbatim
}

message CreateTargetResponse {
  aa.ir.v1.Target target = 1;
  aa.ir.v1.ErrorProjection error = 2;
}

enum InstallMode {
  INSTALL_MODE_UNSPECIFIED = 0;
  INSTALL_MODE_PREINSTALLED = 1; // entry point must already resolve on PATH
  INSTALL_MODE_PRIVATE_VENV = 2; // engine-owned venv from the LOCAL harness dir; never registry install_cmd
}

message ImportCliAnythingRequest {
  string repo_path = 1; // e.g. /Users/me/CLI-Anything
  repeated string harnesses = 2; // registry names; empty = list only
  InstallMode install_mode = 3;
  string authorization_statement = 4;
}

message ImportedHarness {
  string name = 1;
  string target_id = 2; // "cli-anything.<name>"
  string harness_tree_sha = 3;
  aa.ir.v1.ErrorProjection error = 4;
}

message ImportCliAnythingResponse {
  repeated ImportedHarness harnesses = 1;
  repeated string available = 2; // registry names found when harnesses is empty
}

// ===== DistillService: discover -> characterize -> synthesize -> verify -> package =====
service DistillService {
  rpc Harvest(HarvestRequest) returns (stream HarvestResponse);
  rpc Characterize(CharacterizeRequest) returns (CharacterizeResponse);
  rpc ApplyProposal(ApplyProposalRequest) returns (ApplyProposalResponse);
  rpc Compile(CompileRequest) returns (CompileResponse);
  rpc Verify(VerifyRequest) returns (stream VerifyResponse);
  rpc Package(PackageRequest) returns (PackageResponse);
}

message HarvestRequest {
  string target_id = 1;
  repeated string harvesters = 2; // empty = all applicable, by probe order
  aa.ir.v1.DiscoveryAuthority max_authority = 3; // default INTROSPECT
  bool allow_network = 4; // default false; scrubbed env either way
}

message HarvestResponse {
  oneof event {
    string progress = 1;
    aa.ir.v1.SourceRef source = 2; // a raw artifact was stored
    uint32 candidates = 3; // running count
    aa.ir.v1.ErrorProjection error = 4; // per-harvester failure, harvest continues
    HarvestSummary summary = 5; // final
  }
}

message HarvestSummary {
  repeated aa.ir.v1.SourceRef sources = 1;
  uint32 candidate_count = 2;
  repeated string skipped_harvesters = 3; // with reason in progress stream
}

message CharacterizeRequest {
  string target_id = 1;
}

message CharacterizeResponse {
  repeated aa.ir.v1.Capability capabilities = 1; // admission CANDIDATE/WITHHELD only
  uint32 unknown_effect_count = 2;
}

message ApplyProposalRequest {
  aa.ir.v1.ManifestProposal proposal = 1;
  bool dry_run = 2; // validate only
}

message RejectedOp {
  uint32 index = 1;
  string reason = 2; // e.g. "raise_effect may not lower", "unknown capability_id"
}

message ApplyProposalResponse {
  repeated uint32 accepted = 1;
  repeated RejectedOp rejected = 2;
  string overlay_digest = 3;
}

message CompileRequest {
  string target_id = 1;
  bool check = 2; // fail if the on-disk lock differs (CI drift mode)
}

message CompileResponse {
  string lock_digest = 1;
  string catalog_digest = 2;
  repeated string added = 3;
  repeated string removed = 4;
  repeated string changed = 5; // admission resets to WITHHELD/CANDIDATE on change
  aa.ir.v1.ErrorProjection error = 6;
}

message VerifyRequest {
  string target_id = 1;
  repeated string capability_ids = 2; // empty = all
  repeated aa.ir.v1.Lane lanes = 3; // empty = ladder up to what each level requires
  repeated string require_tools = 4; // missing => FAILED, not SKIPPED
  string fixture = 5; // seed state recipe id from the overlay
}

message VerifyResponse {
  oneof event {
    string progress = 1;
    aa.ir.v1.VerificationRecord record = 2;
    VerifySummary summary = 3;
  }
}

message VerifySummary {
  uint32 servable = 1;
  uint32 withheld = 2;
  uint32 failed = 3;
  uint32 skipped = 4;
  aa.ir.v1.TargetStatus target_status = 5;
}

enum Renderer {
  RENDERER_UNSPECIFIED = 0;
  RENDERER_MCP_TYPED = 1;
  RENDERER_MCP_SEARCH = 2; // search + describe + invoke
  RENDERER_CLI_TREE = 3;
  RENDERER_SKILL_MD = 4;
  RENDERER_USAGE_KDL = 5;
  RENDERER_SERVER_JSON = 6;
}

message PackageRequest {
  string target_id = 1;
  repeated Renderer renderers = 2;
  bool check = 3; // drift mode: compare to files on disk, write nothing
}

message RenderedFile {
  string path = 1; // relative to the target's packages/ dir
  string sha256 = 2;
  bool drifted = 3;
}

message PackageResponse {
  repeated RenderedFile files = 1;
  string catalog_digest = 2;
}

// ===== CatalogService: what front ends advertise =====
service CatalogService {
  rpc ListCapabilities(ListCapabilitiesRequest) returns (ListCapabilitiesResponse);
  rpc SearchCapabilities(SearchCapabilitiesRequest) returns (SearchCapabilitiesResponse);
  rpc DescribeCapability(DescribeCapabilityRequest) returns (DescribeCapabilityResponse);
  // The exact projection a front end serves; its digest is the catalog identity.
  rpc GetExposure(GetExposureRequest) returns (GetExposureResponse);
}

message ListCapabilitiesRequest {
  string target_id = 1; // empty = all
  string family = 2;
  bool servable_only = 3;
  aa.ir.v1.EffectLevel max_level = 4;
  uint32 offset = 5;
  uint32 limit = 6;
}

message ListCapabilitiesResponse {
  repeated aa.ir.v1.Capability capabilities = 1;
  uint32 next_offset = 2; // 0 = end
  uint32 total = 3;
}

message SearchCapabilitiesRequest {
  string query = 1;
  string target_id = 2;
  uint32 limit = 3;
}

message SearchCapabilitiesResponse {
  repeated CapabilitySummary results = 1;
}

message CapabilitySummary {
  string id = 1;
  string title = 2;
  aa.ir.v1.EffectLevel level = 3;
  aa.ir.v1.AdmissionState admission = 4;
  string mcp_name = 5;
}

message DescribeCapabilityRequest {
  string capability_id = 1;
}

message DescribeCapabilityResponse {
  aa.ir.v1.Capability capability = 1;
  repeated aa.ir.v1.VerificationRecord records = 2;
}

enum ExposureProfile {
  EXPOSURE_PROFILE_UNSPECIFIED = 0;
  EXPOSURE_PROFILE_MCP_TYPED = 1;
  EXPOSURE_PROFILE_MCP_SEARCH = 2;
  EXPOSURE_PROFILE_CLI = 3;
}

message GetExposureRequest {
  ExposureProfile profile = 1;
  repeated string target_ids = 2;
  repeated string families = 3;
  bool include_withheld = 4; // listed as disabled with reasons, never callable
}

message ExposedTool {
  string name = 1; // MCP tool name or CLI path
  string capability_id = 2;
  string title = 3;
  string description = 4;
  string input_schema_json = 5; // compact profile; server re-validates canonical
  string output_schema_json = 6;
  aa.ir.v1.McpHints hints = 7;
  string meta_json = 8; // _meta: effects vector, level, mutates_session, untrusted
  bool enabled = 9;
}

message GetExposureResponse {
  repeated ExposedTool tools = 1;
  string catalog_digest = 2;
}

// ===== RuntimeService: invoke capabilities =====
service RuntimeService {
  rpc Invoke(InvokeRequest) returns (stream InvokeResponse);
  rpc Cancel(CancelRequest) returns (CancelResponse);
  rpc GetActionStatus(GetActionStatusRequest) returns (GetActionStatusResponse);
}

message InvokeRequest {
  string capability_id = 1;
  string input_json = 2; // canonical input; CLI flags are sugar over this
  bool dry_run = 3;
  string approval_token = 4; // from PolicyService.ResolveApproval
  string idempotency_key = 5; // required for non-idempotent levels >= WRITE
  string expected_catalog_digest = 6; // optional stale-client guard
  uint32 deadline_ms = 7;
  string front_end = 8;
}

message InvokeResult {
  string output_json = 1; // validated against output_schema when known
  bool output_untrusted = 2;
  repeated string evidence_ids = 3;
}

message ConfirmationChallenge {
  string approval_id = 1;
  string summary = 2; // engine-authored: what, where, effects, dry-run preview
  aa.ir.v1.EffectLevel level = 3;
  string dry_run_preview_json = 4;
  uint32 expires_in_s = 5;
}

message InvokeResponse {
  oneof event {
    string progress = 1;
    InvokeResult result = 2; // terminal
    aa.ir.v1.ErrorProjection error = 3; // terminal
    ConfirmationChallenge confirmation = 4; // terminal; nothing dispatched
  }
  string action_id = 10; // set on every event
}

message CancelRequest {
  string action_id = 1;
}

message CancelResponse {
  aa.ir.v1.Outcome outcome = 1;
}

message GetActionStatusRequest {
  oneof key {
    string action_id = 1;
    string idempotency_key = 2;
  }
}

message GetActionStatusResponse {
  aa.ir.v1.ActionRecord record = 1;
}

// ===== PolicyService: grants, approvals, kill switch =====
service PolicyService {
  rpc Evaluate(EvaluateRequest) returns (EvaluateResponse);
  rpc ListApprovals(ListApprovalsRequest) returns (ListApprovalsResponse);
  // Only accepted over an operator channel (see "Approval channels").
  rpc ResolveApproval(ResolveApprovalRequest) returns (ResolveApprovalResponse);
  rpc SetKillSwitch(SetKillSwitchRequest) returns (SetKillSwitchResponse);
}

message EvaluateRequest {
  string capability_id = 1;
  string input_json = 2;
}

message EvaluateResponse {
  aa.ir.v1.PolicyDecision decision = 1;
}

message ListApprovalsRequest {}

message ListApprovalsResponse {
  repeated ConfirmationChallenge pending = 1;
}

message ResolveApprovalRequest {
  string approval_id = 1;
  bool approve = 2;
  aa.ir.v1.ApprovalChannel channel = 3;
  string scope = 4; // "once" | "session" (never "forever" from this RPC)
}

message ResolveApprovalResponse {
  string approval_token = 1; // single-use, bound to (capability, input_sha256, expiry)
}

message SetKillSwitchRequest {
  bool engaged = 1;
}

message SetKillSwitchResponse {
  uint32 cancelled_actions = 1;
}

// ===== AuditService: the action log =====
service AuditService {
  rpc QueryActions(QueryActionsRequest) returns (QueryActionsResponse);
  rpc VerifyChain(VerifyChainRequest) returns (VerifyChainResponse);
}

message QueryActionsRequest {
  string target_id = 1;
  string capability_id = 2;
  uint64 after_seq = 3;
  uint32 limit = 4;
}

message QueryActionsResponse {
  repeated aa.ir.v1.ActionRecord records = 1;
  uint64 next_seq = 2;
}

message VerifyChainRequest {}

message VerifyChainResponse {
  bool intact = 1;
  uint64 records = 2;
  uint64 first_broken_seq = 3;
}
```

### 6.4 `aa/adapter/v1/adapter.proto`: the out-of-process adapter protocol

```proto
syntax = "proto3";

// Out-of-process adapter protocol. Implemented by adapters in any language.
// Two framings of the same messages:
//   1. gRPC over a 0600 UDS the adapter binds (long-lived, sessions, bridges).
//   2. one-shot: `<exe> --aa-oneshot <Method>`; proto3-JSON request on stdin,
//      JSONL proto3-JSON responses on stdout, reserved exit codes (see doc).
package aa.adapter.v1;

import "aa/ir/v1/ir.proto";
import "aa/ir/v1/record.proto";

service AdapterService {
  // Must succeed before any other call; every field is compared by the engine.
  rpc Handshake(HandshakeRequest) returns (HandshakeResponse);
  rpc Harvest(HarvestRequest) returns (stream HarvestResponse);
  rpc Invoke(InvokeRequest) returns (stream InvokeResponse);
  // Receipts: lets the engine check a request before retrying a non-idempotent op.
  rpc GetRequestStatus(GetRequestStatusRequest) returns (GetRequestStatusResponse);
  // State read used by oracles and pre/post observations.
  rpc Observe(ObserveRequest) returns (ObserveResponse);
  rpc Shutdown(ShutdownRequest) returns (ShutdownResponse);
}

message AdapterIdentity {
  string id = 1; // "aa.click-harvest"
  string name = 2;
  string version = 3;
  string language = 4; // "python3.14"
  string executable_sha256 = 5; // engine verifies independently
}

message HandshakeRequest {
  string run_id = 1; // from the 0600 descriptor; adapter must echo it
  string protocol_version = 2; // "aa.adapter.v1"
  uint64 max_frame_bytes = 3;
  string engine_version = 4;
}

message HandshakeResponse {
  string run_id = 1;
  string protocol_version = 2;
  uint64 max_frame_bytes = 3; // must equal request, else startup fails
  AdapterIdentity identity = 4;
  repeated string operations = 5; // exact set; engine compares to manifest
  repeated string harvesters = 6;
  aa.ir.v1.TargetIdentity target = 7; // for bridges living inside a host app
  uint64 generation = 8; // changes when active document/window changes
}

message HarvestRequest {
  string harvester = 1;
  aa.ir.v1.TargetIdentity target = 2;
  aa.ir.v1.DiscoveryAuthority max_authority = 3;
  map<string, string> options = 4;
}

// A candidate is a partially filled Capability plus per-field provenance;
// the engine, not the adapter, decides effects and admission.
message CandidateCapability {
  aa.ir.v1.Capability capability = 1;
  repeated aa.ir.v1.EffectSignal signals = 2; // raw native signals only
}

message RawArtifact {
  string name = 1; // "click-info.json"
  string media_type = 2;
  bytes content = 3; // verbatim; engine stores in CAS and digests
}

message HarvestResponse {
  oneof event {
    RawArtifact raw = 1;
    CandidateCapability candidate = 2;
    aa.ir.v1.ErrorProjection error = 3;
    string next_probe = 4; // suggested follow-up (REA next_probes), inferred
  }
}

message InvokeRequest {
  string request_id = 1; // engine-chosen, used for receipts
  string operation = 2;
  string input_json = 3; // already validated against the canonical schema
  uint32 deadline_ms = 4;
  uint64 expected_generation = 5; // refuse if the host moved on
  bool dry_run = 6;
}

message InvokeResponse {
  oneof event {
    string progress = 1;
    string output_json = 2; // terminal
    aa.ir.v1.ErrorProjection error = 3; // terminal; upstream reasons preserved
  }
}

enum RequestState {
  REQUEST_STATE_UNSPECIFIED = 0;
  REQUEST_STATE_UNKNOWN_ID = 1;
  REQUEST_STATE_RUNNING = 2;
  REQUEST_STATE_COMPLETED = 3;
  REQUEST_STATE_FAILED = 4;
  REQUEST_STATE_POISONED = 5; // deadline passed while host could not cancel
}

message GetRequestStatusRequest {
  string request_id = 1;
}

message GetRequestStatusResponse {
  RequestState state = 1;
}

message ObserveRequest {
  string view = 1; // adapter-declared view name
  string args_json = 2;
}

message ObserveResponse {
  string state_json = 1;
  bool complete = 2; // false => predicates decided by omission must fail
  uint64 generation = 3;
}

message ShutdownRequest {}

message ShutdownResponse {}
```

---

## 7. Transport and local security

| Mode | When | Listener | Authentication | Notes |
|---|---|---|---|---|
| **UDS (default)** | macOS, Linux | Linux: `$XDG_RUNTIME_DIR/agentic-anything/engine.sock`; macOS: `$TMPDIR/agentic-anything/engine.sock` (per-user `/var/folders/...` dir, short enough for the 104-byte `sun_path` limit) | Dir 0700, checked with `lstat` (refused if it is a symlink, has another owner, or has a looser mode). The private directory is the real guard, so there is no cross-user window. The socket is chmodded to 0600 right after bind, avoiding a process-wide umask change in a multithreaded runtime. Per-request interceptor: `UdsConnectInfo.peer_cred.uid == geteuid()`, else `PERMISSION_DENIED` (tonic 0.14.9 exposes `peer_cred`, checked in source) | **No token on UDS.** A token file readable by the same uid adds nothing; REA says its token "protects the IPC channel, not the user account" (`[rea-engine-bridges-and-process-supervision §1]`) |
| **Loopback TCP (opt-in)** | Windows, WSL↔host, containers | `aa daemon --listen tcp://127.0.0.1:0` only. The engine classifies the *bound* address and refuses non-loopback, as REA's HTTP listener does (`[rea-mcp-server-adapter §Streamable HTTP listener]`) | 32-byte random bearer token in `engine.token` (0600), compared in constant time (`subtle`). The endpoint is published by atomic rename of `endpoint.json` (REA bridge pattern). `:authority` must be a loopback literal (DNS-rebinding defence) | No TLS in v1 (loopback only) |
| **In-process** | tests, CI, `aa --embedded` | tonic over `tokio::io::duplex` | none (same process) | no daemon flakiness in CI |

Further rules:
- **Single instance.** The engine takes `fd-lock` on `engine.lock`. It removes a stale socket only while holding the lock.
- **Never root.** `aad` refuses to start with `euid == 0`. REA never invokes sudo (`[survey-safety-rails-and-authorization-models §4]`).
- **Adapter channel.** Each adapter run gets a private dir (0700) under `runtime/adapters/<run-id>/` and a 0600 descriptor `{run_id, socket_path, token, max_frame_bytes, protocol}`.
  - Only the *path* of the descriptor goes in env, as `AA_ADAPTER_DESCRIPTOR`. The adapter reads the descriptor and deletes it.
  - For gRPC framing, the adapter binds its own 0600 UDS and the engine connects to it with the token.
  - `Handshake` must echo `run_id` and `max_frame_bytes`, and report the exact operation set. Any mismatch fails startup.
  - This is REA's socket-bridge bootstrap, unchanged in substance (`[rea-engine-bridges-and-process-supervision §1, §8.2-8.4]`).
- **MCP front end.** stdio only in v1, inheriting the MCP client's trust. Streamable HTTP is deferred. When it lands, it gets REA's Host/Origin allowlist and loopback classification.
- **Honest boundary.**
  - Any process running as the same uid can talk to the engine. So can an agent that has a shell, and it can call `aa approve`.
  - In single-uid mode, grants and approvals are therefore **guardrails against accidents, not a boundary against a determined agent**. Every `PolicyDecision` with `channel = OPERATOR_TTY` is labelled accordingly in its record.
  - The real boundary is the **two-uid mode**, spiked in M2. `aad` runs as `_aa`, and config maps peer uids to roles: `agent_uids` may call Invoke, Catalog and Audit; `operator_uids` may call ResolveApproval, SetKillSwitch and edit grants. The peer-uid interceptor then becomes an authorization check. This is the "separate OS principal" pattern every vendor ships (`[survey-safety-rails-and-authorization-models §9 Process model]`).

---

## 8. Adapter and binding model

### 8.1 Decision rule
1. **If the target's surface is spec-shaped**, use an **in-process generic binding**. A new target is then a manifest only, with zero code. The four bindings are `cli-argv`, `mcp-tool`, `http` (OpenAPI), and `grpc` (reflection + `prost-reflect` dynamic messages).
2. **If talking to it needs a foreign runtime**, use an **out-of-process adapter**. Examples: Python import for Click/argparse, Swift for AX/sdef/App Intents, C# for UIA/COM, and an in-host plugin bridge for Blender, GIMP or LibreOffice UNO.
3. **Never load foreign code into the engine.** No dylib plugins. WASM components are deferred, but they are a possible later sandbox for pure-data harvesters.

### 8.2 `cli-argv` executor (the slice's only binding)
- **Never a shell.** `argv = [exe] + argv_prefix + [state_handle_flag, value]? + command_path + mapped args`. Positionals are always emitted after `--`, so a value starting with `-` cannot become an option. This answers CLI-Anything's own threat model of crafted arguments (`[survey-agent-interface-generators §2.7]`).
- **Path params:**
  - `format: path` parameters are made absolute against the caller's cwd in the front end (REA `rea:src/cliJsonInput.ts:273-313`);
  - the engine canonicalizes the parent and checks the result against the grant scopes before dispatch.
- **Supervision** goes through `aa-exec`:
  - an owned process group;
  - `stdin=/dev/null`;
  - a scrubbed env allowlist (`PATH`, `LANG`, `LC_*`, plus binding-declared vars);
  - a deadline (`deadline_ms`), then TERM 250 ms, then KILL;
  - stdout capped at `max_output_bytes` and stderr kept as a bounded excerpt.

  A timeout yields `OUTCOME_UNKNOWN` with `ERROR_CODE_EXECUTION_STATE_UNKNOWN`, because a killed write may or may not have happened. This is REA's poison rule (`[rea-engine-bridges-and-process-supervision §1 "Timeouts poison the stream"]`).
- **Error conventions:**
  - `EXIT_CODE`: nonzero exit means failure.
  - `CLI_ANYTHING_JSON`: exit 1 with `{"error","type"}` on stdout becomes `ERROR_CODE_UPSTREAM_ERROR`, with the upstream `type` kept in `details_json` and `message_untrusted=true`. Observed: `writer remove 99` → `{"error": "Index 99 out of range (0-0)", "type": "IndexError"}`, exit 1.
  - Click usage errors (exit 2, `Error: No such option`) are a **schema/parser mismatch**. The engine returns `INVALID_REQUEST` and flags the capability for re-verification.
- **Identity re-check before every dispatch.** The engine stats the executable and interpreter (inode, size, mtime) and rehashes on change. A mismatch with the lock yields `TARGET_IDENTITY_CHANGED` with `recovery: REDISTILL` (`[rea-desktop-applications-and-agent-environments §What to distill.2]`).

### 8.3 Out-of-process adapters
- **Two framings of one schema** (`aa.adapter.v1`):
  - **One-shot:** `<exe> --aa-oneshot <Method>`. A proto3-JSON request goes on stdin (capped at 8 MiB). Responses come back as JSONL on stdout, one `…Response` message per line, each line ≤ `max_frame_bytes`, parsed strictly. Exit codes: `0` ok, `64` bad request, `65` refused/out of authority, `69` dependency unavailable, `70` internal, `75` temporary. Used for harvesters and stateless helpers. It is REA's shapes 5 and 6 (one-shot file/JSON helpers) made uniform.
  - **gRPC over UDS:** for long-lived adapters, sessions and in-host bridges. Covers REA's "socket+bearer" shape. Mutual-HMAC attach to user-started hosts (REA's Cutter) is deferred.
- **Mapping REA's six shapes** (`[rea-engine-bridges-and-process-supervision §1]`):

| REA shape | Here |
|---|---|
| pipe-REPL | future `repl` binding |
| pipe-MCP | `mcp-tool` binding with an owned child |
| socket+bearer | Adapter gRPC |
| socket+HMAC | deferred |
| external MCP | `mcp-tool` binding with `attach` and no kill |
| one-shot helper | one-shot framing |

- **The adapter proposes; the engine decides.** Adapters return `CandidateCapability` and raw native `EffectSignal`s. The engine runs characterization, so a buggy or hostile adapter cannot self-certify.
- **Receipts and generations.** `GetRequestStatus(request_id)` lets the engine check before retrying a non-idempotent operation. `expected_generation` refuses actions after the host's document or window has changed (Cutter's pattern, `[rea-engine-bridges-and-process-supervision §What to distill.4-5]`).
- **Bridge kits.** `adapters/python/aa_adapter_kit` provides a descriptor reader, both framings, a closed method table, health and shutdown, and frame limits. Swift follows in M3. A plugin author writes only the method table (`[rea-engine-bridges-and-process-supervision §What to distill.2]`).
- **Conformance.** `aa adapter conformance -- <cmd>` checks:
  - the handshake echoes;
  - a limit mismatch is refused;
  - unknown methods return a typed error;
  - oversize frames are rejected before allocation;
  - a deadline makes the adapter report `POISONED`;
  - shutdown leaves no orphan in the process group.

---

## 9. Pipeline → crates → RPCs

| Stage | Crate | RPC | Artifact | Deterministic | LLM role |
|---|---|---|---|---|---|
| Discover | `aa-harvest` (+ one-shot adapters) | `DistillService.Harvest` | CAS raw + `harvest/index.json` + candidates | yes | none |
| Characterize | `aa-characterize` | `Characterize`, then `Compile` | `capabilities.lock.json` | yes | none |
| Synthesize | `aa-proposal` | `ApplyProposal` | overlay ops + `proposals/*.json` | validator yes | proposes names, descriptions, families, core set, raises, oracle pairings, examples, schema narrowing, exclusions |
| Verify | `aa-verify` | `Verify` | `verification/**` | yes | drafts examples and fixtures (as proposals) |
| Package | `aa-render` | `Package`, `CatalogService.GetExposure` | `packages/**`, catalog digest | yes | none |

### 9.1 Harvester catalogue (ranked by fidelity, `[survey-capability-discovery-and-distillation §3, §What to distill.3]`)

| Harvester | Probe | Authority | Runs as | Native effect signal | Milestone |
|---|---|---|---|---|---|
| `cli-anything-registry` | `registry.json` + harness dir + `SKILL.md` | STATIC | in-proc | none | **slice** |
| `click-info` | import entry point → root `click.Group` → `to_info_dict()` | INTROSPECT (imports harness code) | one-shot Python | none (types only) | **slice** |
| `usage-kdl` | usage spec | STATIC | in-proc (`kdl`) | `effect: read\|write\|destructive` (DECLARED) | M1 |
| `swift-dump-help` | `X --experimental-dump-help` | INTROSPECT | in-proc | none | M1 |
| `cobra-complete` | recursive `X __complete <path> ""` (+ flags with `--`) | INTROSPECT (may hit network, e.g. kubectl) | in-proc | none (enums yes) | M1 |
| `json-fields` | `X <cmd> --json` with no fields → output schema | INTROSPECT | in-proc | — | M1 |
| `help-text` | `--help` recursion, verbatim raw | INTROSPECT | in-proc | keywords (raise only) | M1 (LLM proposals fill types; L1 parser lane checks) |
| `mcp-tools-list` | rmcp client `tools/list` | INTROSPECT (launches server) | in-proc | MCP annotations (untrusted hints; raise only unless the server is marked trusted) | M1 |
| `openapi` | file, or owned loopback URL (no redirects, size cap, AWS-style fetch hardening) | STATIC/INTROSPECT | in-proc | HTTP method (DECLARED only if the spec is operator-trusted) | M1 |
| `sdef` | `Info.plist` → `.sdef`, `xi:include` resolved | STATIC | in-proc (`roxmltree`) | Standard Suite verbs, `requires-access` | M3 |
| `app-intents` | `Metadata.appintents/extract.actionsdata` | STATIC | in-proc | `systemProtocols` (`DeleteEntity` → destructive) | M3 |
| `ax-menubar` | AX menu bar enumeration (no clicks) | INTROSPECT | Swift adapter | none | M3 |
| `graphql` | `__schema` | INTROSPECT | in-proc | query vs mutation root (DECLARED) | M5 |
| `grpc-reflection` | `ServerReflectionInfo` | INTROSPECT | in-proc | none | M5 (dogfooded on `aad`) |
| `dbus` | `Introspect` (activates services) | INTROSPECT | in-proc (`zbus`, Linux) | property `access` | M5 |

Common harvester rules:
- A harvester is a pure function of `(target identity, authority, harvester version)`, and its output is cached Evidence.
- Introspection runs with a scrubbed env: no `KUBECONFIG`, tokens or proxies. It runs with a private HOME/TMPDIR, `stdin=/dev/null`, a deadline, and no network unless `allow_network`.
- `PROBE` and `EXPLORE` run only inside verification lanes.

### 9.2 Characterization tables as code
- One Rust module per source family lives under `aa-characterize/src/rules/`. Each has a version (`rule:cli-verbs@1`) that is stamped into every signal it emits, so a rule change is a visible lock diff.
- The v1 CLI verb table (leaf command token) is **raise-only**:

| Leaf token | Signals raised |
|---|---|
| `add\|new\|create\|set\|modify\|rename\|apply\|save\|import\|open\|insert\|update` | `mutates_target` |
| `remove\|delete\|clear\|reset\|purge\|drop\|prune\|wipe\|overwrite` | `may_discard_data` |
| `send\|publish\|post\|pay\|upload\|push` | `external_side_effect` + `accesses_network` |

- Read-ish tokens (`list|info|show|get|status|json|presets|profiles|formats|history`) emit a `CONFIDENCE_INFERRED` **proposal for review**, not a FALSE signal. Only the L3 lane or an operator can lower an effect.
- Option names raise too:
  - `-o/--output/--out/output_path` → `writes_filesystem` with scope `WRITE_PATH`;
  - `--force/--yes/-f/--hard` → `may_discard_data`;
  - `--overwrite` → `may_discard_data`.
- This keeps REA's split explicit: heuristics are allowed for *synthesized* tools, are forbidden for hand-audited tools, and always carry `inferred` (`[survey-capability-discovery-and-distillation §3.7]`).
- **Oracle pairing heuristics** (deterministic proposals, still verified by L4):
  - a write in group `G` pairs with `G list`, then `G info`, then the root `json`/`info` read;
  - a capability with a file output gets a `FileOracle` with magic bytes by extension (`%PDF-`, `PK\x03\x04`, PNG and so on), from CLI-Anything's output-verification doctrine (`[survey-agent-interface-generators §2.5]`).

### 9.3 The LLM boundary (`aa-proposal`)

The agent loop:
1. `aa propose context <target>` emits a bounded context pack: candidates, withheld reasons with hints, and raw help excerpts, with all target text wrapped as untrusted data (spotlighting, `[survey-safety-rails-and-authorization-models §3.4]`).
2. The LLM writes a `ManifestProposal`.
3. `aa propose apply p.json`, or the `aa_dev_propose` tool in the dev MCP exposure, calls `ApplyProposal`.
4. The validator returns accepted and rejected ops with reasons.
5. Accepted ops are written into the overlay with `# proposal:<id> author:<…>` comments for git review.

Validator rules. Each is a unit-tested predicate, and the module is under a blocking mutation-score floor (§14).
1. `base_lock_digest` must equal the current lock. There are no blind merges.
2. Every `capability_id` must exist in the current harvested candidate set. The LLM cannot invent addresses: "every core tool must map to a discovered address" (`[survey-capability-discovery-and-distillation §5]`).
3. `RaiseEffectOp.value` must be `TRI_TRUE`, or `TRI_FALSE` for `idempotent` only. A proposal can never lower an effect.
4. No op can touch admission, grants, preconditions, identity, `assume`, or `TRUE → FALSE` overrides. Those are operator-only overlay keys, and their presence in a proposal is a hard reject of the whole proposal.
5. `RefineSchemaOp` must **narrow**, checked structurally over the supported JSON Schema subset:
   - the schema stays closed;
   - `required` may only grow;
   - `enum` may only shrink or be added;
   - numeric bounds may only tighten;
   - `type` may not widen;
   - unknown keywords are rejected.

   Any accepted narrowing resets the capability's verification. Example: `export.render --preset` is TEXT in Click, but `export presets` lists `odt, ods, odp, html, text, …` (observed). The proposal adds that enum.
6. A `PairOracleOp` must reference an existing read capability. Its predicates must be declarative (no regex). It is untrusted until L4 passes.
7. `AddExampleOp` inputs must validate against the canonical schema. They only ever run inside lanes.
8. A `DescribeOp` has budgets (title ≤ 64, description ≤ 1024, `use_when` ≤ 200) and no control characters. It is stored with `CONFIDENCE_PROPOSED` and marked LLM-authored in MCP `_meta`.
9. Size caps: 1 MiB per proposal and 2,000 ops.

Why the LLM client is not in the engine:
- It keeps the engine deterministic and testable.
- It makes the proposer replaceable: any agent, any vendor, or a human.
- The trust decision then lives in one auditable place.

### 9.4 Renderers (`aa-render`), all from the lock
- **`mcp-typed`:**
  - Tools come from the servable core set (≤ 30 per target) plus families.
  - Annotations come from `hints`. `_meta.aa` carries `capability_id`, the effects vector, `level`, `mutates_session` and `untrusted`.
  - Canonical schemas are served with a compact advertised profile, wider but never weaker. The handler re-validates against the canonical schema (`[rea-tool-design-philosophy-and-verification §1.3]`).
- **`mcp-search`:** `aa_search`, `aa_describe` and `aa_invoke`, all policy-checked per underlying capability. It is the default when more than 30 capabilities are servable (`[survey-agent-interface-generators §6]`).
- **`cli-tree`:** `cli_path` plus flag specs derived from the schema. `cli_omit_reason` is a typed field, replacing REA's test-side list of 68 MCP-only tools (`[rea-cli-surface-and-agent-packaging §What to distill.1,3]`).
- **`skill-md`:**
  - Follows the agentskills.io constraints: `name` = `aa-<slug>` matching its directory, description ≤ 1024, body ≤ 500 lines, families in `references/`.
  - `compatibility` comes from preconditions; `metadata` carries `aa.lock_digest`, `aa.catalog_digest` and `aa.version`.
  - The body holds a core table with levels, `aa run …` fallbacks, a withheld section with reasons, and the REA rule "trust the live catalog over this skill" (`[survey-agent-interface-generators §5.1]`, `[rea-cli-surface-and-agent-packaging §What to distill.11]`).
- **`usage-kdl`** (M5) and **`server-json`** (M5).
- **Catalog digest:** SHA-256 over JCS of the sorted exposure projection `{name, title, description, input/output schema, hints, meta, enabled}`, plus profile and CLI paths. This is REA's `createToolCatalog` set (`rea:src/catalogIdentity.ts:43-56`). It is exposed by `aa_status` and checked through the optional `expected_catalog_digest`.

### 9.5 Verification ladder as code (`aa-verify`)

| Lane | What it does | Applies to | Emits | Explicit non-claims |
|---|---|---|---|---|
| L0 static | closed schema, refs resolve, every target-facing field decided or withheld with a hint, names unique, binding resolves against harvested address, oracle references servable read | all | pass/fail | runtime behaviour |
| L1 parser | for each param, invoke with a wrong type, and once with an unknown flag `--aa-probe-unknown`; expect usage exit (Click: 2) and no file change | CLI | `PARSER_MISMATCH` on unexpected acceptance | semantic correctness |
| L2 dry-run | run with `dry_run.argv`; for `NO_PERSIST`, assert the state-handle hash is unchanged (observed: `--dry-run` before the subcommand left `p.json` unchanged) | when supported | pass/fail | that the preview matches the real run |
| L3 sandbox-diff | build fixture in a private root (cwd/HOME/TMPDIR redirected, env scrubbed); run ≥ 2 distinct examples; blake3 tree-diff of the root; process-group survivors check | all | OBSERVED signals: FALSE for unchanged *observable* fields, TRUE for observed changes; `SIDE_EFFECT` if observed ⊄ declared | writes outside the root; network (at ISOLATION_PROCESS); effects on cloud state |
| L4 read-back | run read oracle on seed (predicates must **fail**), run the write, run oracle again (must **pass**); FileOracle magic + size | WRITE/DESTRUCTIVE | REA reasons: `no-observation`, `unconfirmed`, `contradicted`, `side-effect`, `unexplained-change` | that the real backend rendered it (unless a precondition binds the backend identity) |
| L5 golden | record (input → normalized output + diff) on a reference version; replay after upgrade; categorized JSON-pointer diffs, capped at 20 per step | M4 | drift | |
| L6 differential | the same capability over two surfaces agrees | M4 | | |
| L7 property | schema-generated inputs (owned instances only) | M5 | | |
| L8 agent | routing eval (first-tool, repeats, validation failures), kept separate from correctness | M6 | | "a scripted sequence tests that sequence" |

Rules carried over from REA and CLI-Anything:
- A skipped lane never counts as coverage. `--require-tools X` turns a missing tool into FAILED.
- An unreachable backend is a run error, not a score.
- Every record has a mandatory `non_claims` field.

Sources: `[rea-tool-design-philosophy-and-verification §7.9,7.12]`, `[survey-capability-discovery-and-distillation §6.2]`.

---

## 10. Ingesting CLI-Anything harnesses

Facts observed on this machine (owner's fork at `/Users/nhanclie/CLI-Anything`, HEAD `34f5195`):
- `registry.json` has `{meta, clis[79]}`. Entries have `name`, `display_name`, `version`, `description`, `requires`, `homepage`, `source_url`, `install_cmd`, `entry_point`, `skill_md`, `category`, `contributors`.
- There are 69 `*/agent-harness` dirs, and 58 of their `*_cli.py` files define `def main()`. In LibreOffice's harness, `main()` just calls `cli()`, so the console-script entry point is a function and **not** the `click.Group`. My first probe crashed on exactly this. The harvester has to import the module and pick the root group.
- Installed from the local harness dir into a private venv (`uv venv` + `uv pip install <dir>`), the LibreOffice harness brought in Click 8.5.0. `to_info_dict()` returned **40 leaf commands**, each with typed params (`Int`, `String`, `Bool` flags, `Choice` with values, `multiple`, `required`, `default`).
- Root group params: `--json` (flag), `--project` (path to the project JSON), `--dry-run` (no-persist). They must come **before** the subcommand. A misplaced `--dry-run` gave a usage error, exit 2.
- Writes autosave to the `--project` file. `writer list` left the project file's SHA-256 unchanged, which is the evidence L3 records to lower a read.
- `export render out.odt --preset odt` produced a valid ZIP (`PK\x03\x04`, 1520 bytes) without soffice. `--preset pdf` failed: `/opt/homebrew/bin/soffice` is a shim, and `/Applications/LibreOffice.app` is absent. That is exactly a `PRECONDITION_UNMET` case.

Import flow, `aa import cli-anything <repo> --harness libreoffice --install private-venv --authorization "…"`, via `TargetService.ImportCliAnything`:
1. **STATIC.** Parse `registry.json` and the harness dir. Record `git rev-parse HEAD:<name>/agent-harness` as `identity.source_tree_sha` and the repo `LICENSE` (Apache-2.0) in `License`. Field mapping:

| `registry.json` | IR |
|---|---|
| `name` | `Target.id = "cli-anything.<name>"` |
| `display_name` | `Target.display_name` |
| `version` | `identity.version_banner` (harness version) |
| `description` | target description, `untrusted` |
| `requires` | `Precondition "binary:<x>"` (free text → `INFERRED`; resolved with `which` + realpath + SHA-256) |
| `entry_point` | executable name in the private venv |
| `skill_md` | `SourceRef` (descriptions/`use_when` **fallback only**, untrusted) |
| `category` | family tag |
| `install_cmd` | stored verbatim as a raw artifact, **never executed**: `cli-hub` runs these with `shell=True`, and in-repo entries are unpinned `git+` installs (`[survey-agent-interface-generators §2.7]`) |

2. **Install.** In `private-venv` mode, `uv venv envs/<id>` and `uv pip install <local harness dir>`. The engine owns the env, and the identity records the interpreter realpath and the installed dist version. In `preinstalled` mode, the entry point must resolve on PATH.
3. **INTROSPECT.** Run the `click-info` one-shot harvester with the venv's `python -I` in a private root.
   - It resolves the console script through `importlib.metadata`.
   - It picks the entry object if it is a `click.Command`; otherwise it picks the module-level `click.Group` with the most commands.
   - It emits `RawArtifact click-info.json` and one `CandidateCapability` per leaf.
4. **Characterize** with the `cli-anything` convention rules:
   - Click → JSON Schema:
     - `String` → string; `Int`/`IntRange` → integer with bounds; `Float` → number;
     - a flag → boolean (`ARG_STYLE_FLAG`); `Choice` → enum;
     - `Path`/`File` → string with `format: path` and a scope by mode;
     - `multiple` → array (`REPEATED_OPTION`); `nargs=-1` → array positional;
     - `required` maps over directly; `default` → DECLARED default; hidden params and `--help` are dropped.
   - `--json` always goes into `argv_prefix` and is never exposed.
   - `--project` becomes the state-handle param `project` (optional input on every leaf).
   - `--dry-run` becomes `DryRun{NO_PERSIST}`. It is not an input; it is the aa-level `dry_run` flag.
   - Errors use `ERROR_CONVENTION_CLI_ANYTHING_JSON`.
   - A leaf named `repl`, or a group with `invoke_without_command=True` invoked bare, is withheld as `INTERACTIVE`. The binding never calls the group without a leaf.
   - Autosave rule: any leaf that raises `mutates_target` and accepts `project` also raises `writes_filesystem` (scope `STATE_HANDLE`).
5. **Overlay bootstrap.** The importer writes a starter `target.aa.yaml` with `state_handle`, `exclude: [repl]`, and pairing proposals (§9.2). The `assume` block is left commented out until the operator runs `aa target assume <id> network=false launches=false foreign_code=false`, or passes `--accept-assumptions` at import. Either way the assumption is attributed.
6. `aa distill` then verifies and packages.

Expected result on this machine:
- the reads servable;
- `writer.*` and `calc.*` adds servable after L4;
- `*.remove*` servable as DESTRUCTIVE (ask at runtime);
- `export.render` servable once a proposal narrows `--preset` to the ODF/HTML/text presets, and withheld `PRECONDITION_UNMET: binary:soffice` for PDF/DOCX presets;
- `session.*` excluded with the observed reason.

Two non-claims are recorded:
- "L4 does not establish that LibreOffice rendered this file". This is the CLI-Anything doctrine/practice gap; the GIMP harness's Pillow fallback is the cautionary case (`[survey-agent-interface-generators §2.5]`).
- "Network not observed at ISOLATION_PROCESS".

`public_registry.json` (third-party CLIs) and `matrix_registry.json` (CLI matrices) are read as **target suggestions** only. Those targets go through their native harvesters (Cobra and others). Matrices map to future recipes (deferred).

---

## 11. Front ends

### 11.1 `aa` CLI (`aa-cli`)
- **Engine-ops subcommands** are static clap code:

```
daemon, status, stop, import cli-anything, target add|ls|assume,
harvest, compile, propose context|apply, verify, package, distill,
ls, describe, search, run, x, log, approve, mcp, adapter conformance, doctor
```

- **`aa run <target> <cli_path…> [flags]`** builds a clap `Command` tree **at runtime** from `GetExposure(CLI)`:
  - string → `String`; integer → `i64` parser; boolean → flag; array → `num_args(1..)`/repeated; enum → `PossibleValues`.
  - The tree is cached on disk by `catalog_digest`, so `--help` is instant.
  - Flags are sugar. `--input-json <inline|file|->` is always available, with stdin capped at 8 MiB (REA `rea:src/cliJsonInput.ts:31-164`).
  - `aa x <capability-id> --input-json …` is the canonical form.
- **Output:**
  - pretty JSON on a TTY, compact when piped;
  - `--format jsonl` for streams, with a mandatory trailer record;
  - `--full-output` envelope `{ok, data|error, meta:{action_id, catalog_digest}}`.

  Output-flag errors are rejected before dispatch (`[rea-cli-surface-and-agent-packaging §What to distill.4]`).
- **Exit codes:**

| Code | Meaning |
|---|---|
| 0 | completed (completeness lives in payload coverage) |
| 1 | failed |
| 2 | usage / schema rejected |
| 3 | needs confirmation or authority (`CONFIRMATION_REQUIRED`, `POLICY_DENIED`, `CAPABILITY_WITHHELD`) |
| 130 / 143 | signal, set after cleanup |
| 141 | EPIPE |

  One classifier maps `ErrorProjection.code` → exit code (`[rea-cli-surface-and-agent-packaging §What to distill.5-6]`).
- **`aa approve [<id>]`** shows the engine-authored challenge and requires a TTY. It is labelled an operator channel. In single-uid mode it is not a boundary (§7).

### 11.2 MCP server (`aa mcp`, crate `aa-mcp`)
- Launch: `aa mcp [--target X]… [--profile typed|search] [--families …]` over stdio, using rmcp 3.8 `ServerHandler`.
- `list_tools` comes from `GetExposure`. It is implemented manually, not with macros, because tools are data.
- `call_tool` maps to `Invoke`. Progress becomes MCP progress notifications. `structuredContent` is the output when an output schema is known. Errors are `isError` carrying the `ErrorProjection`.
- Fixed tools: `aa_status` (catalog digest, kill switch, servable counts) plus the search-profile tools. Withheld capabilities are **not** listed as tools; `aa_search` shows them with reasons.
- **Confirmation.**
  - If the client advertised `elicitation` at initialize (REA checks this, `rea:src/server/mcpClientMetadata.ts:58-66`), `aa-mcp` elicits with the engine-authored summary and the dry-run preview.
  - Accept → `ResolveApproval(channel=MCP_ELICITATION, scope=once)` → re-`Invoke` with the single-use token, which is bound to `(capability, input_sha256, expiry)`.
  - Decline or cancel → `OUTCOME_NOT_DISPATCHED`.
  - Without elicitation: `CONFIRMATION_REQUIRED` with `recovery: REQUEST_CONFIRMATION` and the instruction `aa approve <id>`. Elicitation is never used for secrets (`[survey-safety-rails-and-authorization-models §9]`).
- **Doctor.** `aa mcp doctor` spawns the real `aa mcp` as an MCP client. It checks the initialize identity, exact inventory equality with `GetExposure`, the digest, and one target-free call, as REA's `mcp doctor` does (`[rea-cli-surface-and-agent-packaging §Summary]`).

### 11.3 Agent Skills
- Rendered per target by `aa-render skill-md` (§9.4). The generated sections are marker-delimited, so hand-written judgment can sit around them.
- `skills/aa-distill/SKILL.md` is the one hand-authored skill. It teaches an agent the loop "context → proposal → apply → verify → read withheld hints → iterate". It states the fence explicitly: you cannot lower effects, enable tools or grant authority.

---

## 12. State and persistence

- **Files are the product, and SQLite is runtime state.**
  - Locks, overlays, verification records and packages are files: diffable, committable and digest-addressed.
  - The CAS holds immutable blobs.
  - `state.db` holds only things that are appended or mutated at runtime.
- **SQLite** uses `rusqlite 0.41` with `bundled`, WAL, and one writer task fed by an mpsc channel; readers use a pool. It is synchronous, statically linked, and needs no async driver. Tables:

| Table | Columns / purpose |
|---|---|
| `actions` | `seq INTEGER PK, action_id TEXT UNIQUE, target_id, capability_id, record BLOB (proto), record_sha256, prev_sha256, created_at` |
| `approvals` | id, challenge, input_sha256, expiry, resolution, channel |
| `sessions` | M3 |
| `harvest_runs` | |
| `meta` | |

- **Hash chain.** `record_sha256 = sha256(JCS(record with record_sha256=""))`, and `prev_sha256` links to the previous row. `AuditService.VerifyChain` walks the chain. The head hash is mirrored to `audit.head` so that truncation is visible. This is **tamper-evident, not tamper-proof**: the same uid can rewrite both.
- **Every Invoke writes exactly one ActionRecord, including refusals.** A denial records `OUTCOME_NOT_DISPATCHED` (`[rea-contracts-and-evidence-model §action record]`).
- **Redaction.**
  - Inputs are stored as `input_sha256` plus `input_redacted_json`. Params with `x-aa-secret` in the schema are blanked.
  - stderr excerpts are bounded and pass through token redaction that handles truncated prefixes (`[rea-engine-bridges-and-process-supervision §What to distill.3]`).
- **Retention.** Action records are kept forever by default. CAS blobs are garbage-collected when no lock, record or action references them (`aa gc`).
- **Drift.** On identity change (exe hash or version), the engine refuses Invoke for that target and marks capabilities `candidate`. `aa distill` re-harvests and diffs, and only changed capabilities lose admission. The rest keep it if the golden replay agrees (M4).

---

## 13. Policy (enough for the slice, full in M2)

```toml
# ~/.config/agentic-anything/policy.toml   (deny > ask > allow; default deny)
default = "deny"
[[allow]]
target = "cli-anything.libreoffice"
max_level = "write"
write_paths = ["~/Documents/aa-out/**"]
[[ask]]
level = "destructive"
[[deny]]
capability = "*.license.*"        # also: licence/activation/register/drm surfaces are EXCLUDED_BY_POLICY at characterize time
```

- Rules are evaluated on **structured descriptors**: target, capability, level, tier, and the canonicalized `WRITE_PATH`/`HOST` scope values. They are never evaluated on command text, because "matching is textual" is not a boundary (`[survey-safety-rails-and-authorization-models §3.6]`).
- The engine has a kill switch (`aa stop` / `SetKillSwitch`): it cancels in-flight actions and refuses new Invokes.
- A circuit breaker trips after 3 consecutive or 20 total denials per client session (the Claude Code pattern, same report).
- Hard exclusions are applied at characterize time. Capabilities or targets matching licence, activation, DRM or anti-cheat surfaces are `EXCLUDED_BY_POLICY` and cannot be re-enabled by overlay. This follows REA's fuse-gate stance (`[rea-desktop-applications-and-agent-environments §What to distill.13]`).

---

## 14. Testing and verification strategy

1. **Property tests** (`proptest`) on `aa-ir::effects`:
   - combination is commutative, associative and idempotent;
   - adding any signal never lowers any field's unsafe rank;
   - `PROPOSED`/`INFERRED` signals never produce a FALSE;
   - projections are monotone (a higher effect never yields a lower level or tier).

   JCS digests are checked against RFC 8785 test vectors.
2. **Set-equality conformance**, REA style, with no snapshot files for invariants (`[rea-capability-registry §What to distill.5]`):
   - the servable lock set equals the `GetExposure` enabled set, which equals the live `aa mcp` `tools/list` (spawned for real);
   - every capability is CLI-projected or has a typed `cli_omit_reason`;
   - every servable WRITE has a PASSED `LANE_READ_BACK` record with the current `lock_digest`;
   - the catalog digest recomputed by the test equals the served one;
   - dependency-direction check over `cargo metadata`.
3. **Harvester fixtures.** Raw artifacts recorded from real tools are committed under `aa-testkit/fixtures/raw/`: `click-info.json` from the LibreOffice harness, `gh __complete` transcripts, `shortcuts --experimental-dump-help`, an MCP `tools/list`. Harvest → candidates is snapshotted with `insta`. Snapshots are appropriate here because the normalization diff is what reviewers should read.
4. **Proposal fence tests.** A table of hostile proposals must be rejected:
   - lower an effect;
   - set `additionalProperties: true`;
   - widen a type;
   - remove `required`;
   - invent a capability id;
   - stale `base_lock_digest`;
   - an oracle that references a withheld read;
   - a proposal containing `assume`;
   - a description with control characters or instructions ("ignore previous…", which is stored as data and flagged).

   **cargo-mutants** runs on `aa-ir::effects`, `aa-proposal` and `aa-policy` with a **blocking** score floor. REA keeps its mutation score non-blocking, but the authorization core deserves better (`[rea-tool-design-philosophy-and-verification §7.10]`).
5. **`aa-exec` torture tests.** The test child:
   - ignores SIGTERM;
   - forks a grandchild that outlives it;
   - floods stdout;
   - sleeps past its deadline;
   - writes outside its private root.

   Expected: the group is reaped, output is bounded, the outcome is `UNKNOWN`, the survivor is reported, and the out-of-root write is *not* claimed as observed.
6. **Hermetic end-to-end with `fixture-notes`.** This is an ~100-line Click app shaped like a CLI-Anything harness, with:
   - global `--json/--project/--dry-run`;
   - `{"error","type"}` errors;
   - `notes list|add|remove|purge`;
   - `export --output` writing a ZIP;
   - one *lying* command, `notes stats`, which claims read but writes `~/.cache/notes/stats` inside the redirected HOME, so L3 must catch `SIDE_EFFECT`;
   - one command that ignores an unknown flag, so L1 must flag `PARSER_MISMATCH`.

   CI needs only `uv` and Python. No real app.
7. **Live lanes.** `aa verify cli-anything.libreoffice --require-tools soffice` runs nightly on a runner with LibreOffice installed. Locally, a missing tool is reported as a skip with a reason, never as a pass.
8. **Front-end tests.**
   - `assert_cmd` exit-code tests on `aa`, covering 0/1/2/3 and SIGINT → 130 after cleanup.
   - An rmcp client test driver checks `aa mcp`: initialize, `tools/list`, a read call, a write call (result plus action record), and a destructive call (elicitation path and no-elicitation path).
9. **Adapter conformance** runs against the Python kit in CI.
10. **Agent-eval lane** (M6): REA's gate shape, with the oracle changed to resulting app state read through an independent channel (`[rea-tool-design-philosophy-and-verification §7.11]`).

---

## 15. The first vertical slice (one day, four agents in parallel)

**Goal:** `aa import` + `aa distill` turn the CLI-Anything LibreOffice harness into servable MCP tools and `aa run` commands. Reads are admitted by observation, `writer.add-paragraph` is admitted by a read-back oracle, and every call lands in the hash-chained log. The hermetic `fixture-notes` target proves the same pipeline in CI.

**Hour 0, done by one agent before fan-out:**
- Land `proto/` exactly as in §6.
- Land `aa-proto` with the verified `build.rs` recipe.
- Land workspace skeleton crates with `todo!()` service stubs returning `UNIMPLEMENTED`.

The proto is the contract between the agents.

| Agent | Delivers | Done when |
|---|---|---|
| **A: IR + store** | `aa-ir`: ids, JCS digest, effects lattice and projections, admission matrix, overlay YAML (keys: `assume`, `state_handle`, `exclude`, `fixtures`, `capabilities.*.{effects,oracle,examples}`, `core`), lock read/write. `aa-store`: workspace layout, CAS, SQLite `actions` with hash chain + `VerifyChain` | proptest suite green; lock round-trips byte-identically |
| **B: exec + bind + verify** | `aa-exec` (process group via process-wrap, deadline, caps, private root, env scrub, poison). `aa-bind::cli_argv` (argv builder with `--` rule, CLI-Anything error convention, identity re-check). `aa-verify` lanes L0, L1, L3, L4, fixture builder, `VerificationRecord` | torture tests green; L3 catches the lying `notes stats`; L4 admits `notes add` and rejects an oracle that does not fail on the seed |
| **C: harvest + characterize** | `adapters/python/aa_click_harvest.py` (one-shot, ~120 lines, logic as observed in §10). `aa-harvest`: `cli-anything-registry`, `click-info` (via `aa-adapter-host` one-shot runner). `aa-characterize`: Click → schema table, CLI-Anything conventions, verb/option rules v1, oracle-pairing proposals, MCP naming. `aa-testkit/fixture-notes` | `click-info.json` fixture → 40 LibreOffice candidates snapshot; fixture-notes → expected lock |
| **D: engine + front ends** | `aa-engine`: UDS listener with peer-uid interceptor, in-process duplex. RPCs: `EngineService.GetStatus`, `TargetService.ImportCliAnything`, `DistillService.{Harvest,Characterize,Compile,Verify,Package}`, `CatalogService.{ListCapabilities,DescribeCapability,GetExposure}`, `RuntimeService.Invoke` (policy v0), `AuditService.QueryActions`. `aa-render`: `mcp-typed` + `skill-md`. `aa-client`; `aa-cli` (`daemon, import, distill, ls, describe, run, x, log, mcp`); `aa-mcp` (stdio list/call) | e2e test below passes with `--embedded` and against a real daemon |

**Policy v0** (deliberately minimal):
- The target must be allow-listed in `policy.toml`.
- Servable READ and WRITE are allowed within `max_level`, with write paths checked.
- DESTRUCTIVE and UNKNOWN return `CONFIRMATION_REQUIRED`/`CAPABILITY_WITHHELD` with no approval path yet (M2).
- `idempotency_key` is auto-generated by front ends.

**Acceptance script** (live part, run on this machine):

```sh
aa import cli-anything ~/CLI-Anything --harness libreoffice --install private-venv \
   --accept-assumptions --authorization "owner workstation; MPL-2.0 app; Apache-2.0 harness"
aa distill cli-anything.libreoffice
aa ls cli-anything.libreoffice
#   40 harvested; repl excluded; session.* excluded (reason shown)
#   READ servable: document.info, writer.list, calc.list-sheets, calc.get-cell, export.presets, ...
#   WRITE servable: document.new, writer.add-paragraph (vr_... read_back PASSED), ...
#   DESTRUCTIVE (ask): writer.remove, calc.remove-sheet, ...
#   WITHHELD: export.render (no_oracle / precondition_unmet binary:soffice) until a proposal narrows --preset
mkdir -p ~/Documents/aa-out
aa run cli-anything.libreoffice document new --type writer --name R --output ~/Documents/aa-out/r.json
aa run cli-anything.libreoffice writer add-paragraph --project ~/Documents/aa-out/r.json --text Hello
aa run cli-anything.libreoffice writer list --project ~/Documents/aa-out/r.json      # -> [{"index":0,"type":"paragraph","text":"Hello"}]
aa run cli-anything.libreoffice writer remove 0 --project ~/Documents/aa-out/r.json; echo $?   # -> 3 (confirmation required; nothing dispatched)
aa log --last 4          # four ActionRecords incl. the NOT_DISPATCHED one; chain intact
claude mcp add aa -- aa mcp --target cli-anything.libreoffice
#   tools/list: libreoffice_writer_add_paragraph {readOnlyHint:false, destructiveHint:false, idempotentHint:false, openWorldHint:false}
#               libreoffice_writer_list {readOnlyHint:true, ...}; no session/repl tools; SKILL.md at packages/skill/aa-cli-anything-libreoffice/
```

**Hermetic part, in CI:** the same flow over `fixture-notes` as a nextest e2e, with no network and no real app.

**Out of the slice:**
- proposals (M1);
- other harvesters (M1);
- approvals and elicitation (M2);
- long-lived gRPC adapters (M3);
- TCP;
- sessions.

---

## 16. Milestones after the slice

These map onto the GitHub epic's M0–M4 where possible.

| Milestone | Content | Exit criterion |
|---|---|---|
| **M0** (ADRs) | ADRs extracted from this doc: IR/lock format, effects lattice + lowering rules, admission matrix, adapter protocol, action-record schema, approval channels | judged against the other proposals |
| **M1** Distill breadth | `ApplyProposal` + `aa propose` + `skills/aa-distill`; harvesters `usage-kdl`, `swift-dump-help`, `cobra-complete`, `json-fields`, `help-text`, `mcp-tools-list` (+ `mcp-tool` binding), `openapi` (+ `http` binding); L2 dry-run lane; `cli-tree` render; `mcp-search` profile; `aa compile/package --check` in CI; `WatchCatalog` → MCP `tools/list_changed` | `gh`, `docker` (read-only core), `shortcuts`, a local MCP server and an owned OpenAPI service all reach READ-servable in under 10 minutes each, measured |
| **M2** Authority | full `policy.toml` (scopes, ask rules), approvals over MCP elicitation and the TTY, dry-run previews in challenges, kill switch + circuit breaker, idempotency receipts (`GetActionStatus`), `VerifyChain` in doctor; **two-uid spike** | destructive actions need a human on a non-agent channel in two-uid mode |
| **M3** Adapters + Mac surfaces | long-lived Adapter gRPC over UDS; Python and Swift bridge kits + conformance; `Lifetime.SESSION`; harvesters `sdef`, `app-intents`, `ax-menubar`; one in-host bridge (Blender's Python, installed locally); UI level with an a11y oracle | Music/Books READ via sdef; one Blender write admitted by a bridge read-back |
| **M4** Verification depth + drift | L5 golden + replay, L6 differential, identity drift → re-distill, Linux Landlock/bubblewrap lane (`ISOLATION_CONTAINER`), `non_claims` linting, agentenv-style export | upgrading a harness re-admits only unchanged capabilities |
| **M5** Breadth II + packaging | `graphql`, `grpc-reflection` (+ dynamic `grpc` binding), `dbus`, `usage-kdl` export, `server.json`, plugin manifests, `aa doctor` | |
| **M6** Agent eval | L8 lane, per-task token accounting, CLI vs MCP exposure measurement (`[survey-agent-interface-generators §Open questions]`) | |

---

## 17. Explicitly deferred (and why)

| Deferred | Why |
|---|---|
| An LLM client inside the engine | Keeps the engine deterministic and the trust decision in one validator. Any agent can propose through the API. |
| Generating adapter **code** (CLI-Anything-style synthesis) | CLI-Anything already does this, under Apache-2.0, with 69 harnesses. We ingest their output and verify it. Writing our own synthesizer duplicates their work and inherits their drift problem. |
| UI exploration (`EXPLORE` authority), AppAgent-style | Needs disposable per-OS sandboxes (a VM on macOS with TCC grants). It is the only discovery method that acts (`[survey-capability-discovery-and-distillation §3.6]`). |
| Vision / computer-use fallback | Lowest fidelity rung of the technique ladder. Nothing in the generator lens needs it. |
| `inject` tier / dynamic instrumentation | Own-software only, and highest risk. Excluded by default. |
| Windows native (named pipes, UIA, COM harvester) | TCP+token works now. Native work comes after the macOS and Linux core is proven. |
| MCP over streamable HTTP | stdio covers local hosts. HTTP needs REA-grade Host/Origin hardening and session handling. |
| Code-mode exposure (one `execute` doing many ops) | It weakens per-call authorization unless host callbacks re-check every operation (`[survey-agent-interface-generators §6]`). The search profile gets most of the token benefit without that risk. |
| Two-uid mode as the default | It is the real boundary, but the setup burden is high. It is spiked in M2 and the single-uid limits are documented. |
| Fig/carapace effect priors | Third-party, version-lagged, and redistribution licence unclear (`[survey-capability-discovery-and-distillation §Open questions.6]`). |
| usage KDL as the **source** format | We export it (M5). The IR needs effect vectors, oracles and bindings, which KDL lacks. This answers discovery open question 1. |
| CLI-Anything REPL/session support | The REPL is interactive. The undo stack does not persist across one-shot calls (observed). Sessions come in M3 if a harness needs them. |
| `cli-hub` registry install | It executes remote shell strings and makes unpinned installs. We install only from a local checkout, into an engine-owned venv. |
| Multi-step recipes (Arazzo-like, `matrix_registry.json`) | REA's rule is "primitives first". Recipe effect aggregation is an open question. |
| Remote / multi-user engine, TLS, non-loopback | Out of scope for a local control plane. It would require real authentication. |

---

## 18. Risks

1. **The "minutes" illusion.** Scaffolding is fast and correctness is slow; REA's tail was 3–5× the first commit. *Mitigation:* `TargetStatus` plus per-capability admission visible everywhere, including the skill's withheld section. Track the time spent by each target in each state.
2. **Over-conservatism starves the catalog.** If most capabilities sit at `effects_unknown`, users will bypass the gates. *Mitigation:*
   - operator `assume` for fields that cannot be observed;
   - L3 lowering for fields that can;
   - every withheld entry carries a specific hint;
   - the KPI is "% servable after `aa distill` with no overlay edits", measured per harvester.
3. **Weak sandbox at `ISOLATION_PROCESS`.** It cannot see network traffic or writes outside the root, so a "read" could be admitted wrongly. *Mitigation:*
   - those fields cannot be lowered by L3 at that isolation;
   - `non_claims` say so;
   - Landlock/bubblewrap comes in M4.
   
   macOS has no supported per-process FS sandbox (`sandbox-exec` is deprecated), which is a real gap.
4. **Introspection runs foreign code.** Importing a Click harness executes module top-level code, and Cobra completions can hit the network. *Mitigation:* INTROSPECT authority requires the target's `authorization_statement`, a scrubbed env, a private HOME, a deadline, and no network by default.
5. **Single-uid approvals are not a boundary** against an agent that has a shell. *Mitigation:* honest labelling, and two-uid mode in M2.
6. **CLI-Anything quality varies,** and harness doctrine has drifted (Pillow fallbacks). *Mitigation:* we verify behaviour, not doctrine. Backend preconditions are identity-bound, and the non-claim "real backend rendered this" stays unless a precondition proves it.
7. **Two schema languages.** Proto defines the IR and JSON Schema defines tool inputs. *Mitigation:* JSON Schema is carried as canonical strings and validated with `jsonschema`. `buf breaking WIRE_JSON` protects the on-disk JSON form. Schema-narrowing only runs over a supported subset, and anything else is rejected.
8. **Context blowup** when large targets mirror every capability. *Mitigation:* a core set of ≤ 30, the search profile beyond that, and families (`[rea-tool-design-philosophy-and-verification §7.5]`).
9. **Crate churn.** rmcp is on major version 3, and tonic 0.14 split out its prost crates. *Mitigation:* pin exact versions. rmcp stays behind `aa-mcp` and tonic stays behind `aa-proto`/`aa-client`. The verified `build.rs` recipe is recorded in §5.
10. **Daemon lifecycle bugs:** autostart races, stale sockets. *Mitigation:* lock-guarded startup, `--embedded` for CI, and `aa doctor` checks.
11. **Prompt injection through harvested text,** into the proposing LLM and into agents. *Mitigation:*
    - every harvested string is flagged untrusted;
    - context packs use spotlighting;
    - proposals cannot raise authority or lower effects;
    - descriptions an LLM wrote are flagged in `_meta`.
12. **Licensing and authorization scope.** CLI-Anything is Apache-2.0: we execute harness code, we do not vendor it, and NOTICE text is recorded. Target app licences are recorded per target. Hard exclusions (licence, activation, DRM and anti-cheat surfaces) are enforced at characterize time and cannot be re-enabled by overlay.
