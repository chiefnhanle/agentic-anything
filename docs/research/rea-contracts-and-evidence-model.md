# REA research: contracts and Evidence model

Area: `contracts-and-evidence-model`
Source checkout: `/tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src` (third-party repo `morluto/rea`, read only; nothing was built or run).
All file refs below are relative to that checkout. "OBSERVED" = read directly in source/docs. "INFER" = my reading between the lines. Unknowns are marked.

---

## Summary

- Every evidence-producing tool returns one strict, provider-neutral **Evidence record** (`src/domain/evidence.ts:87-110`) whose `evidence_id` is `ev_` + SHA-256 over an RFC 8785 canonical projection of the record's *semantic* content (`evidenceSemanticContent`, `evidence.ts:232-254`). Caller display metadata (`subject.local_path`, `subject.name`, `caller_input_path`) is deliberately excluded from identity, so path aliases do not fork identity.
- Epistemic status is a first-class, closed vocabulary on every record: `confidence: observed | derived | inferred` and `authority: shipped-artifact | controlled-replay | historical-reference | external-service | analyst-inference` (`evidence.ts:72-78, 102-103`). "Unknowns" are not a confidence level; they are a separate, versioned **residual-unknown ledger** (`src/domain/residualUnknown.ts`) with its own content-addressed `unk_` IDs and revision chain.
- Projections never upgrade certainty: `inspect_analysis_view` derives `confidence` as `inferred` if the parent is inferred (or if the view itself infers), else `derived`; `authority`/`environment` are copied from the parent (`src/application/analysisView/AnalysisViewService.ts:420-433`).
- Tool contracts are the single source of truth (`ToolContract` in `src/contracts/toolContractTypes.ts:37-66`): zod `inputSchema` + `outputSchema`, a hand-audited `effects` record (`src/contracts/toolEffects.ts:2-22`), MCP annotations *derived* from effects rather than from tool names (`toolEffects.ts:765-787`), and a `returnsTargetContent` flag that drives an `_meta.provenance.untrusted_content` marker on results and errors.
- Errors are a tagged algebra (`AnalysisError` subclasses with `_tag`, `src/domain/analysisErrorBase.ts:7-35`) projected once into a stable discriminated union (`src/contracts/errorSchemas.ts:41-101`) with `code`, `category`, `message`, `retryable`, `remediation.action`, optional bounded `recovery` next-step and `details` (partial observation, cleanup state, captured output). `docs/error-contract.schema.json` is the generated JSON Schema of that union; `docs/user-visible-errors.csv` is a 125-row harm-ranked audit of every user-facing error string.
- Expected failures are modelled as `Result<T,E>` (`src/domain/result.ts`, 10 lines) through domain/application layers; adapters convert to the projection at the edge. Partial facts survive failure: `details.partial_observation`, `details.cleanup`, `details.captured_output`.
- Transport-size pressure is solved by **retain-then-view**, not truncation: `detail: "summary"` retains the complete record and returns a derived summary whose `normalized_result.parent_evidence_id` points back; `inspect_analysis_view` pages it; `get_evidence_bundle` offers `complete | summary | records | links`; `export_evidence_bundle` streams a canonical bundle atomically to disk (`docs/mcp-contracts.md:14-58, 332-363, 528-549`).
- Redaction is rule-based and narrow by policy (AGENTS.md): remove transport credentials (URL userinfo, `authorization`/`proxy-authorization`/`cookie`/`set-cookie` headers) and caller-declared literal `sensitive_values`; never infer secrecy from names (`src/browser/history/CaptureRedaction.ts`, `src/domain/explicitSensitiveValues.ts`). Redactions are recorded as `{pointer, reason}` rows inside the Evidence so consumers know what was withheld.
- The reconstruction obligation ledger (`docs/reconstruction-obligation-ledgers.md`) separates **authenticity** (digest verifies), **authority** (right kind/strength of observation), and **applicability** (proof bound to exact owner digest + obligation revision). This three-way split is the most directly reusable idea for an action-driving control plane.
- The closest REA analogue to an "action record" already exists: `capture_process_scenario` / `capture_browser_scenario` Evidence with `authority: controlled-replay`, which commits the scenario, records before/after filesystem checkpoints, an interleaved event journal, exit/settlement, per-scope residual unknowns, and an explicit cleanup report (`src/domain/process/processCapture.ts:216-286`).

---

## How it works

### 1. Contracts (`src/contracts/`)

**Contract shape.** OBSERVED `ToolContract<Name, InputSchema, OutputSchema>` (`src/contracts/toolContractTypes.ts:37-66`):
- `name`, `title`, `description`, `kind: ToolKind` (adapter family; closed list at lines 13-31: `official-proxy`, `enhanced`, `native-provider`, `artifact-provider`, ..., `application`, `session`).
- `requiresActiveTarget?` and `retainedSummary?: { evidenceParameters: "omit-detail" | "preserve-detail" }` (whether the `detail` arg is stripped from Evidence `parameters`, so delivery choice never changes Evidence identity).
- `inputSchema`, `outputSchema` (zod).
- `effects: ToolEffects`, `annotations: DerivedToolAnnotations`, `returnsTargetContent: boolean`, `examples`.

