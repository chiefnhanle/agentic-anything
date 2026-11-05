# REA tool design philosophy and verification: how REA keeps tools agent-friendly and proves them

Research date: 2026-11-05

Area: `tool-design-philosophy-and-verification`. This is the full rewrite of a lost report. It checks and extends the
salvaged summary at `docs/research/salvaged/tool-design-philosophy-and-verification.json`.

Source: the owner's local REA fork at `/Users/nhanclie/rea`, HEAD `3484432fb18482dcced81272e6181719a3cdfbfa`
(committed 2026-11-05, `package.json` version 6.8.0). The salvaged summary was written against upstream `93b8b82`.
Some counts have moved since then; see "Corrections to the salvaged summary". Paths are relative to the REA root.
Nothing in REA was modified, built or run. I computed one set of numbers by importing a *copy* of
`src/contracts/toolEffects.ts` into the scratchpad with Node 25 type stripping; these are labelled OBSERVED (computed).

Related reports, referenced here rather than repeated:
- `rea-contracts-and-evidence-model.md`: covers the `ToolContract` type, the Evidence record, the tagged error algebra and
  the projection internals.
- `rea-mcp-server-adapter.md`: covers the deferred/eager runtimes, transports, the recorded catalog and the schema
  profile plumbing.
- `survey-safety-rails-and-authorization-models.md`: covers authority and confirmation models.

This report covers the parts those reports do not: the design *rules*, the *gates that enforce them*, and the
*verification ladder*.

Evidence legend:
- **OBSERVED**: read in the REA checkout, cited as `path:line`.
- **SECONDARY**: third-party or summary material.
- **INFERRED**: my interpretation.
- **UNKNOWN**: not established.

---

## 1. Summary

1. **REA's surface philosophy is task-first and fails closed.** It has three parts:
   - A tool exists for a *distinct outcome or materially different authority* (`docs/tool-design.md:7-9`).
   - Tools take one of six *task shapes*, and REA does not mix discovery, execution and mutation modes in one tool
     (`docs/tool-design.md:14-27`).
   - Primitives come first. A workflow is composed only when "repeated use demonstrates" a multi-source outcome
     (`docs/tool-design.md:29-49`). OBSERVED.
2. **Context cost comes first, and it is engineered.** "Agent tokens and context" is the first performance priority,
   counted "for the whole task, including schemas, extra calls and retries" (`AGENTS.md:47-53`). The mechanisms are:
   - `detail: "summary"`, which returns a retained-parent summary (`docs/mcp-contracts.md:46-58, 247-252`).
   - One universal pager, `inspect_analysis_view`.
   - A default *compact* input-schema profile that holds every advertised schema to 13,312 bytes and ≤10 levels of depth
     (`docs/mcp-contracts.md:1761-1847`).
   - An opt-in 4,096-byte output-schema profile (`docs/mcp-contracts.md:1935-1942`).
   - A `REA_MCP_TOOL_FAMILIES` scope (`docs/mcp-contracts.md:1849-1918`).
   - Advertised examples never embed full Evidence (`docs/mcp-contracts.md:1716-1721`). OBSERVED.
3. **Server-side validation never weakens.** Every advertised projection can be *wider* than the canonical Zod contract,
   and the handler always validates against the canonical contract (`docs/mcp-contracts.md:1697-1703, 1753-1755`). This
   is how REA reconciles strict contracts with picky clients (Gemini, Moonshot/Kimi, strict Anthropic-compatible
   gateways, Pi). OBSERVED.
4. **One closed error envelope serves every CLI/MCP error.** The envelope is
   `{code, category, message, retryable, remediation{action}, recovery?, details?}`, with 20 codes, 10 categories and 3
   recovery kinds. It is generated to `docs/error-contract.schema.json` and checked for freshness
   (`src/contracts/errorSchemas.ts:7-132`; `scripts/generate-error-schema.mjs:14-21`). The design rules for recovery:
   - Distinguish a malformed request from a valid selector the target cannot resolve.
   - Keep the repair at the owning contract.
   - Never auto-switch context (`docs/tool-design.md:102-143`).
   - Recovery text must name the limit, its value and unit, the source, the bounds, where to set it, whether a restart is
     needed, and what the change does *not* raise (`docs/design/limits-and-recovery.md:288-298`). OBSERVED.
5. **Effects are data, and a test keeps descriptions honest against them.** Beyond the build-time throw on an unaudited
   tool (`src/contracts/toolEffects.ts:818-822`), `src/contracts/toolEffects.test.ts:16-170` does three things:
   - It regex-scans every description for run/launch/execute/start/install wording.
   - It fails when such a tool declares neither `executesForeignCode` nor `mutatesDevice`, unless it has a written
     exception.
   - It also fails on stale exceptions.

   A frozen golden list of the 22 foreign-execution/device-mutation tools must all derive `destructiveHint: true`
   (`toolEffects.test.ts:172-205`). OBSERVED.
6. **The annotations are coarse in practice.** Of 234 audited tools, only **9** derive `readOnlyHint: true`. **109**
   are non-read-only *solely* because they record session Evidence. 33 are destructive, 43 are non-idempotent and 101
   are open-world. OBSERVED (computed from a copy of `src/contracts/toolEffects.ts`). This is the strongest concrete
   argument for splitting "mutates the target" from "mutates the control-plane session" in agentic-anything.
7. **The verification ladder is explicit.** In order of strength:
   1. A real public CLI/MCP workflow.
   2. Production-boundary integration.
   3. Producer goldens.
   4. Module tests, kept only for semantics that stronger workflows cannot reproduce (`docs/testing.md:420-424, 514-517`).

   Seven behavioural depths each have a defined scope (`docs/testing.md:849-868`). Every rung states what it does *not*
   prove: "a scripted sequence tests that sequence, not an agent's ability to discover it" (`docs/testing.md:465-469`).
   "A lane must not imply that a host or target is covered when it was skipped" (`docs/testing-lanes.md:18-23`).
   OBSERVED.
8. **Mutation testing is narrow and AST-anchored.** Two Stryker configurations target named lifecycle and cleanup code:
   - `stryker.config.mjs`, which uses the Vitest runner with `perTest` coverage.
   - `stryker.semantics.config.mjs`, which uses a command runner with coverage off to run every mutant exhaustively.

   Targets are resolved from TypeScript ASTs by selectors such as `{kind, owner, method, ...}`. The resolver throws when a
   selector matches zero or more than one node (`scripts/lib/reverse-tools-mutation-targets.mjs:139-181`). Mutation
   score is non-blocking (`thresholds: {break: null}`). Required CI only runs `--dryRunOnly` preflights
   (`.github/workflows/ci.yml:392-393`). The full sweep runs daily (`.github/workflows/mutation.yml:5`). OBSERVED.
9. **Agent usability is measured with a real agent and graded by a strict formula.**
   `agentEvaluationPassed` (`tests/evaluation/model-evals/AgentEvaluationReport.ts:24-42`) requires all of:
   - exit 0;
   - correct natural use (or *no* REA calls for the negative "missing-target" scenario);
   - the correct first tool;
   - zero repeated calls;
   - zero input-validation failures;
   - an ordered required subsequence of successful tools;
   - an expected scenario outcome;
   - non-zero tokens;
   - a passed factual rubric, or (if no rubric exists) heuristics that REA itself labels non-factual.

   Repetition is keyed by target revision for `binary_session` (`tests/evaluation/model-evals/CodexAgentEval.ts:164-182`).
   OBSERVED.
10. **Verification is recorded as dated, revision-pinned, scope-limited records.** `docs/verification/` holds 53 files.
    Each record names its issue, baseline and final SHAs, executed scope and what is *not* claimed, a reproduction, and a
    JSON companion with source SHA-256s (e.g. `docs/verification/javascript-analysis-effects-2026-10-15.md:1-55`). One
    record shows the effects audit was *wrong* for a real tool (worker launch and staging omitted). The fix was a
    regression, not a runtime effect monitor. **REA does not mechanically verify declared effects against observed
    behaviour.** OBSERVED (record) / INFERRED (no monitor found).

---

## 2. Corrections to the salvaged summary