**Catalog assembly.** OBSERVED `TOOL_CONTRACTS` is a flat ordered spread of ~45 per-domain arrays (`src/contracts/toolContracts.ts:48-94`). `toolContract(name)` resolves by name with exact types (`:97-112`). `TOOL_CONTRACT_GROUPS` (`src/contracts/toolFamilies.ts:52-100`) is the same arrays keyed by constant name; `TOOL_FAMILY_DEFINITIONS` (`:116-245`) maps 17 selectable family ids (`direct`, `enhanced`, `native`, `artifact`, `managed`, `firmware`, `android`, `pcap`, `flutter`, `frida`, `apktool`, `jeb`, `adb`, `browser`, `electron`, `javascript-runtime`, `application`, `session`) onto groups; every contract belongs to exactly one family. `REA_MCP_TOOL_FAMILIES` narrows what `tools/list` advertises; out-of-scope tools stay registered but disabled (`docs/mcp-contracts.md:1743-1811`).

**Effects audit.** OBSERVED `ToolEffects` has 10 booleans (`toolEffects.ts:2-22`): `mutatesTarget`, `mutatesSession` (includes additive Evidence recording), `writesFilesystem`, `launchesProcess`, `accessesNetwork`, `changesUiState`, `mayDiscardData`, `executesForeignCode` ("runs or drives code REA does not own"), `mutatesDevice`, `idempotent`. `TOOL_EFFECTS` is a hand-written record for every public tool (`:54-762`); `toolContractMetadata(name)` throws if a tool lacks an audit entry (`:820-832`). Annotations are derived purely from effects (`annotationsFromEffects`, `:765-787`): `readOnlyHint` = no effect flag set; `destructiveHint` = `mayDiscardData || executesForeignCode || mutatesDevice`; `openWorldHint` = `launchesProcess || accessesNetwork`. Docs state annotations are hints, not authorization (`docs/mcp-contracts.md:1248-1259`).

**Untrusted-content marker.** OBSERVED `TARGET_CONTENT_FREE_TOOLS` allowlist of 7 tools (`toolEffects.ts:801-809`); every other tool, and any *unknown* name, is marked `returnsTargetContent: true`. Marked results (success and `isError`) carry `_meta.provenance: { untrusted_content: true, source: "analysis_target" }` (`src/server/toolResultEncoding.ts:30-49`; `docs/mcp-contracts.md:485-527`).

**Output schema helpers.** OBSERVED `evidenceResultOf(schema)` = strict `evidenceSchema.shape` with `normalized_result` specialised (`src/contracts/toolOutputSchemaPrimitives.ts:30-33`). `analysisDetailSchema` / `analysisDetailSummaryDefaultSchema` (`src/contracts/analysisDetail.ts`) are the `complete | summary` enums; `analysisDetailOutputSchemaOf(analysis, summary)` makes the output a union of the complete record and *exactly that tool's* summary view, so the advertised output does not accept other view kinds.

**Schema advertisement profiles.** OBSERVED compatibility projections (`docs/mcp-contracts.md:1589-1854`): input schemas inline refs, flatten root unions into one object (Gemini/Anthropic-gateway constraints), `compact` profile caps each input schema at 13,312 bytes, output schemas can be `full | compact (4,096 B) | off`. The server always validates against the canonical zod contract regardless of what was advertised; the projection "only widens".

**Shared Evidence input declaration.** OBSERVED `evidenceInputSchema = declareEvidenceInput(z.union([evidenceSchema, retainedEvidenceReferenceSchema]))` (`src/domain/evidenceReference.ts:31-33`); `evidenceSourceInputSchema` adds `{kind:"evidence-file", path, evidence_id?}` (`:52-55`). `getEvidenceInputContract(schema)` compiles the request schema once and finds exactly the declared Evidence slots so the application layer can resolve `retained-evidence` references before the format owner parses (`src/domain/evidenceInputContract.ts:111-120`; design rationale `docs/tool-design.md:180-223`). Borrowed inputs (`borrowEvidenceInput`, `borrowEvidenceArray`) let the SDK admit size before a large inline record is parsed.

### 2. Error contract

**Algebra.** OBSERVED abstract `AnalysisError extends Error` with `abstract readonly _tag: AnalysisErrorTag` (27 tags, `src/domain/analysisErrorBase.ts:7-35`). Constructor options carry caller-safe context that must survive projection: `diagnostics`, `limit`, `continuingCallerGuidance`, `invocationDiagnostics`, `capturedOutput`, `cleanup`, `partialObservation`, `cleanupPrimary` (`:79-98`). Cleanup ownership is tracked on the error (`cleanupIncomplete`, `cleanupResources`, dependencies, retry) so a composed failure reports only resources still unresolved (`:105-200`; rules in `docs/tool-design.md:386-404`).

**Concrete classes** (`src/domain/analysisErrorCore.ts`): `AnalysisInputError` with `issues: AnalysisInputIssue[]` (path, reason enum, expected, observed, min/max; "secret-safe correction metadata", `:172-189`) and `conformsToInputContract` (valid input the target cannot answer: no schema-inspection hint, `:148-154`); `AnalysisResourceConstraintError` with resource kind `memory|cpu|file-size|transport|capacity` and exactly one recovery (fixed allowance XOR remediation text, `:33-136`); `AnalysisTimeoutError`, `AnalysisCancelledError`, `AnalysisAccessDeniedError`, `AnalysisArtifactChangedError`, `AnalysisCapabilityUnavailableError`, `AnalysisUnsupportedTargetError`, `AnalysisOutputError`, `AnalysisProtocolError`.

**Result.** OBSERVED `Result<T,E> = {ok:true,value} | {ok:false,error}` with `ok()`/`err()` (`src/domain/result.ts`). Used at boundaries that read untrusted data, e.g. `readEvidenceBundle(...): Result<EvidenceBundle, AnalysisInputError | EvidenceIntegrityError>` (`src/domain/evidenceBundle.ts:303-327`). AGENTS.md: "Parse unknown boundary values and model expected failures with the tagged error algebra and `Result`."

**Projection.** OBSERVED `projectAnalysisError(error, callerLifetime)` (`src/domain/analysisErrorProjection.ts:67-70, 137-248`) produces `{ code, category, message, retryable, remediation:{action}, recovery?, details? }`. `code` is a closed union of 21 literals (`errorSchemas.ts:41-101`): `invalid_request, access_denied, artifact_changed, unreadable_output, capability_unavailable, unsupported_target, provider_unavailable, provider_timeout, resource_constraint, cancelled, artifact_integrity_mismatch, artifact_operation_failed, evidence_integrity_mismatch, truncated, process_capture_failed, cleanup_incomplete, revision_conflict, configuration_invalid, target_unavailable, execution_failure`. `category` is 10 coarse buckets (`:7-18`). Mapping tag to code is a static table (`analysisErrorProjection.ts:367-393`) plus specialised dispatchers; `cleanup_incomplete` overrides the underlying code whenever cleanup is unresolved (`:169`). `callerLifetime: "continuing" | "invocation"` changes remediation text (an MCP session can call `update_analysis_settings`; a CLI invocation cannot) (`analysisErrorBase.ts:57-76`).

**Recovery.** OBSERVED `AnalysisErrorRecovery` is a 3-member discriminated union (`src/domain/analysisErrorRecovery.ts:10-24`): `inspect_input_contract {operation}`, `inspect_timeout_state {operation, timeout_ms}`, `retry_cleanup {operation}`. Explicitly bounded: "never include the complete catalog or caller arguments" (`docs/mcp-contracts.md:158-178`).

**Details that survive failure.** OBSERVED `details` can hold `partial_observation`, `captured_output {stdout, stderr, truncated, exit_code, signal}`, `cleanup: "incomplete"`, `resources[]`, `cleanup_reason`, `cleanup_history`, `cleanup_report`, `limit`, `diagnostics`, `published/published_path/bytes` receipts for atomic writes (`analysisErrorProjection.ts:171-220`; `docs/mcp-contracts.md:1181-1200`). Oversized errors are retained as Evidence and replaced by a bounded diagnostic with an export reference (`docs/mcp-contracts.md:181-183`).

**Docs artefacts.** OBSERVED `docs/error-contract.schema.json` is `z.toJSONSchema(analysisErrorProjectionSchema)` (`errorSchemas.ts:130-133`), a `oneOf` of 21 object schemas. `docs/user-visible-errors.csv` has columns `harm_rank, location, trigger, current_copy, user_risk, proposed_replacement, implementation_status, verification_result`, 125 data rows; harm rank 1 is "Provider {providerId} cannot execute {operation}: {reason}" flagged for exposing provider/internal names, with a replacement and verification status recorded per row.

### 3. Evidence record (`src/domain/evidence.ts`)

OBSERVED fields of `evidenceBaseSchema` (`:87-110`, `.strict()`):

| field | type | in identity? |
|---|---|---|
| `evidence_id` | `ev_<64 hex>` | is the identity |
| `subject` | `{name, digest:{sha256}, local_path, format, architecture}` or null | only `digest/format/architecture` (`evidenceSubjectIdentity`, `:222-229`) |
| `provider` | `{id, name, version|null}` | yes |
| `analysis_profile` | `{provider, parameters, digest}` or null (`src/domain/analysisProfile.ts`) | yes; must be present, null is meaningful (`docs/mcp-contracts.md:21-23`) |
| `predicate_type` | string, default `rea.analysis` | yes |
| `operation` | string | yes |
| `parameters` | JSON object | yes |
| `caller_input_path` | verbatim request path | **no** |
| `raw_result` | provider representation or null | yes |
| `normalized_result` | JSON | yes |
| `confidence` | `observed|derived|inferred` | yes |
| `authority` | 5-enum | yes |
| `environment` | `{id, platform, architecture, isolation: none|process|container|virtual-machine}` or null | yes |
| `limitations` | string[] | yes |
| `relationship_coverage` | object or null | yes (pre-6.6 omitted null; now rejected with a specific diagnostic, `:256-268`) |
| `locations` | `address | address-range | artifact-path | file-offset | file-offset-range` | yes |
| `evidence_links` | `ev_*[]` | yes |