| Salvaged claim | Current fact | Status |
|---|---|---|
| "hand-audited for 235 tools" | `TOOL_EFFECTS` has 234 keys. `toolEffects.test.ts:143-147` asserts they equal `TOOL_CONTRACTS` names exactly. | OBSERVED (computed). The salvage was probably at another revision. |
| "packaged skill (v59)" | `.agents/skills/reverse-engineer-anything/SKILL.md:5` says `version: "60"`. The trust-the-live-catalog line is now `:26`. | OBSERVED |
| `docs/mcp-contracts.md:1719` "Claude Code measurements above" unknown | The dangling reference is now at `docs/mcp-contracts.md:1852`. I still found no Claude Code catalog-payload measurement in the file. | OBSERVED / UNKNOWN |
| Error schema has "20 oneOf alternatives" (salvage); 21 (contracts report) | 20 alternatives (`errorSchemas.ts:41-101`; parsed JSON). `category` is a free 10-value enum on every code, not tied to the code. | OBSERVED |
| Mutation-threshold behaviour not detailed | `thresholds: { break: null }` in both configs. Score is non-blocking (`stryker.config.mjs:20`; `docs/testing.md:964-971`). | OBSERVED |
| Effects not checked against behaviour: unknown | No runtime effect observer was found. Drift is caught by issue → regression (`docs/verification/javascript-analysis-effects-2026-10-15.md:3-25`) plus the wording test. | OBSERVED / INFERRED |
| "repeatedCallCount === 0 must be keyed by target state" (salvaged CHANGE) | Already partly done: `binary_session` signatures are prefixed with a target revision that increments on successful open/close (`CodexAgentEval.ts:164-178`). Other tools are not keyed. | OBSERVED |
| ~35 CI workflows, ~22 real lanes | 35 workflow files, of which 19 are `real-*.yml`. 114 `verify:*` npm scripts. | OBSERVED |

---

## 3. Tool design philosophy (how the surface stays small and agent-friendly)

### 3.1 Admission rule: when a tool may exist

- Design starts from "the analyst question". Inspect the canonical contracts, the provider capabilities and the
  "nearest existing tool" first. Extend a tool when "intent and result contract match". Add one only "for a distinct
  outcome or materially different authority" (`docs/tool-design.md:5-9`). OBSERVED.
- "Upstream capability alone does not require a new tool or equal support from every provider"
  (`docs/tool-design.md:56-58`). Lack of API parity is not a bug (`:99-100`). Public names stay provider-neutral, with
  coverage reported per provider (`:8-9`). OBSERVED.
- Before defining a tool, state the question, what the caller already has, the intended result and authority, and
  "what must remain unknown". Then try the nearest REA workflow *and* a native or script path (`docs/tool-design.md:53-58`).
  OBSERVED.
- "Keep the catalog complete; report capability/session availability instead of truncating schemas. Serialized bytes
  alone do not establish agent usability" (`docs/tool-design.md:11-12`). OBSERVED.
- Custom orchestration languages, replay engines or prepare/execute plans need "an observed requirement those
  primitives cannot satisfy. A plan without execution does not establish runtime behavior" (`docs/tool-design.md:46-49`).
  OBSERVED.
- AGENTS.md adds a cost gate. Before adding complexity for a "speculative or vanishingly unlikely race or security
  edge case", explain the failure mode, its likelihood and its cost, and get buy-in (`AGENTS.md:25`). The PR template
  repeats this (`.github/PULL_REQUEST_TEMPLATE.md:19-23`). OBSERVED.

### 3.2 Shapes and naming

- **Six shapes** (`docs/tool-design.md:16-23`). Each shape has a defined result:

  | Shape | Defined result |
  |---|---|
  | `inspect` | facts + facet availability |
  | `search`/`list` | stable ordering, pagination when needed |
  | `trace` | typed edges + unresolved paths |
  | `compare` | identity, comparable coverage, supported deltas |
  | `workflow` | inline answer + contributing evidence + partial facets |
  | `observe`/`capture` | authority, launch/attach effects, lifecycle, cleanup status |

  They are "task shapes, not mandatory prefixes. Name tools for the action and object callers reason about. Avoid
  unrelated discovery/execution/mutation modes inside one tool" (`:25-27`). OBSERVED.
- The verb distribution across the 234 audited names, by first token, is: `inspect` 65, `list` 21, `compare` 11,
  `trace` 10, `procedure` 7, `capture` 6, then `start`/`set`/`search`/`read`/`observe`/`get` at 5 each. OBSERVED
  (computed from `src/contracts/toolEffects.ts:54-762`). INFERRED: names are `verb_object` snake_case, the verbs are
  mostly read-shaped, and the effectful verbs (`start_`, `install_`, `push_`, `capture_`) are rare and distinctive.
- Titles are derived mechanically from names (`toolEffects.ts:823-827`). There is no hand-written title drift.
  Descriptions are hand-written and can be long; the `open_binary` description runs about 1.4 kB
  (`src/contracts/sessionToolContracts.ts:69-73`). I found **no description-length check**. OBSERVED (grep) /
  INFERRED (the gap).
- Server instructions are short, scope-aware strings. They name a tool only if that tool's family is advertised, "so
  guidance never names a disabled tool" (`src/server/serverInstructions.ts:8-44`). Guided prompts are hidden when any
  step's tool is out of scope (`docs/mcp-contracts.md:1869-1874`). OBSERVED.

### 3.3 Granularity

- A **primitive** "reports a reusable fact about an identified object or relationship". Format decoders qualify; "one
  application's business interpretation belongs outside the general contract" (`docs/tool-design.md:31-37`). OBSERVED.
- **Batching** is allowed "for homogeneous reads when that removes repeated work while preserving per-item outcomes"
  (`docs/tool-design.md:42-43`). The analysis-optimization skill applies the same rule: "Do not introduce an index,
  generic cache, batch endpoint, or orchestration layer without identifying the work it removes"
  (`.agents/skills/rea-analysis-optimization/SKILL.md:59-61`). OBSERVED.
- **Prompts are optional.** They avoid "prescribing call sequences that a direct tool can replace"
  (`docs/tool-design.md:43-44`). OBSERVED.
- Lifecycle tools (`open_binary`, `close_binary`, `reset_investigation`) are explicit, separate tools.
  `reset_investigation` is "the explicit destructive boundary" (`docs/tool-design.md:274-286`). OBSERVED.

### 3.4 Inputs

- Inputs are strict, with explicit target identity and required fields. REA preserves meaningful target, action,
  capture and output choices, and removes "ignored options and repeated permission declarations"
  (`docs/tool-design.md:147-149`). Contracts use `z.strictObject` (e.g. `src/contracts/sessionToolContracts.ts:60-65`).
  OBSERVED.
- Evidence inputs use one union across all tools: complete inline Evidence, or
  `{kind:"retained-evidence", evidence_id}`. Some tools also accept `{kind:"evidence-file", path}`. Resolution happens
  once in the shared application entrypoint, never in an adapter (`docs/tool-design.md:180-223`;
  `docs/mcp-contracts.md:29-38`). OBSERVED. The contracts report covers this in more detail.
- Expected input failures "must retain caller field/index paths and safe constraints"
  (`docs/tool-design.md:528-531`). SDK validation failures are normalized into the canonical `invalid_request` with
  field paths and constraints (`docs/mcp-contracts.md:185-197`). An unknown tool returns JSON-RPC `-32602` that points
  to `tools/list` (`:194-197`). OBSERVED.
- Root unions are flattened for model APIs, and partner-field rules move into descriptions. The reason is that Gemini
  rejects `dependentRequired`, and "one unknown keyword fails the client's whole tool catalog"
  (`docs/mcp-contracts.md:1723-1735`). A default is advertised only if every input group accepts it with the same value
  (`:1726-1728`). OBSERVED.

### 3.5 Results: answer inline, keep provenance and coverage honest

- Return the requested facts "inline" with identity, source locations, Evidence references, coverage, confidence and
  limitations. "Missing coverage is unknown, not absence. Avoid extra lookups solely to understand the answer or its
  provenance" (`docs/tool-design.md:155-160`). OBSERVED.
- Keep **discovered / applicable / examined / retained** scopes distinct. Ten discovered, eight examined and five
  returned does not mean complete coverage. "A filtered page can be complete for its selection while the parent
  inventory remains partial." "Structured completeness and absence fields must agree with the stated limitations"
  (`docs/tool-design.md:489-496`). OBSERVED.
- Coverage is a producer-owned structured field (`coverage.scope`, `coverage.status: complete|unknown`). "Limitations
  are explanatory and do not supply or infer coverage" (`docs/mcp-contracts.md:89-118`). Per-facet
  `facet_availability` separates "not observed" from "observed empty" (`:120-140`). OBSERVED.
- Multi-stage work retains each completed observation before the next stage. Failures carry partial facts "with
  explicit coverage of unperformed work; do not construct a successful result for stages that never ran"
  (`docs/tool-design.md:327-331`). OBSERVED.
- Limits only for "real format, protocol, authority, or measured resource constraints". "An assumed agent budget is
  insufficient justification for a limit." Name the ceiling and its recovery (`docs/tool-design.md:174-178`). OBSERVED.

### 3.6 Paging and summaries

| Mechanism | Rule | Source |
|---|---|---|
| `detail: "summary"` on list/search/name tools and graphs | Retains the complete Evidence. Returns row count, provider identity, profile digest, original query, `parent_evidence_id`, limitations and explicit unknown discovery coverage. Delivery detail is excluded from provider arguments and Evidence identity. | `docs/mcp-contracts.md:247-252, 299-310` |
| `analyze_function` default | Retains the full dossier and returns a summary whose `normalized_result.parent_evidence_id` names it. Consumers needing full facts reject the summary and take the parent reference. | `docs/mcp-contracts.md:46-58`; `docs/tool-design.md:208-211` |
| `inspect_analysis_view` | The single pager over retained Evidence (summary, section/module, stable page). Continuation via `normalized_result.coverage.next_offset`. Fixed 1 MiB page projection allowance. | `docs/mcp-contracts.md:236-238, 340-358` |
| Transport budget | Results are prepared within the pinned stdio client's 10 MiB receive buffer. An oversize result becomes `resource_constraint` with `details.resource:"transport"` and an `evidence_reference`, decided *before* building a document-sized string. Oversized errors are retained as Evidence too. | `docs/mcp-contracts.md:179-183, 227-239` |
| Contract flag | `ToolContract.retainedSummary.evidenceParameters: "omit-detail" \| "preserve-detail"`: "detail never reaches analysis". | `src/contracts/toolContractTypes.ts:52-55` |
| Page/cursor identity | A cursor reused with another query fails ("Byte search cursor does not match the target and query"). Pages of one inventory must carry the same coverage declaration. | `docs/user-visible-errors.csv` row 4; `docs/mcp-contracts.md:118` |

INFERRED: the pattern is *summary-first by default for large producers, complete-on-request (`detail:"complete"`),
and one generic pager keyed by evidence ID*. Each producer does not invent its own cursor protocol.

### 3.7 Schema and catalog advertisement budget