- `validateAnalysisProfileProvider` refuses a profile whose provider differs from the record provider (`:113-132`).
- `analysis_profile.digest` is itself a canonical digest of `{provider, parameters}` and is verified on parse (`analysisProfile.ts:20-30`). INFER: this is how REA invalidates snapshot reuse when analyzer settings change ("Analysis profiles version these declarations", `docs/mcp-contracts.md:142-143`).
- `parseEvidence` recomputes the digest and rejects tampered IDs (`authenticatedEvidenceSchema`, `:139-151`). Immutability is tracked with a `WeakMap` of sealed snapshots so re-authentication of large payloads is skipped (`:159, 291-296, 364-368`).
- `createEvidence(target, provider, observation)` defaults `confidence: "observed"`, `authority: "shipped-artifact"`, and injects a limitation string when no subject identity is available (`:464-475`).
- Canonical hashing is streaming SHA-256 over RFC 8785 ordering, refusing NaN/Infinity/circular (`src/domain/canonicalDigest.ts`).

**Observations vs derivations vs inferences vs unknowns.** OBSERVED:
- Primitive provider reads produce `observed` (`ProviderOperationExecution.ts:54` restricts to `observed | derived`).
- Workflows/comparisons/views produce `derived` (`WorkflowEvidence.ts:209`, `BrowserEvidence.ts:103`, `AnalysisViewService.ts:431`).
- Templating/grouping/type-guessing produce `inferred` (`InventoryProjectionEvidence.ts:87`, `ReconstructionObligationLedgerService.ts:50`, API-surface/environment-manifest views when any group is templated).
- Unknowns live outside the record: `residual_unknowns` arrays inside results for process/trace tools, plus the session-level ledger of `ResidualUnknown` records (`residualUnknown.ts:79-119`) with `status: open|investigating|blocked|contradicted|resolved`, `severity`, `required_authority`, `required_confidence`, `required_environment`, `supporting_evidence_ids`, `contradicting_evidence_ids`, `recommended_probes`, `relationships (depends-on|related-to|contradicts)`, `resolution` (verified/withdrawn/out-of-scope), `mutation_evidence_ids` (each revision is itself backed by Evidence). `unk_` id = digest of normalized question + domain + scope digest (`:22-38`); revisions chain via `previous_revision_digest`.
- Coverage is a separate axis from confidence: `relationship_coverage`, inventory `coverage.scope/status`, `facet_availability {available:true} | {available:false, reason}`, page `coverage.next_offset`. Docs repeat the rule "Missing coverage is unknown, not absence" (`docs/tool-design.md:159`).

### 4. Retained Evidence, summary vs complete, paging, bundles

- OBSERVED `detail: "summary"` on `analyze_javascript_application`, `inspect_binary_layout`, `inspect_artifact`, `inspect_wasm_artifact`, `inspect_macho`, `decode_interface_builder`, `analyze_function` (default summary for the last) retains the complete record and returns `inspect_analysis_view` summary Evidence whose `normalized_result.parent_evidence_id` names the parent (`docs/mcp-contracts.md:528-549`). Delivery detail is excluded from provider args and from identity.
- OBSERVED `inspect_analysis_view` input = `{source: retained-evidence | inline | evidence-file, view: {kind, ...}}`; views include `summary`, `native-inventory` (offset/limit/fields), `native` facets (pseudocode paged by UTF-16 code units without splitting surrogates), `native-observation`, `native-context`, `inventory`, `wasm`, `macho`, `batch-decompile`, `feature-trace`, `api-surface`, `environment-manifest`, `environment-state`, and many format-specific kinds (`src/contracts/analysisViewToolContracts.ts:20`; `docs/mcp-contracts.md:241-331, 573-593`). Each view is new Evidence (`predicate_type: "rea.analysis-view"`, parameters `{parent_evidence_id, view}`) carrying its own digest and a link to the parent (`AnalysisViewService.ts:407-418`).
- OBSERVED oversized results return `resource_constraint` with `details.resource: "transport"` and `details.reported_limits.evidence_reference` plus a ready-to-call `continuation` (`docs/mcp-contracts.md:221-233, 317-331`). Nothing is truncated; the complete record stays in the session.
- OBSERVED `get_evidence_bundle` input (`src/contracts/sessionToolSchemas.ts:17-44`): `detail: complete | summary | records | links`, `filters {evidence_id, operation, target_sha256, analysis_profile_digest, procedure_address}` (`src/domain/evidenceRecordSummary.ts:10-16`), `offset/limit`, `traversal: direct | retained-closure`, `link_status`. Summary rows: `{evidence_id, subject, provider, operation, confidence, authority, analysis_profile_digest, retention: "complete-record", limitation_count, native_dossier}` plus `missing_direct_links` (`:50-76`).
- OBSERVED `export_evidence_bundle` writes a canonical bundle atomically (`sessionToolContracts.ts:95-99`); `import_evidence_bundle` validates every ID and manifest then merges; "Imported content is data only and is never executed" (`:100-104`). Bundle schema (`src/domain/evidenceBundle.ts:45-53`): `artifacts[]`, `providers[]`, `environments[]`, `scenarios[]` (records with `authority: controlled-replay`), `captures[]` (`predicate_type: rea.process-capture`), `unknowns[]`, `records[]`, all derived deterministically from records and sorted by code point (`:103-158`). Duplicate IDs and non-canonical manifests are rejected.
- OBSERVED retention has a fixed in-memory allowance derived from V8 heap headroom, no eviction, no automatic persistence; `reset_investigation(save_path)` is the explicit destructive boundary (`docs/tool-design.md:272-313`).

### 5. Redaction rules

OBSERVED policy (AGENTS.md "Product and Authority"): "Redact transport authentication credentials and explicitly marked sensitive values; do not infer secrecy from names or text patterns. Do not persist the ambient environment merely because a child inherits it."

Implementation:
- `CaptureRedaction` (`src/browser/history/CaptureRedaction.ts`): credential header set `{authorization, proxy-authorization, cookie, set-cookie}` (`:7-12`); URL userinfo stripped via `parseRecordedTransportUrl` which returns a raw span or `"ambiguous"` (then the whole value is withheld) (`src/domain/recordedTransportUrl.ts:18-107`; `CaptureRedaction.ts:63-86`); caller `sensitive_values` literals cause the whole field to be nulled; each redaction is recorded as `{pointer (RFC 6901), reason: "transport-credential" | "explicit-sensitive-value", scope?: "property-name"}` (`src/domain/webNetworkCapture.ts:90-104`). Declaration strings "are never persisted in Evidence parameters" (`:64-69`).
- `redactExplicitText` (`src/domain/explicitSensitiveValues.ts`): longest-first literal replacement; picks a replacement token that cannot itself contain a literal; returns `""` if boundaries recombine into a literal.
- `redactExplicitFailure` / `redactSensitiveFailure` (`src/domain/explicitSensitiveFailure.ts:73-312`): re-creates each typed error with redacted message/remediation/diagnostics/captured output/cleanup resources/issue paths, while preserving the tag, cleanup ownership (`mirrorCleanup`) and partial observations; identity values (paths, digests) that would be altered are dropped entirely rather than partially rewritten (`:25-49`).
- Process capture: sensitive scheduled inputs are committed as `{data: "", sensitive: true, input_bytes}` (`src/domain/process/processScenario.ts:98-103`; schema invariant `processCapture.ts:43-65`); terminal output redaction is an Aho-Corasick-style trie with hard resource ceilings and *withholds all terminal text on failure* (`src/process/capture/ProcessSensitiveOutput.ts:8-32`).
- Provider diagnostics: `redactCapturedTransportCredential` handles a token cut by an output budget (`src/process/ProviderDiagnosticRedaction.ts`).
- Browser scenarios: secrets are env-var references, never inline; Basic password must come from a secret/saved value, not a literal, because "Evidence records literals as written" (`src/domain/browserScenario.ts:36-41, 211-220`). Frida: auth values "held only for the active call or session and are excluded from Evidence" (`docs/frida-instrumentation.md:5`).
- UNKNOWN: there is no generic redaction pass over arbitrary `normalized_result` payloads; each producer owns its own format-aware exclusion (INFER from `explicitSensitiveFailure.ts:91` comment "Other observation owners already apply their format-aware exclusions").

### 6. Tool shapes (`docs/tool-design.md`)

OBSERVED table (`:14-27`): `inspect` (facts about one object: fields, locations, facet availability), `search`/`list` (stable ordering, pagination), `trace` (typed edges, supporting facts, unresolved paths), `compare` (identity, comparable coverage, supported deltas), `workflow` (inline answer + contributing evidence + partial facets), `observe`/`capture` (authority, launch/attach effects, lifecycle, cleanup status). "These are task shapes, not mandatory prefixes." Primitives report reusable facts; workflows compose them "without hiding caller choices or uncertainty" (`:29-49`). Agents are expected to compose experiments with ordinary scripts; "a plan without execution does not establish runtime behavior" (`:46-49`).

Contract rules (`:145-178`): strict inputs with explicit target identity; return facts inline with identity, locations, Evidence refs, coverage, confidence, limitations; keep observed/derived/inferred distinct; declare effects truthfully; limits only for real constraints.

Comparison discipline (`:242-258`): separate proofs for lookup, correspondence, content and coverage; a hash cannot prove two values belong to corresponding objects.

### 7. Reconstruction ledgers and readiness

OBSERVED `build_reconstruction_obligation_ledger` (`docs/reconstruction-obligation-ledgers.md`): obligations have `obligation_id`, `obligation_version`, `application_layer`, `family`, `target`, `required_case_kinds` (positive/negative/malformed/cancellation/teardown), `required_original_authority` / `required_fixture_authority` / `required_verifier_authority`, `dependency_obligation_ids`, `residual_unknown_ids`, `unavailable_authority`, `required_next_evidence`, `disposition`, `review_evidence_ids`. A manifest binding names an owner with `owner_sha256`, fixtures per case kind, and a verifier whose `result_evidence_id` must be in the bundle and must enumerate the obligation id. Closure requires authenticity + authority + applicability (`:341-406`); "a green unit test cannot close a packaged-process obligation" (`:337-338`). `ledger_id` and `closure_digest` identify the full result. `EvidenceCompletionLedger` (`src/domain/evidenceCompletionLedger.ts`) is the smaller primitive: `{claim_id, status: pass|fail|unsupported|truncated|unknown, evidence_ids}` with `complete` only if all pass; `ecl_` id is a digest of sorted records.