- The default input profile is `compact`. Every advertised input schema fits within a 13,312-byte budget that was
  measured below Moonshot's cap (#1484). Schemas deeper than 10 levels (#2813) get a reduced first-level projection.
  The profile also covers WorkBuddy/CodeBuddy's `11133` errors (#2999), strict Anthropic-compatible gateway translation
  failures (#3800) and Gemini `items` rules (`docs/mcp-contracts.md:1761-1808`). The compact profile drops nested
  descriptions, examples, defaults and titles, but keeps the root description and a root `detail` description
  (`:1814-1818`). OBSERVED.
- Output schemas default to `full`. `compact` (≤4,096 bytes per schema) and `off` are opt-in, for stdio hosts that
  reject a large `tools/list` frame (#2512). The server-identity tool-schema digest includes the selected profile
  (`docs/mcp-contracts.md:1929-1942`). OBSERVED.
- Families: `REA_MCP_TOOL_FAMILIES` scopes the registration. Disabled tools are omitted from `tools/list` but return a
  named `Tool ... disabled` error when called. Availability reports, prompts, recovery instructions and doctor all
  follow the same scope. Startup fails on unknown family IDs (`docs/mcp-contracts.md:1849-1889`). A workflow-to-family
  table makes scoping "additive and start narrow" (`:1904-1918`). OBSERVED.
- Examples: each advertised example is "a distinct valid canonical input". Embedded Evidence appears only as a
  retained reference (`docs/mcp-contracts.md:1716-1721`). Examples are parsed through the input schema when the module
  loads, so an invalid example throws (`src/contracts/toolContractHelpers.ts:22-33`). OBSERVED.
- Measurement discipline: "Fewer bytes alone do not establish fewer tokens" (`AGENTS.md:66-67`). The JS effects record
  reports a selected-schema task at 16,548 `cl100k_base` tokens versus 204,093 with the full catalog projection, and
  labels these as "static counts, not model billing"
  (`docs/verification/javascript-analysis-effects-2026-10-15.md:30-34`). OBSERVED. INFERRED: tool-family scoping cuts
  context by about 12x on that task.

### 3.8 Effects, annotations and untrusted content

The contracts report covers the vocabulary. The new findings here are about enforcement and its consequences:

- **Build-time completeness:** `toolContractMetadata` throws `Missing effect audit for <name>`
  (`src/contracts/toolEffects.ts:818-822`). `toolEffects.test.ts:143-147` asserts a one-to-one mapping between
  `TOOL_EFFECTS` and `TOOL_CONTRACTS`. OBSERVED.
- **Wording-vs-effects lint:** `EXECUTION_WORDING` is a regex for run/launch/execute/start/install/uninstall. A
  description that matches it must declare `executesForeignCode` or `mutatesDevice`, or carry an entry in
  `EXECUTION_WORDING_EXCEPTIONS` with a one-line reason. There are 74 such entries, e.g.
  `start_frida_session: "spawned targets remain suspended until resume_frida_session"`. Stale exceptions fail too
  (`src/contracts/toolEffects.test.ts:16-119, 149-170`). OBSERVED.
- **Frozen destructive set:** 22 named tools must have `readOnlyHint:false, destructiveHint:true`
  (`toolEffects.test.ts:172-205`). The derivation algebra is tested independently of any tool (`:207-238`). OBSERVED.
- **Annotation distribution (computed):** readOnly 9 of 234; destructive 33; non-idempotent 43; openWorld 101;
  `executesForeignCode` 16; `changesUiState` 7; `mayDiscardData` 16. **109 tools are non-read-only only because of
  `mutatesSession`.** The cause is that `mutatesSession` "includes additive Evidence recording"
  (`toolEffects.ts:4-5, 768-778`). OBSERVED (computed). INFERRED: an MCP host's approval policy keyed on `readOnlyHint`
  would prompt on almost every REA read.
- **Untrusted by default:** only 7 audited tools are content-free. A conformance test also checks that
  `docs/mcp-contracts.md` lists exactly those 7, using a regex over the prose (`tests/conformance/contracts/targetContentProvenance.test.ts:9-43`).
  A positive list of 21 tools must stay marked (`:45-72`). OBSERVED.

### 3.9 Error shapes and recovery

- **Envelope** (`src/contracts/errorSchemas.ts:19-35, 41-101`):
  - `code` comes from 20 literals: `invalid_request`, `access_denied`, `artifact_changed`, `unreadable_output`,
    `capability_unavailable`, `unsupported_target`, `provider_unavailable`, `provider_timeout`, `resource_constraint`,
    `cancelled`, `artifact_integrity_mismatch`, `artifact_operation_failed`, `evidence_integrity_mismatch`,
    `truncated`, `process_capture_failed`, `cleanup_incomplete`, `revision_conflict`, `configuration_invalid`,
    `target_unavailable`, `execution_failure`.
  - `category` comes from 10 values: `invalid_input`, `unsupported_provider`, `unsupported_target`,
    `integrity_mismatch`, `truncated`, `cancelled`, `timeout`, `resource_constraint`, `unavailable`,
    `execution_failure`.
  - `message` (min 1), `retryable` (boolean) and `remediation: {action}` (strict, required).
  - Optional `recovery`, one of `inspect_input_contract{operation}`, `inspect_timeout_state{operation,timeout_ms}` or
    `retry_cleanup{operation}`.
  - Optional `details` (JSON record). Only `artifact_integrity_mismatch` has a typed `details` union (sha256, md5,
    dfu-crc32).

  The CLI wraps the same projection (`:104-128`). The JSON Schema is generated and checked for freshness by a Turbo
  task (`scripts/generate-error-schema.mjs`; `turbo.json` `artifacts:error-schema`). OBSERVED.
- **Delivery:** `isError:true`, with the full `{error}` as JSON text. Failures omit `structuredContent` "since each
  advertised `outputSchema` describes successful results" (`docs/mcp-contracts.md:158-183`). OBSERVED.
- **Recovery hint semantics:**
  - `inspect_input_contract`: re-read the schema and "do not repeat the unchanged call". A well-formed but unanswerable
    request omits this hint.
  - `inspect_timeout_state`: names REA's own deadline. "Raising only the MCP client's deadline cannot extend this
    budget." Hints "never include the complete catalog or caller arguments" (`docs/mcp-contracts.md:161-178, 515-527`).

  OBSERVED.
- **Design rules for recovery** (`docs/tool-design.md:102-143`):
  - The same `invalid_request` can mean "violates input contract" or "valid selector the target can't resolve".
    Suggest schema inspection only for the first case. For the second, preserve the exact selector, the reason and the
    candidates. "An inventory miss does not prove that the artifact lacks the requested item."
  - Distinguish missing executable / wrong path / incompatible version / denied permission / timeout, instead of
    reporting "not installed".
  - Derive the required engine from the operation: Binwalk vs Unblob.
  - Say *where* a setting must change ("a shell export does not change an already running MCP server's environment").
  - "An error description is not a repair instruction."
  - Never switch target or provider automatically.
  - A suggested retry "must be safe".

  OBSERVED.
- **Recovery-text checklist** (`docs/design/limits-and-recovery.md:288-303`): limit/setting name; effective value and
  unit; source (`default|setting|runtime|request`); default, max and floor; next action and *where*; whether a restart
  is required; what the change does not raise. Fixed ceilings say "This release has no runtime override" and do not
  invent a setting (`:279-286`). An agent-action table maps each code to the correct next move
  (`:305-349`, e.g. `cleanup_incomplete` → retry that exact owner, never the operation). OBSERVED.
- **`docs/user-visible-errors.csv`:** a harm-ranked audit of 111 user-visible error messages. Columns:
  `harm_rank, location, trigger, current_copy, user_risk, proposed_replacement, implementation_status,
  verification_result`. Status counts: 80 implemented, 22 unchanged, 8 removed. Rank 1 is
  `AnalysisCapabilityUnavailableError` ("Exposes provider and internal operation/reason" → "This analysis is
  unavailable for the current target. Choose another analysis or target."). Verification results are mostly "focused
  assertions passed; full `npm run check` passed". Some rows say "real-provider verification required". OBSERVED. Its
  only automated consumer is `src/main/messages.test.ts:17-31`, which asserts that six canonical recovery constants
  appear in the CSV. INFERRED: it is a review artefact (a copy audit) rather than a generated contract.
- **Test rule:** "Assert error codes, structured limit facts, source provenance, and observed recovery behavior instead
  of freezing explanatory sentences". After replacing prose assertions, show that a harmless wording change passes and
  a wrong budget fails (`docs/testing.md:527-535`). OBSERVED.

### 3.10 Skills are subordinate; governance is fail-closed

- "Prioritize fixing the owning behavior, contract, and actionable result or recovery at the point of use … do not use
  a skill as the primary repair for a discoverability or contract defect" (`AGENTS.md:93-97`). The packaged skill tells
  agents to "Use the connected server's actual tool list and input schemas"
  (`.agents/skills/reverse-engineer-anything/SKILL.md:26`). OBSERVED.
- The four contributor skills each carry *workflow judgment, not authority*. Each says it grants no additional
  permission:
  - `rea-tool-design`: an audit/design/implement router to tool-design and testing (`.agents/skills/rea-tool-design/SKILL.md:8-34`).
  - `rea-ci-tests-audit`: pin baseline → prove failure/protection by perturbation → measure equivalent completed work →
    stop (`.agents/skills/rea-ci-tests-audit/SKILL.md:20-112`).
  - `rea-analysis-optimization`: independent oracle, earliest unnecessary work, cache key/owner/invalidation/lifetime,
    compare completed tasks (`.agents/skills/rea-analysis-optimization/SKILL.md:27-118`).
  - `rea-changelog-update`: inventory before prose, never fabricate credit, unknown handles stay unknown
    (`.agents/skills/rea-changelog-update/SKILL.md:13-46`).

  OBSERVED.
- Fail-closed integration: if scope, authorization, side effects or verification remain "materially unresolved, do not
  merge, enable, or include". "Record unknowns explicitly; do not turn them into 'none'". Passing CI is not scope
  approval (`CONTRIBUTING.md:183-199`). The PR template carries an "Expected effects" checkbox list that mirrors the
  vocabulary, and says "Unchecked items do not establish absence" (`.github/PULL_REQUEST_TEMPLATE.md:49-59`).
  AI agents must not tick human readiness declarations (`CONTRIBUTING.md:111-115`; `AGENTS.md:126-133`). OBSERVED.

---

## 4. How REA verifies tools

### 4.1 The ladder and the depth taxonomy

- Preference order: "real public CLI/MCP workflow, then production-boundary integration, then a producer golden".
  "Provider-behavior claims require the matching real-provider lane" (`docs/testing.md:420-424`). "Keep module tests for
  distinct failures or semantics that stronger workflows cannot reliably reproduce. Paths, compiled imports, and suite
  names do not establish test depth" (`:514-517`). OBSERVED.
- Claim→coverage table (`docs/testing.md:426-433`):

  | Claim | Required coverage |
  |---|---|
  | Source semantics | local tests |
  | CLI/MCP workflow | actual public calls ("an injected provider has limited scope") |
  | Docs/metadata | `docs:check` + `docs:build` |
  | Engine adapter | real-provider lane |
  | Client schema conversion | native client lane ("loopback models vs live services") |
  | Agent investigation quality | agent evaluation + transcript review |

  OBSERVED.
- Behavioural depths, each with an allowed scope (`docs/testing.md:851-868`):

  | Depth | Location | Scope |
  |---|---|---|
  | Module | `src/**/*.test.ts` | Narrow public surface; no sessions, servers, subprocesses or listeners |
  | Composition | `tests/composition` | Provider-neutral wiring; no production I/O boundary |
  | Boundary | `tests/boundary` | Exactly one production boundary |
  | MCP boundary | `tests/boundary/mcp` | — |
  | Acceptance | `tests/acceptance` | Compiled CLI/MCP; "injected providers still make it integration" |
  | Conformance | `tests/conformance` | Shared provider contracts parameterized by declared capabilities |
  | Evaluation | `tests/evaluation` | Deterministic evaluator logic |

  OBSERVED.

### 4.2 Static and structural gates

| Gate | What it enforces | Source |
|---|---|---|
| oxlint custom rules | `rea/no-positional-tool-contract` (catalog index never selects operation/schema; display-only allowed), `rea/no-evidence-json-preadmission`, `rea/cancellation-cause`, `rea/no-buffer-deep-equality`, `rea/no-fixture-root-cleanup`, `rea/no-shared-module-mock`. Each rule has a RuleTester suite under `tests/evaluation/*Rule.test.ts`. | `.oxlintrc.json:4-63`; `docs/testing.md:170-195`; `tests/evaluation/positionalToolContractRule.test.ts:1-30` |
| Typed lint | `no-floating-promises` is an error everywhere. `no-misused-promises` is an error in `src`. Exhaustiveness is an error when a union member has no case. | `docs/testing.md:206-214` |
| Architecture guards | Regex "use X instead" rules that name the canonical owner, e.g. "src/domain uses localeCompare (use compareUnicodeCodePoints)", "inline paired prlimit ceilings (use prlimitArgumentVector …)". | `scripts/verify/architecture-guards.mjs:63-160` |
| Module boundaries | Resolved import-direction rules, included in lint. | `CONTRIBUTING.md:24-25` |
| Knip | Every unused-file/dep/export rule set to `error`. `knip:production` reachability writes advisory candidates; execution/report errors fail. | `knip.json:30-41`; `knip.production.json`; `docs/testing.md:223-225` |
| jscpd | `threshold: 4` (%), `minLines: 5`, `minTokens: 50`, over production and test code. | `.jscpd.json:1-19` |
| Test discovery / temp hygiene | `verify:test-discovery` after adding or moving tests. `verify:test-temp-hygiene` runs under a fresh `TMPDIR` and rejects leftover REA paths. | `CONTRIBUTING.md:24, 414-416` |
| Generated-file freshness | Catalogs, the error schema and package metadata are regenerated with `--check`. A stale file fails. | `scripts/generate-error-schema.mjs:14-21`; `CONTRIBUTING.md:376-395` |

REA's own caveat: "Static checks locate candidates; they do not establish semantic ownership … Passing them does not
prove that two parsers agree, a wrapper is still needed at runtime, or cleanup has one owner" (`docs/testing.md:164-168`).
OBSERVED.

### 4.3 Contract and catalog conformance ("golden catalog")

- The catalog is generated **through the real MCP SDK**. All contracts are registered on an `McpServer` behind an
  `InMemoryTransport` and listed through a real `Client`. Every input and output schema is validated with `Ajv2020`.
  The script fails if the SDK omits a registered tool. It emits `{name, title, description, kind, requiresSession,
  inputSchema, outputSchema, annotations, effects}` (`scripts/generate-mcp-tool-catalog.mjs:1-80`). OBSERVED.
- **Golden checks over that catalog** (`tests/boundary/mcp/toolSchemaValidity.test.ts:559-1029`):
  - no recursive refs;
  - no local refs in inputs;
  - unresolved output refs rejected even for uncalled tools;
  - client array constraints;
  - object roots advertised while unions are enforced at runtime.

  OBSERVED.
- **Metamorphic order test:** a fixture registers contracts in *reversed* order. The test then asserts identical
  advertised input and output schemas per name, and that each handler still answers for its own operation
  (`tests/boundary/mcp/contractPresentation.test.ts:28-62`). This proves "catalog ordering must never select an
  operation or schema" (`AGENTS.md:39`). OBSERVED.
- **Profile tests:**
  - Every compact schema is pinned under the budget and must still accept every canonical example.
  - Compact workflow tests check canonical rejection before provider work.
  - Full-profile tests keep nested restrictions (`docs/mcp-contracts.md:1839-1847`; `tests/conformance/contracts/mcpInputSchemaProfile.test.ts`).

  OBSERVED.
- **Client-boundary fault injection:** the regression changes only `canonical_path` in a real stdio response to a
  number. The old helper accepted that; the named schema must reject it. Artifact verifiers must discover advertised
  definitions themselves and refuse caller-supplied definitions, "so a stale or permissive schema cannot bypass it"
  (`docs/testing.md:652-666`). OBSERVED.
- "Meta-schema validity does not establish client acceptance or useful rendered declarations"
  (`docs/testing.md:275-279`). OBSERVED.

### 4.4 Semantic test design rules (the anti-false-pass discipline)

From `docs/testing.md:519-580` and the CI audit skill. All OBSERVED.

- Prove a suspicious green test: "perturb the relevant behavior in an isolated copy or inject a controlled failure.
  Verify that the existing assertion or gate detects it" (`.agents/skills/rea-ci-tests-audit/SKILL.md:53-56`;
  `docs/testing.md:544-546`).
- "A negative probe inside a conditional or loop must assert that it ran." Watch for `every` on an empty list,
  optional chaining, and comparing two missing values (`docs/testing.md:541-544`).
- "Comparing two paths through the same replacement algorithm does not preserve the old oracle" (`:539-541`). For
  optimization: "Comparing two wrappers around the same faulty decoder does not establish correctness"
  (`rea-analysis-optimization/SKILL.md:35-39`).
- Pair an accepted boundary case with every rejected case "so a stricter parser cannot pass by refusing both"
  (`docs/testing.md:573-575`).
- "Protocol fixtures must reject unmodeled commands and preserve actual producer reply shapes" (`:558-559`).
- Representative scenarios, not Cartesian matrices (`:555`). Use a boundary-regression table by failure class:
  - format assumptions;
  - raw identity/normalization;
  - missing/empty/unknown;
  - completed-then-failed;
  - independent limits;
  - representation expansion;
  - snapshots/caches;
  - public success/error delivery (`:590-599`).
- A lifecycle-regression table covers:
  - pending startup;
  - late/invalid handshakes;
  - launch error with a valid handle;
  - cancellation during final cleanup;
  - mixed child outcomes;
  - cleanup failure and retry;
  - lost transport (`docs/testing.md:705-845`).
- "A missing engine is not a pass" (`docs/testing.md:845-847`). Unavailable authority is "a named skip" (`:668`).

### 4.5 Coverage thresholds

Coverage thresholds are glob-specific and "never updated automatically" (`docs/testing.md:933-935`):

| Scope | Statements / branches / functions / lines |
|---|---|
| Global | 65 / 60 / 60 / 68 |
| `src/domain/**` | 80 / 75 / 75 / 80 |
| `src/contracts/**` | 85 / 80 / 80 / 85 |

Shards run with zero thresholds and are enforced on the aggregate. `retry: 0`. A custom `processOwnershipRunner`
retires native process-ownership hosts when each worker stops (`vitest.config.ts:98-178, 142-144`;
`tests/processOwnershipRunner.ts:6-18`). OBSERVED. "Coverage does not replace named boundary or real-provider
scenarios" (`docs/testing.md:935`). OBSERVED.

### 4.6 Mutation testing

- **Scope:** reverse-tools lifecycle code, including:
  - `ReverseEngineeringService` observe/cleanup guards;
  - `ExecFileOutput`;
  - `providerEnvironment`;
  - the GDB session manager's constructor and protocol start;
  - Rizin debug session deadlines and timeout Evidence;
  - Cutter bridge identity;
  - `ownedDebuggerSession`;
  - `analysisResourceLimits`;
  - reverse-engineering CLI commands.

  Sources: `stryker.config.mjs:31-239`; `stryker.semantics.config.mjs:27-53`; `docs/testing.md:937-953`. OBSERVED.
- **Selector design:** mutation regions are named AST selectors, not line ranges. Selector kinds are `method`,
  `constructor`, `guard` (with `condition` text), `declaration`, `try` (with `assignment`/`callee`), `call` (with
  `callee`/`event`), `expression` (exact text) and `command-handler`. Owner-requiring kinds must name a class. A
  malformed source, an unknown kind, or a selector matching ≠1 node throws *before* mutation generation
  (`scripts/lib/reverse-tools-mutation-targets.mjs:139-181`). OBSERVED. INFERRED: line ranges are produced when the
  config loads, so refactors cannot silently move mutation scope.
- **Two passes:**
  1. Vitest runner, `related: true`, `coverageAnalysis: "perTest"`, TypeScript checker (`stryker.config.mjs:3-27`).
  2. Command runner over a 4-file semantic suite with `coverageAnalysis: "off"`, so every compilable mutant runs
     regardless of per-test coverage association (`stryker.semantics.config.mjs:3-23`; `vitest.reverse-tools-semantics.config.ts`).

  Compile-error mutants are reported separately (`docs/testing.md:943-946`). OBSERVED.
- **Gating:** both configs set `thresholds: { break: null }`, so the score is non-blocking. Required CI runs
  `--dryRunOnly` for both, which resolves targets, instruments and runs the initial tests, but executes no mutants
  (`.github/workflows/ci.yml:392-393`; `docs/testing.md:955-962`). A separate **Mutation sweep** runs daily at 04:47
  UTC and on dispatch. It succeeds only with both jobs and non-empty reports (`.github/workflows/mutation.yml:5, 24-62`;
  `docs/testing.md:964-980`). OBSERVED.

### 4.7 Real-toolchain ("live") lanes

- There are 114 `verify:*` npm scripts (`package.json`) and 19 `real-*.yml` workflows (`.github/workflows/`). OBSERVED.
- Lane rules (`docs/testing-lanes.md:18-23`):
  - require only the host tools needed for the claim;
  - use host-native fixtures;
  - put cross-target formats in separate commands;
  - check prerequisites before expensive work;
  - name the missing command, target and lane;
  - never imply coverage when skipped.

  OBSERVED.
- **What a strong lane asserts.** The host Ghidra lane (`docs/testing-lanes.md:71-140`) runs through the production CLI
  *and* a separate stdio MCP process with "No provider or transport … mocked". It checks:
  - every advertised schema and every exercised output;
  - address-spelling ambiguity;
  - CLI/MCP parity;
  - atomic annotation rollback;
  - unchanged executable bytes;
  - discarded edits after reopen;
  - cancellation after a real headless launch;
  - an adversarial 12 KB regex reported as a resource constraint with the session still usable;
  - `artifact_changed` when the source changes between open and first query;
  - real permission denial via `requirePermissionDenial`, which skips explicitly when the host bypasses DAC.

  OBSERVED.
- **Oracles are independent.** Examples: compiler-produced fixtures with known source facts, ELF bytes and table
  slots for switch recovery, an oracle that is itself fault-injected to prove it rejects wrong answers
  (`docs/testing-lanes.md:142-156`), and source-built conformance fixtures with hash and toolchain manifests
  (`tests/conformance/README.md:1-30`). OBSERVED.
- **A required-case manifest prevents skip-as-pass.** `scripts/ci/require-provider-tests.mjs:1-60` maps a lane to
  exact test-file paths and exact case titles. It fails unless the Vitest JSON report is successful and contains each
  named case. INFERRED from the visible portion: each case must have passed and not been skipped.
- **Client lanes** (`docs/client-verification.md:46-275`) cover Qwen, Pi, Hermes, Gemini, OpenCode, Claude Code,
  Copilot, Grok, Command Code, OMP and DeepSeek. Each lane:
  - uses disposable homes;
  - runs public `rea setup` with backups and idempotence checks;
  - verifies native skill discovery;
  - validates all forwarded input schemas;
  - checks full named-schema Evidence for a Unicode path.

  Every receipt states that the loopback model "do[es] not prove live model-provider compatibility". OBSERVED.

### 4.8 Agent-experience evaluation

- `npm run verify:agent` runs scenarios sequentially through a real local Codex CLI. Isolation is strict
  (`docs/client-verification.md:489-521`):
  - disposable `HOME`/`CODEX_HOME` outside `/tmp`;
  - skill and MCP registration installed via public `rea setup`;
  - only auth copied, then deleted after confirmed process cleanup;
  - no user config, rules, plugins or history;
  - per-scenario preapproval of only the needed tools (e.g. `open_binary`, `close_binary`).

  OBSERVED.
- **Scenarios are data, and each prompt is owned together with its grammar.** The negative routing scenario
  (`missing-target`) must make zero REA calls and return a strict JSON clarification. The native scenario accepts
  exactly one `provider_unavailable` open refusal *or* open-then-close. Any other error, an unclosed session, or a close
  without acquisition fails (`tests/evaluation/model-evals/AgentEvaluationScenarios.ts:11-65`). OBSERVED.
- **Gate formula** (`AgentEvaluationReport.ts:24-42`), quoted in §1.9. `inputValidationFailureCount` counts
  `invalid_request` responses *and* failed calls whose arguments do not parse under the current canonical input
  schema (`CodexAgentEval.ts:183-190`). Client approval rejections and provider unavailability stay distinct
  (`docs/client-verification.md:582-585`). OBSERVED.
- **Known-answer rubrics:**
  - The final answer must be a strict JSON claim manifest with claim IDs, values, Evidence IDs, authority and
    confidence. Golden values are not in the prompt.
  - Values are checked against both the fixture oracle and authenticated REA results. "An Evidence ID by itself is
    insufficient" (`docs/client-verification.md:541-556`).
  - Failure modes form a closed enum: `claim_missing`, `value_mismatch`, `unknown_evidence`, `conflicting_evidence`,
    `source_subject_mismatch`, `selector_failed`, and others (`tests/evaluation/model-evals/KnownAnswerEvaluation.ts:44-59`).

  OBSERVED.
- **Honest labelling:** heuristic metrics "can still accept fabricated prose and must not be read as factual
  assessments". Rubric-less scenarios are `not_assessed` (`docs/client-verification.md:568-594`). The report schema is
  versioned (v4 adds nullable `expectedFirstTool`) (`:596-610`). OBSERVED.

### 4.9 Verification records

The record pattern (`docs/verification/*.md`; 53 files):
- An issue link.
- Pinned baseline and final SHAs; a runtime-manifest SHA when relevant.
- **Executed scope** and **explicit non-claims**: e.g. "This validates projection; it does not independently prove that
  every producer inference is correct" (`docs/verification/web-bundle-paging-2026-10-26.md:11-15`).
- Real third-party inputs pinned to a commit.
- Regressions shown to *fail against the old code*.
- Before/after measurements with their measurement method (cgroup peak, CPU, elapsed, tokenizer).
- A runnable reproduction.
- A JSON companion with per-file source SHA-256 (`docs/verification/javascript-analysis-effects-2026-10-15.json`).
- Negative controls: "The old manual provider-close list was temporarily restored as a negative …"
  (`docs/verification/capability-composition-2026-11-01.md:28`).

OBSERVED. INFERRED: these records are the human-readable proof layer that sits between CI (deterministic) and release
notes.

### 4.10 Orchestration invariants

- Turbo caches deterministic build and static tasks, but `test:run` is `cache: false` (`turbo.json:134-135`). "A
  previous test pass therefore cannot replace current validation" (`docs/testing.md:313-318`). OBSERVED.
- No test retries (`vitest.config.ts:142`; `docs/testing.md:985`). "Failed, cancelled, or missing selected shards still
  fail the required gate", and "a missing explicit test or missing required report must not turn green"
  (`docs/testing.md:994-995`; `rea-ci-tests-audit/SKILL.md:34-35`). OBSERVED.
- Static checks, docs, build, package and real-system lanes "remain separate jobs so one kind of evidence cannot stand
  in for another" (`docs/testing.md:1011-1013`). OBSERVED.

---

## 5. Tool design rubric (adoptable, machine-checkable)

Severity key: **E** = error (blocks generation or publish), **W** = warning, **R** = review-only. "Check" says how
agentic-anything's adapter generator or linter can enforce each rule automatically. INFERRED design, derived from the
REA rules cited.

| # | Rule | REA basis | Automated check for generated adapters | Sev |
|---|---|---|---|---|
| R1 | One contract per tool: name, title (derived), description, input schema, output schema, effects, untrusted flag, ≥1 example. Handlers bind by name. | `toolContractTypes.ts:36-66`; `AGENTS.md:39` | Schema-validate the adapter manifest. Register in reversed and shuffled order and diff the advertised catalog and handler dispatch (metamorphic test as in `contractPresentation.test.ts`). | E |
| R2 | Name = `verb_object` snake_case. The verb comes from a closed shape vocabulary (`inspect, list, search, trace, compare, observe, capture`, plus a declared action-verb set). The name is never a mode switch. | `tool-design.md:16-27` | Regex plus a verb allowlist. Fail if the input schema has a `mode`/`action` enum that crosses effect classes (e.g. a branch with `readOnly` and another with `mutatesTarget`). | E |
| R3 | No mixed authority: all input branches of one tool have the same effect set. | `tool-design.md:7-9, 25-27` | Compute effects per input branch (adapter declares per-branch effects). Error if they differ. | E |
| R4 | Effects are declared from a closed vocabulary. Unknown means the most conservative value. A missing declaration fails the build. | `toolEffects.ts:1-30, 818-822` | Generator defaults every effect of an *auto-discovered* action to `true` (except `idempotent=false`) until a human audit narrows it. | E |
| R5 | Description wording agrees with effects. | `toolEffects.test.ts:16-170` | Verb-wording regex (run/launch/execute/send/delete/install/click/type/submit…) requires matching effects, or a reasoned exception. Stale exceptions fail. | E |
| R6 | Annotations are derived, never hand-set. Read-only with respect to the *target* is separate from session writes. | `toolEffects.ts:764-786`; computed 9/234 readOnly | Derive `readOnlyHint` from target-facing effects only. Expose `mutatesSession` as a separate `_meta` field. Freeze a golden list of destructive tools per adapter. | E |
| R7 | Untrusted by default. Only an audited allowlist may omit `_meta.provenance.untrusted_content`. | `toolEffects.ts:788-816`; `targetContentProvenance.test.ts` | Any tool returning target-derived strings (UI text, titles, clipboard, logs) is marked. The allowlist is checked against docs. | E |
| R8 | Strict inputs: `additionalProperties:false`, explicit target identity field, no ignored options, no permission booleans. | `tool-design.md:147-149` | Schema lint: root object is strict, has a required `target` or session reference, and no `confirm`/`force`/`allow_*` field unless an effect requires it. | E |
| R9 | Advertised schema ≤ budget (default 13,312 B per input schema; depth ≤10). There is a compact projection, and server-side validation always uses the canonical schema. | `mcp-contracts.md:1761-1847` | Measure the serialized size after SDK conversion for each profile. Validate that every canonical example passes the compact projection. Strip root `anyOf/oneOf/allOf`. Forbid `dependentRequired`. Arrays with `items` must have a single type. | E |
| R10 | Examples are valid canonical inputs, distinct, and never embed large records (references only). | `toolContractHelpers.ts:22-33`; `mcp-contracts.md:1716-1721` | Parse every example at build time. Size cap per example. | E |
| R11 | Results answer inline, with identity, locations, references, coverage `{scope,status}`, limitations and confidence. Missing coverage means `unknown`. | `tool-design.md:155-160, 489-496`; `mcp-contracts.md:89-118` | The output schema must contain `coverage.status ∈ {complete, unknown, partial}`. A property test checks that `status=complete` never co-occurs with a non-empty "omitted" limitation. | E |
| R12 | Large results are summary-first with a retained parent ID plus one generic pager. Paging carries upstream coverage, and the cursor binds to target+query. | `mcp-contracts.md:46-58, 247-252, 340-358` | Any list/tree producer whose p95 output exceeds X KB must declare `detail` and `parent_id`. Cursor-misuse test: reusing a cursor with a different query must return `invalid_request`. | W→E |
| R13 | Oversized responses become `resource_constraint` with a retrievable reference, decided before serialization. | `mcp-contracts.md:227-239` | Fuzz with a giant fixture. Assert a bounded error plus a reference, and that the connection survives. | E |
| R14 | Limits exist only for real constraints. Each limit has a name, value, unit, source and bounds, and says what does not raise it. | `tool-design.md:174-178`; `limits-and-recovery.md:279-298` | Every limit constant is registered in a limits registry. Error text is generated from the registry, never hand-written. | E |
| R15 | Errors use one closed envelope `{code, category, message, retryable, remediation.action, recovery?, details?}`, with a generated JSON Schema. | `errorSchemas.ts:19-132` | Validate every error emitted in tests against the schema. Freshness-check the schema. | E |
| R16 | Malformed input and an unresolvable valid selector are different errors. Only the former gets `inspect_input_contract`. A miss is never reported as absence. | `tool-design.md:104-110`; `mcp-contracts.md:161-166` | Paired test per tool: malformed input → hint present; valid-but-missing selector → hint absent, selector echoed, candidates listed. | E |
| R17 | Recovery is actionable and context-preserving. It never switches target or provider automatically, and a suggested retry must be safe. | `tool-design.md:124-138` | Lint `remediation.action` for an imperative verb and the setting location. Execute the follow-the-advice test (§6, L4). | W |
| R18 | User-visible copy is audited by harm rank. Internal names and paths are not leaked unless useful. | `docs/user-visible-errors.csv` | Generate a CSV catalog of all message templates. Diff it per release and require review of new rows. | R |
| R19 | Workflows and batches exist only with demonstrated reuse; per-item outcomes are preserved. | `tool-design.md:41-44` | A batch tool's output schema must be `items[]` with a per-item `status`/`error`. | E |
| R20 | Guidance (instructions, prompts) never names a disabled tool. A skill is not a fix for a contract defect. | `serverInstructions.ts:8-44`; `AGENTS.md:93-97` | Render instructions and prompts per family scope. Assert every referenced tool name is advertised. | E |
| R21 | Family scoping: every tool belongs to exactly one family. A disabled tool returns a named error, not "unknown tool". | `mcp-contracts.md:1849-1889` | Catalog test per family subset. | E |
| R22 | Description budget. | No REA check found (gap) | W at >1,000 chars, E at >2,500 chars. Require first sentence = outcome + object. | W |

---

## 6. Verification ladder for generated adapters

Each level states what it proves and what it does **not** prove, following REA's labelling discipline
(`docs/testing.md:465-469`; `docs/client-verification.md:568-594`). INFERRED design.

| Level | Name | What runs | Proves | Does not prove | Gate |
|---|---|---|---|---|---|
| L0 | Manifest and static | Manifest schema, rubric R1–R22 lint, effects-wording lint, untrusted allowlist, jscpd/knip-style dead-code checks on generated code | The surface is well-formed and honestly labelled | Any runtime behaviour | Required, every build |
| L1 | Catalog golden | Register through the real MCP SDK; Ajv2020-validate every advertised schema per profile; size/depth budgets; examples parse; reversed-order metamorphic diff; family-subset catalogs; generated error schema freshness | Clients receive valid, bounded, order-independent declarations | That a specific client or model accepts them | Required |
| L2 | Contract semantics (fake backend) | Recording or scripted backend; per tool: success, malformed input, valid-but-missing selector, unavailable, cancellation, partial, oversize; error-envelope validation; paging-coverage propagation; cursor misuse | Adapter logic and error and recovery shapes | Real target behaviour ("simulated providers do not establish real-provider behavior") | Required |
| L3 | Mutation | AST-selector mutation over lifecycle, cleanup, cancellation and effect-gating code; selector must match exactly 1 node; dry-run preflight in required CI; full sweep nightly | Tests actually detect faults in the risky code | Coverage of unselected code | Preflight required; score tracked (consider a floor for gating code) |
| L4 | Live target lane | A real target app (owned fixture build) per adapter, through public CLI *and* MCP; independent oracle (app's own API/state file/a11y dump read through a separate channel); before/after state; cleanup verified; follow the remediation and re-check; CLI/MCP parity | The adapter works on that host/version/target | Other hosts, versions, or agent discoverability | Required for the adapter's claimed host matrix; named skip ≠ pass; required-case manifest |
| L5 | Client lane | Install via public setup into a disposable client home; schema forwarding check; one call with full result readback; loopback model | Client loads, forwards and executes | Live model behaviour | Optional per client; receipts record versions |
| L6 | Agent evaluation | Real agent, disposable home, data-defined scenarios incl. a negative routing case; gate on first tool, zero repeats (keyed by target state revision), zero input-validation failures, required subsequence, cleanup outcome, tokens | Discoverability and efficient use | Unrestricted correctness | Release gate for adapters marked "agent-ready" |
| L7 | Known-answer / state-oracle rubric | Strict JSON claim manifest bound to result IDs; or, for drive tools, resulting application state read through an independent channel with state reset per run; fabricated, extra and unrelated claims fail | Configured facts or outcomes are correct and grounded | Facts outside the rubric | Required for "verified" badge |
| L8 | Verification record | Dated record: SHAs, executed scope, non-claims, negative control, reproduction, JSON companion with source hashes | Auditable claim history | Anything after the pinned revision | Per material change |

Cross-cutting rules, all from REA:
- Live tests are never cached.
- No retries.
- A missing report or a skipped required lane fails.
- Real-target claims require L4.
- Agent claims require L6 or L7.
- Each level's receipt states what it did not cover.

---

## 7. What to distill for agentic-anything

1. **Adopt the single-contract-per-tool record and generate everything from it** (catalog, docs, skill metadata, CLI
   help, error schema). Make the adapter generator emit a `ToolContract`-equivalent YAML/TS manifest. CI should
   regenerate it with `--check` (REA: `scripts/generate-mcp-tool-catalog.mjs`, `scripts/generate-error-schema.mjs`).
2. **Ship the rubric in §5 as an executable linter** (`aa lint adapter/`), with R1–R16, R19–R21 as errors.
   Auto-discovered adapters (from a11y trees, CLI help, OpenAPI, AppleScript dictionaries) must pass it before they can
   be served.
3. **Conservative default effects for discovered actions.** Any action the discovery step cannot classify gets all
   effect flags true and `idempotent:false`, and is served only after either:
   - a human audit narrows the effects, or
   - an REA agent-environments-style read-back confirms the effect (see salvaged summary, "write gating").

   Port REA's wording-vs-effects test directly. For GUI verbs, extend the regex with
   `click|type|press|submit|send|delete|save|close|quit`.
4. **Split target effects from session effects in annotations.** REA's own numbers (9/234 read-only; 109 non-read-only
   only because of Evidence recording) show the cost of a merged flag. Derive `readOnlyHint` from target-facing effects
   only, and expose `mutatesSession` separately in `_meta`, so host approval policies do not prompt on every
   observation.
5. **Budget the catalog from day one.**
   - Use a compact input-schema profile (start at 13,312 B / depth 10, measured per target model), with canonical
     validation on the server.
   - Strip root unions and unsupported keywords for Gemini and strict gateways.
   - Use family scoping with named "disabled" errors.
   - Track a per-task token count (schemas, calls, retries) in L6, not bytes.
6. **Make large observations summary-first with one generic pager.** UI trees, window lists and logs return a
   summary, a `parent_id` and coverage. A single `inspect_view` tool pages any retained record. Cursors bind to target,
   query and state revision. Oversize results become `resource_constraint` plus a reference, decided before
   serialization.
7. **Copy the error envelope and its recovery semantics verbatim:**
   - 3 recovery kinds, `remediation.action` required, `retryable` explicit;
   - malformed vs unresolvable distinction;
   - recovery text generated from a limits registry.

   Add drive-specific codes (`target_state_changed`, `action_unconfirmed`, `confirmation_required`, `policy_denied`)
   and drive-specific recovery kinds (`reobserve_target`, `request_confirmation`).
8. **Keep a harm-ranked user-visible message catalog** (like `docs/user-visible-errors.csv`), generated from message
   templates. Changes are reviewed per release.
9. **Implement the verification ladder in §6 as named CI tiers.** Use a required-case manifest
   (`require-provider-tests.mjs` pattern) so live lanes cannot pass by skipping. Keep `test:run` uncached, use zero
   retries, and fail on missing reports.
10. **Use AST-selector mutation testing for the safety-critical core** (effect gating, confirmation, kill switch,
    cleanup ownership, target-identity re-verification before each action). Fail the selector resolution on ≠1 match.
    Unlike REA, consider a *blocking* score floor for the authorization and confirmation modules, because those are
    the drive-layer equivalent of REA's cleanup code.
11. **Build the agent-eval harness early, with the oracle changed to application state.** Reuse REA's gate shape:
    - a disposable client home;
    - install via public setup;
    - preapprove only scenario tools;
    - a negative routing scenario;
    - repeats keyed by target state revision;
    - validation failures counted against the canonical schema;
    - known-answer manifests bound to action-record IDs.

    For drive tools, the oracle is resulting state read via an independent channel, with reset per run.
12. **Label every claim's scope.** Each receipt and verification record must say what it does not establish (loopback
    model, fake backend, single host). Enforce this with a required `non_claims` field in the record schema.
13. **Govern with fail-closed integration plus a PR-template effects checklist,** extended for drive authority
    (observe/launch/attach/inject/mutate). Unchecked never means absent. AI agents never tick human readiness.
14. **Add what REA lacks:**
    - A description-length budget (R22).
    - A runtime effect monitor in L4 that records process spawns, file writes outside a declared root, network
      connects and UI focus changes, and diffs them against declared effects. REA relies on review plus issue-driven
      regressions (`docs/verification/javascript-analysis-effects-2026-10-15.md`).
    - Typed `details` per error code, not only for integrity mismatches.

---

## 8. Open questions

- `docs/mcp-contracts.md:1852` still cites "the Claude Code measurements above" for per-request catalog payload cost.
  No such measurement is in the file. Where is it (an issue, a verification record), and what did it measure? UNKNOWN.
- `scripts/ci/require-provider-tests.mjs`: I read lines 1-60. Does it reject `skipped`/`pending` status explicitly, and
  is the pattern used beyond the packet and signal lanes? UNKNOWN.
- Is there any plan to gate on mutation score, or to extend mutation targets beyond reverse-tools lifecycle code (e.g.
  to the error projector or the effects derivation)? Only the non-blocking policy was observed.
- `category` is unconstrained per `code` in the error schema (any of 10 categories with any of 20 codes). Is the
  code→category mapping fixed in the projector (`src/domain/analysisErrorProjection.ts`), and is it tested? Partly
  covered in `rea-contracts-and-evidence-model.md`; I did not re-verify it.
- What is the actual token cost of the full versus compact catalog on Claude Code and Codex with REA's 234 tools? The
  only recorded number is a `cl100k_base` static count (204,093 vs 16,548) for one task.
- Has `verify:agent` ever been run against a non-Codex agent? The docs mention only Codex (`docs/client-verification.md:477-489`).
- Would a description-length budget have caught any REA discoverability regressions? There is no data on the
  description length versus tool-selection relationship.
- For agentic-anything: what is the right compact-schema budget per target model family, given that REA's 13,312 B is
  calibrated to Moonshot's cap and not to selection quality?

---

## Sources

All paths are under `/Users/nhanclie/rea` at `3484432fb18482dcced81272e6181719a3cdfbfa` (read only).

- **Guidelines and docs:**
  - Guidelines: `AGENTS.md`, `CONTRIBUTING.md`, `.github/PULL_REQUEST_TEMPLATE.md`.
  - Design and contract docs: `docs/tool-design.md` (1-537), `docs/mcp-contracts.md` (14-260, 515-575, 1695-1962),
    `docs/design/limits-and-recovery.md` (1-60, 269-350).
  - Testing docs: `docs/testing.md` (1-700, 845-1166), `docs/testing-lanes.md` (1-176, 1226-1282),
    `docs/client-verification.md` (1-280, 475-621).
  - Error artefacts: `docs/user-visible-errors.csv` (parsed), `docs/error-contract.schema.json` (parsed).
  - Verification records: `docs/verification/` (listing),
    `docs/verification/javascript-analysis-effects-2026-10-15.{md,json}`,
    `docs/verification/web-bundle-paging-2026-10-26.md`, `docs/verification/investigation-lifecycle.md`,
    `docs/verification/capability-composition-2026-11-01.md`.
- **Skills:** `.agents/skills/rea-tool-design/SKILL.md`, `.agents/skills/rea-ci-tests-audit/SKILL.md` and
  `references/measurement-and-ci.md`, `.agents/skills/rea-analysis-optimization/SKILL.md`,
  `.agents/skills/rea-changelog-update/SKILL.md`, `.agents/skills/reverse-engineer-anything/SKILL.md` (lines 1-30).
- **Source:**
  - `src/contracts/toolContractTypes.ts`, `src/contracts/toolEffects.ts` (plus a computed annotation histogram from a
    scratchpad copy), `src/contracts/toolEffects.test.ts`, `src/contracts/toolContracts.ts`,
    `src/contracts/sessionToolContracts.ts`, `src/contracts/toolContractHelpers.ts`, `src/contracts/errorSchemas.ts`.
  - `src/server/serverInstructions.ts`, `src/main/messages.test.ts`.
- **Tests:** `tests/conformance/contracts/targetContentProvenance.test.ts`,
  `tests/conformance/contracts/mcpInputSchemaProfile.test.ts`, `tests/boundary/mcp/contractPresentation.test.ts`,
  `tests/boundary/mcp/toolSchemaValidity.test.ts` (test index), `tests/evaluation/positionalToolContractRule.test.ts`,
  `tests/evaluation/model-evals/{AgentEvaluationReport,AgentEvaluationScenarios,CodexAgentEval,KnownAnswerEvaluation}.ts`,
  `tests/processOwnershipRunner.ts`, `tests/conformance/README.md`.
- **Scripts:** `scripts/generate-mcp-tool-catalog.mjs`, `scripts/generate-error-schema.mjs`,
  `scripts/lib/reverse-tools-mutation-targets.mjs`, `scripts/lib/agent-evaluation-report-gate.mjs`,
  `scripts/ci/require-provider-tests.mjs` (1-60), `scripts/verify/architecture-guards.mjs`.
- **Configs:** `stryker.config.mjs`, `stryker.semantics.config.mjs`, `vitest.config.ts`,
  `vitest.reverse-tools.config.ts`, `vitest.reverse-tools-semantics.config.ts`, `.jscpd.json`, `knip.json`,
  `knip.production.json`, `turbo.json`, `.oxlintrc.json` (rule list), `package.json` (scripts).
- **CI:** `.github/workflows/` (listing), `.github/workflows/mutation.yml`, `.github/workflows/ci.yml:392-393`.
- **Prior agentic-anything material:** `docs/research/salvaged/tool-design-philosophy-and-verification.json`,
  `docs/research/rea-contracts-and-evidence-model.md`, `docs/research/rea-mcp-server-adapter.md`, `PLAN.md`.