OBSERVED `docs/reconstruction-readiness.md`: an internal verifier (`npm run verify:readiness`, not a public tool) with nine synthetic stages (`discover-classify`, `diagnose-environment`, `acquire-authority`, `static-analysis`, `reactive-scenarios`, `compare-authority-candidate`, `preserve-contradictions`, `verify-reconstruction-closure`, `export-replay`) and fail-closed rules: "`fail`, `unknown`, `unsupported`, `truncated`, and `skipped` never aggregate to `pass`"; "a denied or cancelled grant reports a process launch" is a failure (`:79-97`). Note the `acquire-authority` stage explicitly tests "missing scope reported, narrow grant resumed, expansion reprompted, and denial launched nothing" (`:67`), i.e. REA already thinks about authority grants as observable events, even though the runtime has no permission system of its own (`docs/mcp-contracts.md:1244-1246`).

### 8. Controlled-replay Evidence: REA's existing "action record"

OBSERVED `UnverifiedProcessCapture` (`src/domain/process/processCapture.ts:222-286`):
- `manifest`: tool version, provider version, platform/arch, `started_at/completed_at`, committed `scenario` and `comparison_contract` with their SHA-256s, `selected_executable_sha256` (sampled before spawn) vs `executable_sha256` (bound only if path metadata stayed stable), `executable_identity {state: path_metadata_unchanged | unknown, reason}`.
- Observations: `frames`, `rendered_frames`, `interaction_events` (each with `scheduled_at_ms`, `dispatched_at_ms`, `outcome: dispatched | target_exited | failed`), `exit {code, signal, reason, finalization}`, `settlement {state: quiesced | alive_at_deadline | unverifiable, cleanup_outcome}`, `process_samples`, `filesystem_checkpoints` with before/after `FileState` and `FileEffect {created|deleted|modified|unchanged|unknown}`, a global `event_journal` ordering all collections.
- Honesty fields: `truncated`, `truncation_details`, `limitations[]`, `residual_unknowns[{scope: terminal|interaction|exit|process|filesystem|cleanup|network|environment, reason}]`, `cleanup {owned_process_group: "verified", temporary_root: "removed", unverified_processes?}`.
- On failure, the same facts become `IncompleteProcessCaptureObservations` where every field is `{state: available, value} | {state: unavailable, reason}` (`:316-356`).

OBSERVED browser equivalent (`src/domain/browserScenarioCapture.ts:14-111`): `browser {mode: launch|connect, process_ownership: provider-owned|external, cleanup: terminated-owned-process|disconnected-external|incomplete, product, version}`, `scenario {start_origin, action_count, secret_references, ...}`, `steps` (initial state + one per action, contiguous indices), `events {retained, dropped, items, omissions}`, `completeness`, `limitations`. Schema refinements enforce that counts agree with arrays.

---

## Design choices that make it extensible

- **One record type for everything.** Evidence is generic over `normalized_result`; new domains add a `predicate_type`/`operation` and an output schema via `evidenceResultOf`, never a new envelope (`toolOutputSchemaPrimitives.ts:30-33`). Bundles, views, comparisons, unknown links, and exports all work on the envelope alone.
- **Identity excludes display metadata.** Path aliasing, `caller_input_path`, and delivery `detail` never fork identity (`evidence.ts:96-99, 221-250`; `retainedSummary.evidenceParameters`). This lets the same observation be cited from CLI, MCP, or a file.
- **Content-addressed everything.** `ev_`, `unk_`, `ecl_`, profile digests, `ledger_id`, `closure_digest`, scenario commitment hashes. Tamper detection is parse-time (`parseEvidence`), and references (`{kind: retained-evidence, evidence_id}`) are cheap and safe.
- **Declared Evidence slots.** `declareEvidenceInput` + `getEvidenceInputContract` let any request schema accept inline-or-reference in any nesting without adapters maintaining a second list of paths (`evidenceInputContract.ts`; `tool-design.md:180-223`).
- **Effects as data, annotations derived.** Adding a tool forces an `TOOL_EFFECTS` entry (throws otherwise) and defaults to untrusted content until audited (`toolEffects.ts:101-117, 120-124`).
- **Closed vocabularies with "omitted member is false" rule** for effects/annotations in `binary_session` output (`docs/mcp-contracts.md:905-910`).
- **Error algebra with one projector.** New error types add a `_tag`, a code in the static table, and the TypeScript `satisfies Record<Exclude<Tag, Specialized>, Code>` forces exhaustiveness (`analysisErrorProjection.ts:367-393`).
- **Retain-then-view instead of truncate.** Any producer can opt into `detail: summary` and gain paging for free via `inspect_analysis_view`; views are themselves Evidence so paging never loses provenance.
- **Family scoping.** `TOOL_FAMILY_DEFINITIONS` is the single grouping used for runtime scope, product catalog, prompts and doctor.
- **Caller-lifetime-aware remediation** (`continuing` vs `invocation`) means the same error text is correct for a long-lived MCP session and a one-shot CLI.
- **Schema profiles decoupled from validation.** Compatibility projections (`compact`, flattened unions) never weaken server-side validation.

---

## What to distill for an observe+drive control plane

### Borrow directly
1. **The Evidence envelope** (subject identity by digest, provider, profile, operation, parameters, raw vs normalized result, confidence, authority, environment, limitations, coverage, locations, links, content-addressed id). For a desktop target, `subject` becomes the app/process/window identity (binary digest + version + window/accessibility tree root), and `environment.isolation` already has `none | process | container | virtual-machine`.
2. **Confidence × authority × coverage as three independent axes**, plus a separate unknown ledger. An OCR'd screen read is `inferred`; an accessibility-tree read is `observed`; a diff is `derived`. Coverage says which part of the UI/filesystem was actually examined.
3. **"A projection never strengthens its parent"** (`AnalysisViewService.ts:421`). Any summary/diff/plan built from inferred observations stays inferred.
4. **Effects audit → derived annotations → host approval hooks.** Extend `ToolEffects` with `drivesInput` (synthetic keyboard/mouse), `changesUiState` (already exists), `mutatesTarget`, `executesForeignCode`, `mutatesDevice`. Keep REA's rule that annotations are hints and that hosts apply approval using the effects plus the `untrusted_content` marker ("requiring approval for process-launching or device-mutating tools after target content was read", `docs/mcp-contracts.md:523-526`). That is the prompt-injection firewall for a driver.
5. **Error projection with `partial_observation`, `cleanup`, `captured_output`, bounded `recovery`.** For a driver, a failed click still has value: what was on screen before, whether the input was dispatched, whether the app is still alive.
6. **Retain-then-view + canonical bundle export/import.** Screen captures and accessibility trees are large; return a summary with `parent_evidence_id`, page via views, export bundles for handoff.
7. **Redaction policy**: strip transport credentials and caller-declared secrets; record `{pointer, reason}` rows; never persist secret declaration strings; refuse to produce views that would reveal values from sensitive captures (`environment-manifest` refuses parents inspected with sensitive values). For a UI driver: typed-in secrets should be committed as `{data: "", sensitive: true, input_bytes}` exactly like `sensitiveProcessInputProjection`.
8. **Authenticity / authority / applicability split** from the obligation ledger, applied to "did the action succeed": authentic (record digest verifies), authority (post-state was *observed* by a trusted reader, not inferred from the absence of an error), applicable (post-state observation is bound to the same target identity and revision as the pre-state).

### The "action record" an observe+drive agent needs (INFER, grounded in the controlled-replay shape)

Model it as Evidence with `predicate_type: "<cp>.action"`, `authority: "controlled-replay"` (REA's term for "we drove it under our own control"), `confidence: observed` only when every sub-observation is observed. `normalized_result` should carry:

- **Target identity**: app/binary digest and version, PID + start identity (REA: "a PID and executable path alone do not establish ownership after exit or reuse", AGENTS.md), window/document id, `executable_identity {state, reason}` like `processCapture.ts:260-263`.
- **Preconditions observed**: a retained observation `evidence_id` (accessibility tree / screenshot digest / filesystem checkpoint) with its own `confidence` and `coverage`, plus `observed_at`. Any predicate the agent asserted ("button X is enabled") recorded with the reader that established it. Missing precondition read → `unknown`, not assumed.
- **Authority used**: which grant authorised the action (scope, grantor, time, expiry), the host permission state (macOS Accessibility/Screen Recording as in `docs/mcp-contracts.md:1283-1287`), and whether the grant was narrow/expanded/denied. REA's readiness stage `acquire-authority` names exactly these events ("missing scope reported, narrow grant resumed, expansion reprompted, denial launched nothing"). A denied grant must record that nothing was dispatched.
- **Action issued**: typed action (click/type/key/menu/API call), target coordinates or element identity, `scheduled_at`/`dispatched_at`, `outcome: dispatched | target_exited | failed` (mirror `interactionEventSchema`), input bytes for sensitive text with `data: ""`.
- **Postconditions observed**: a second retained observation id, settlement state (`quiesced | alive_at_deadline | unverifiable`), diff vs precondition as a *derived* record citing both, filesystem/network effects with `created|deleted|modified|unchanged|unknown` statuses, and `residual_unknowns[{scope, reason}]` for anything not re-observed.
- **Cleanup / ownership**: what the agent owned (launched process group, temp files, injected scripts) and `cleanup_outcome` per resource with `unverified` as a legal value.
- **Links**: `evidence_links` to pre/post observations and to the grant record; `limitations[]` for reader caveats (OCR, partial tree, occlusion).
- **Commitment**: hash the issued action plan separately from the observation (REA: `full_scenario_sha256` vs `comparison_contract_sha256`) so replays can be compared.

### Avoid
- Do not infer success from the absence of an error (REA: "Process exit does not establish output closure"; "an inventory miss does not prove that the artifact lacks the requested item").
- Do not let paging/exhaustion imply completeness of the world (REA repeats this on every view).
- Do not make tool names carry semantics that effects should carry (`annotationsFromEffects` comment: "without tool-name or tool-family heuristics").
- Do not over-redact by heuristics; REA deliberately rejects name-based secrecy guessing. The cost is that the caller must declare secrets.

---

## Constraints & risks

- **Scope boundary.** REA is explicitly a read/observe tool: Ghidra "without modifying executable bytes or controlling a GUI", IDA "read-only", Windows Ghidra "no mutation authority" (AGENTS.md). Its driving tools (`capture_process_scenario`, `capture_browser_scenario`, `trace_native_ui_action`, Frida, GDB/rizin) are flagged `executesForeignCode`/`mutatesTarget` and described as destructive. CONTRIBUTING requires a maintainer scope decision for "target execution, injection, mutation, or protection recovery" (AGENTS.md). An observe+drive control plane sits on the far side of that line; borrow the discipline, not the authority posture.
- **Protection-recovery flag.** REA's Mach-O `encryption` facet, Electron fuses/ASAR integrity (`record-and-continue` for mismatched hashes, `docs/mcp-contracts.md:1515-1588`), APK signature inspection and IL2CPP metadata tooling are adjacent to DRM/licensing. REA reports facts and refuses to "silently exempt" integrity failures; the downstream project should keep the same posture: observe integrity state, never bypass it.
- **No authorization model.** "REA does not require permission grants or per-call approval flags" (`docs/mcp-contracts.md:1244-1246`); it relies on the MCP host. A driver needs its own grant records (see action record above); REA offers vocabulary but no implementation.
- **Volume and heap.** Evidence retention is in-memory with a fixed allowance and no eviction; a driver producing screenshots per action will hit this fast. REA's answer (export/reset/import) is manual.
- **Heavy, prose-based contract.** `docs/mcp-contracts.md` is ~1,950 lines; tool descriptions are paragraphs (`inspect_analysis_view` description is a single multi-KB string). Advertised-schema size already forced the `compact` profile. A control plane with many driver tools will face the same context cost.
- **Zod-specific machinery.** `declareEvidenceInput` relies on zod registries and `_zod.processJSONSchema` internals (`evidenceReference.ts:39, 47`); porting to another validator means re-implementing the slot-discovery.
- **Legacy identity drift.** The pre-6.6 `relationship_coverage` change shows that adding a semantic field to the identity projection invalidates all saved IDs; REA chose to reject rather than migrate. Decide the versioning story for the action-record identity up front.
- **Redaction is producer-local.** No generic pass; each new producer must implement its own exclusions. A driver that emits raw screenshots has no literal to match against.
- **Untrusted-content default.** Everything not allowlisted is marked untrusted, including errors. Correct for safety, but a UI driver's results are almost all target-derived, so the marker alone will not discriminate; approval policy must key on effects.

---

## Open questions

- UNKNOWN how `Evidence.environment` is populated in practice for process/browser captures (schema exists; I did not trace a producer setting `isolation`).
- UNKNOWN whether any REA tool records *authority grants* as Evidence at runtime, or only in the synthetic readiness fixture (`acquire-authority` stage). grep found no `grant` predicate in `src/domain`.
- UNKNOWN exact content of `docs/design/limits-and-recovery.md` "What agents do" section (headings only read); it may hold additional guidance on bounded retries worth borrowing.
- UNKNOWN the status of issue #4276 ("broader retention decision" for raw `partial_observation` not recorded in bundles, `docs/mcp-contracts.md:1222-1226`); for a driver, un-retained partials after a crash are the most valuable records.
- INFER but unverified: whether `trace_native_ui_action` (mentioned `docs/mcp-contracts.md:928-934`) actually dispatches UI events or only traces authored UI metadata; the name suggests the latter.
- How would REA's "controlled-replay" authority map when the agent drives *and* a human is also interacting with the same app (mixed authority)? REA's model assumes exclusive ownership of the captured process.

---

## Sources

Source files (checkout-relative):
- `AGENTS.md`
- `src/contracts/toolContractTypes.ts`, `toolContracts.ts`, `toolFamilies.ts`, `toolEffects.ts`, `toolOutputSchemaPrimitives.ts`, `analysisDetail.ts`, `errorSchemas.ts`, `sessionToolContracts.ts`, `sessionToolSchemas.ts`, `analysisViewToolContracts.ts`
- `src/domain/evidence.ts`, `evidenceSubject.ts`, `evidenceReference.ts`, `evidenceInputContract.ts`, `evidenceBundle.ts`, `evidenceRecordSummary.ts`, `evidenceCompletionLedger.ts`, `evidenceErrors.ts`, `analysisProfile.ts`, `canonicalDigest.ts`, `residualUnknown.ts`, `result.ts`
- `src/domain/analysisErrorBase.ts`, `analysisErrorCore.ts`, `analysisErrorRecovery.ts`, `analysisErrorProjection.ts`, `analysisErrorPresentation.ts`
- `src/domain/explicitSensitiveValues.ts`, `explicitSensitiveFailure.ts`, `recordedTransportUrl.ts`, `webNetworkCapture.ts`, `src/browser/history/CaptureRedaction.ts`, `src/process/ProviderDiagnosticRedaction.ts`, `src/process/capture/ProcessSensitiveOutput.ts`
- `src/domain/process/processCapture.ts`, `processScenario.ts`, `src/domain/browserScenario.ts`, `browserScenarioCapture.ts`
- `src/application/analysisView/AnalysisViewService.ts`
- `src/server/toolResultEncoding.ts`, `ToolResultDelivery.ts`

Docs (checkout-relative):
- `docs/mcp-contracts.md`, `docs/tool-design.md`, `docs/reconstruction-obligation-ledgers.md`, `docs/reconstruction-readiness.md`, `docs/reconstruction-coverage.md` (authority table), `docs/error-contract.schema.json`, `docs/user-visible-errors.csv`, `docs/browser-observation.md` (redaction paragraphs), `docs/frida-instrumentation.md`
