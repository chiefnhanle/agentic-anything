# REA's removed permission system: archaeology of grants, ceilings, approvals and elicitation (2026-07-14 to 2026-10-05)

Research date: 2026-11-05

Scope: reconstruct the permission and approval system REA built and then deleted in commit `b5e98915c` (PR #555). The aim is to stop agentic-anything's planned authority model from repeating its failures. Every other report recommends re-adding grants, confirmations and elicitation (`PLAN.md` "Key findings"; `survey-safety-rails-and-authorization-models.md` §9; `rea-desktop-applications-and-agent-environments.md` §12 G6). This report does not repeat those recommendations. It tests them against what REA actually built.

Source: the local read-only checkout `/Users/nhanclie/rea`, read with `git log`/`git show` only. HEAD is `3484432fb` (2026-11-05). The parent of the removal commit is `4f099a10f` (written `@4f099a10f` below). Issue and PR text was read with `gh ... view` (read-only).

Legend:
- **OBSERVED**: read in the repo at the cited commit, or in GitHub issue/PR text.
- **SECONDARY**: web or search summary.
- **INFERRED**: my interpretation.
- **UNKNOWN**: could not establish.

Citations use the form `src/path.ts:line@commit`.

---

## 1. Summary

- **OBSERVED. A complete subsystem existed for 83 days.** It was introduced by `190efcaf1` (2026-07-14, "feat(core): add typed policy and integrity contracts", closing issue #114) and deleted by `b5e98915c` (2026-10-05, #555). The deleted files alone total 4,545 lines at the parent: 36 files, about 2,300 lines of source and 2,200 of tests. The pieces were:
  - a pure domain evaluator (`src/domain/permissionPolicy.ts`, 339 lines);
  - a stateful authority with connection overlays (`src/application/PermissionAuthority.ts`, 435 lines);
  - an owner-only project grant store (272 lines);
  - `rea policy status|list|explain|revoke` (270 lines);
  - MCP elicitation for process capture (243 lines);
  - about 30 `REA_*` environment variables that defined the ceilings;
  - per-call approval booleans across tool schemas.
- **OBSERVED. The model was:** administrator *ceilings* (from env vars), with *grants* beneath them. Grants had a lifetime of `once`, `session`, `project` or `administrator`. Every governed operation built an exact `PermissionRequest` from these dimensions: capability, canonical roots, executables, env-var names, origins, network rank, mount, and operation identity. Evaluation was a subset test, first against the ceiling and then against one grant (`permissionPolicy.ts:127-175@4f099a10f`).
- **OBSERVED. In practice the ceiling usually was the grant.** `parseConfig` turned every ceiling into an `administrator:<capability>` grant unless a per-family `*_AUTO_GRANT=false` was set. The defaults were:
  - process capture: auto-grant on (`environment.ts:36@4f099a10f`);
  - browser and Electron automation: auto-grant off;
  - all other capabilities: always auto-granted (`parseConfig.ts:143-153@4f099a10f`).
- **OBSERVED. Elicitation never ran against a real client.** It used the MCP `2026-07-28` multi-round `inputRequired` continuation, not push-style `elicitation/create`. It covered only `capture_process_scenario`, and only when the operator had set `REA_PROCESS_CAPTURE_AUTO_GRANT=false`. Its tests used synthetic request contexts. The "real modern MCP client round trip" stayed an open acceptance gap (issues #96, #389) until both were closed as superseded on 2026-10-05.
- **OBSERVED. The fatal bug was #545.** A user on Windows had a correctly configured browser-automation ceiling and was still denied. The diagnostic told them to widen the ceiling. The real cause was that no grant existed, and there was no CLI command to issue one: `writeProjectPermissionStore` had no production caller at the parent. #552 fixed the message by adding a `grant` remediation. The whole system was deleted the same day.
- **OBSERVED. The stated reason, from the PR #555 body:** "Tool selection expresses intent." Approval booleans "forced callers to restate implementation behavior". Grant restrictions were removed; "actual OS permissions, target binding, protocol constraints, lifecycle cleanup, and replay confinement remain." AGENTS.md now says: "A selected operation already expresses intent; do not require approval booleans or repeated permission declarations."
- **INFERRED. The deeper reasons:**
  1. Most scope dimensions were *declared, not enforced*. The child process ran unconfined as the user, and network was a literal `"host"`. A grant therefore checked the agent's parameters, not the process's real authority.
  2. The approvals that agents could reach were values the agent supplied itself (`observation_approved: z.literal(true)`), so they gave no protection against a confused or injected agent.
  3. Three overlapping policy layers guarded process capture.
  4. The friction landed on legitimate users while the security gain was close to zero.
- **For agentic-anything:** re-adding authority is still right for the *drive*, *attach* and *inject* tiers. The gate has to work differently from REA's:
  - it binds to the *full action*, with a digest, not to a scope summary;
  - every dimension carries an `enforced | declared-only` status;
  - a grant issuer exists from day one, and denial reasons are distinct;
  - observe tiers are left ungated;
  - nothing the agent can type counts as approval.

## 2. Timeline (OBSERVED, `git log` and issues)

| Date | Commit / issue | Event |
|---|---|---|
| 2026-07-13 | #96, #114 opened | #114: "Replace capability-specific startup-environment friction with one unified, auditable policy engine". #96: "Use MCP `elicitation/create` ... Allow once / for session / Remember for project / Deny" |
| 2026-07-13 | `9526389f8` | `network_access` narrowed from `["loopback","host"]` to `z.literal("host")` because "the current PTY adapter uses host networking". From then on, process capture required `REA_PROCESS_ALLOW_EXTERNAL_NETWORK=true` |
| 2026-07-14 | `190efcaf1` | Policy engine, project store and `rea policy` land. 11 capabilities, including `evidence_read/write`, `snapshot_read/write`, `investigation_*` and `reference_read` |
| 2026-07-14 | `7a4cfb4fb`/`6397e5416`, `3ae7109fa`/`49e843e8f` | Revoke requires interactive confirmation or `--yes`. SIGHUP reloads become atomic |
| 2026-07-14 | `f840e7074` | "defer cache write grants" (`DeferredFileAuthorization.ts`). Analysis caching had to be routed around the grant check |
| 2026-07-17 | #96 comments | The audit finds that "current configuration turns every permission ceiling into an administrator grant ... there is no ceiling-only state within which an elicited grant can operate". It also finds no OS-enforced loopback, so "adding a loopback enum would be a false security claim". `AUTO_GRANT=false` is added. The project-lifetime elicited grant is removed over TOCTOU and rollback gaps |
| 2026-07-18 | `e6cfce390`, `2e5b44365` | Scoped process-capture elicitation lands (21 files, +1,260). The same day, 1,780 lines of `WindowsNativeAuthority` are deleted ("simplify authorization boundaries") |
| 2026-07-24 | `f0069cbee`, `035e1144b` | `include_storage_fingerprints` becomes a separate approval. `tools/list` becomes policy-aware ("advertise available tools dynamically") |
| 2026-08-05 | `8f229ede7` | `electron_automate` is added. Peak inventory: 18 capabilities |
| 2026-09-29 | `0d96275b6`, `c34efca4c`, `f55c2b926`, `2f08c7c93`, `7eb1a9500`, `1b0ab75a8`, `bb6a58c2a` | First removal wave. Evidence, snapshot, reference and workspace roots go ("Evidence bundles and analysis snapshots use the path supplied"). Caps drop to 10. `--approved` is dropped from `rea analyze`. "Artificial" ceilings are removed. Extraction approval is "stabilized" to a fixed tmpdir scope |
| 2026-10-05 08:57Z | #545, #548 | Windows user reports: a ceiling-correct browser automation request is denied with ceiling-widening advice; POSIX-only paths block process capture on Windows |
| 2026-10-05 | `e793e124a` (#552) | `PermissionRemediation` gains `grant` ("stop blaming the ceiling for a missing grant") |
| 2026-10-05 12:16Z | `b5e98915c` (#555) | Everything is deleted: 350 files, +5,345/-10,597 |
| 2026-10-05 13:15Z | #572 | Replay engines and prepare/execute flows are removed, including `approved_plan_sha256` and `run_controlled_replay` |
| 2026-10-05 13:21Z | #96 closed NOT_PLANNED | "This closure does not claim that the old grant-system acceptance criteria were implemented." |
| 2026-10-06 | `2919def82` | The shipped skill (v24) is cleaned of "obsolete filesystem grants, per-call approval flags ... DMG approval flags". `ToolEffects` is re-audited instead |

## 3. Data model at the parent commit (OBSERVED, `@4f099a10f`)

### 3.1 Domain: scopes, ceilings, grants, decisions (`src/domain/permissionPolicy.ts`)

- **Capabilities.** `PERMISSION_CAPABILITIES` (`:6-17`) lists `process_capture`, `browser_observe`, `browser_automate`, `electron_observe`, `electron_automate`, `v8_inspector_observe`, `artifact_extract`, `native_mount`, `javascript_replay` and `managed_runtime`. Observe and automate were already separate capabilities for browser and Electron.
- **Network.** `PermissionNetwork = "none" | "loopback" | "external"` (`:23`) is ordered, and "each mode includes the modes before it". Comparison is by `networkRank` (`:321-330`).
- **Scope.** `PermissionScope` (`:26-34`) is `{capability, roots[], executables[], environment_names[], origins?[], network, mount}`. `PermissionRequest` adds `operation_identity` (`:37-39`). `PermissionCeiling = PermissionScope` (`:42`), so ceilings and requests share one type.
- **Grant.** `PermissionGrant` (`:45-50`) adds `grant_id`, `lifetime: "once"|"session"|"project"|"administrator"`, `operation_identity: string|null` and `expires_at: string|null`.
- **Policy.** `PermissionPolicy` (`:63-68`) is `{ceilings, grants, revoked_grant_ids:Set, consumed_grant_ids:Set}`. It is immutable and replaced on each change.
- **Decision.** `PermissionDecision` (`:83-97`) is `{allowed:true, grant_id, lifetime}` or `{allowed:false, reason, missing}`. The reasons are `outside_administrator_ceiling | grant_required | grant_revoked_or_consumed | grant_expired`. `MissingPermissionScope` (`:53-60`) holds "names and paths but never values".
- **`grantPermission`** (`:111-124`) rejects a grant outside every ceiling, and a `once` grant without an `operation_identity`.
- **`evaluatePermission`** (`:127-175`) works in three steps:
  1. Run a ceiling subset test.
  2. Filter candidate grants: same capability, not revoked or consumed, not expired, and for `once` grants, the same operation identity.
  3. Pick the *first single* grant that fully covers the request.

  On denial, `missing` is computed against the **union of candidate grants** (`:173`), not against the ceiling. This is the root of #545 (§7.1).
- **Path containment.** Roots and executables are matched by containment (`isPathContained`, `:304-316`; `containsAny`, `:318-319`). An executables entry that is a directory therefore authorizes every binary under it (issue #567).
- **Lifecycle helpers:**
  - `consumePermission` marks `once` grants consumed (`:185-197`);
  - `revokePermission` (`:200-206`);
  - `clearSessionPermissions` drops `once` and `session` grants (`:209-216`);
  - `reloadPermissionCeilings` (`:219-225`).
- **INFERRED latent bug.** `missingFromScopes` (`:252-293`) checks each dimension against `relevant.some(...)` over *all* ceilings for the capability. With two ceilings, a request could take its roots from ceiling A and its executables from ceiling B. Config produced at most one ceiling per capability, so this never fired.

### 3.2 Configuration: env vars → ceilings → administrator grants

- **OBSERVED. About 30 policy variables** (`src/config/environment.ts:35-113@4f099a10f`):
  - `REA_PROCESS_CAPTURE_ENABLED`, `REA_PROCESS_CAPTURE_AUTO_GRANT` (default `"true"`, `:36`), `REA_PROCESS_EXECUTABLE_ROOTS_JSON`, `REA_PROCESS_WORKING_ROOTS_JSON`, `REA_PROCESS_ALLOWED_ENV_JSON`, `REA_PROCESS_ALLOW_EXTERNAL_NETWORK`;
  - `REA_BROWSER_OBSERVE_ENABLED`, `REA_BROWSER_CDP_ENDPOINTS_JSON`, `REA_BROWSER_ALLOWED_ORIGINS_JSON`;
  - `REA_BROWSER_SCENARIO_{ENABLED,AUTO_GRANT,EXECUTABLE_ROOTS_JSON,CDP_ENDPOINTS_JSON,ALLOWED_ORIGINS_JSON,ALLOWED_ENV_JSON}`;
  - `REA_ELECTRON_{OBSERVE_ENABLED,AUTOMATE_ENABLED,AUTOMATE_AUTO_GRANT,AUTOMATE_EXECUTABLE_ROOTS_JSON,AUTOMATE_APPLICATION_ROOTS_JSON}`;
  - `REA_V8_INSPECTOR_OBSERVE_ENABLED`, `REA_JAVASCRIPT_REPLAY_{ENABLED,ROOTS_JSON}`, `REA_MANAGED_RUNTIME_{ENABLED,ROOTS_JSON}`;
  - `REA_ARTIFACT_NATIVE_MOUNT_ENABLED`, `REA_ARTIFACT_INTEGRITY_CONTINUE_ENABLED`, `REA_REFERENCE_SECRET_PATTERNS_JSON`;
  - `REA_PERMISSION_PROJECT_{ROOT,STORE}`, which must be set together (`:115-123`).
- **OBSERVED. `buildPermissionCeilings`** (`src/config/parseConfig.ts:100-111@4f099a10f`) always starts with `permissionScope("artifact_extract", ["/"])`, a ceiling of the whole filesystem. It then appends one ceiling per enabled family. The process-capture ceiling is `roots = workingRoots`, `executables = executableRoots`, `environment_names = allowedEnvironment` and `network = networkAccess` (`src/config/processCapture.ts:54-66@4f099a10f`).
- **OBSERVED. `administratorPermissionGrants`** (`parseConfig.ts:143-153@4f099a10f`) maps ceilings to grants through `administratorGrants` (`src/config/permissions.ts:21-30@4f099a10f`, `grant_id: "administrator:<capability>"`, `lifetime: "administrator"`, no expiry). It excludes only `process_capture`, `browser_automate` and `electron_automate`, each only when its `*_AUTO_GRANT` is not `"true"`.
- **INFERRED.** For seven of ten capabilities, "ceiling" and "grant" were the same object. For process capture they were the same by default. Only browser and Electron automation defaulted to the two-level ceiling/grant split, and those are exactly the capabilities where #545 hit.

### 3.3 Project store (`src/application/ProjectPermissionStore.ts@4f099a10f`)

- **OBSERVED. Schema** (`:28-46`):
  - `{project_id: "project_<sha256(realpath(root))>", project_root, grants[]}`;
  - grants are `lifetime: z.literal("project")`.
- **OBSERVED. Reads.** Same-handle owner-only open. The read is rejected if `project_id` or `project_root` does not match the canonical root (`:88-125`). Unsupported where `process.getuid` is undefined, which means Windows (`:96-97`).
- **OBSERVED. Writes.** `write-file-atomic` with mode `0600` and read-back verification (`:151-168`), under a `.lock` file (`:191-230`).
- **OBSERVED. No issuer.** `writeProjectPermissionStore` (`:128`) had **no non-test caller** at the parent: `git grep` finds only the definition and three test files. `revokeProjectPermissionGrant` (`:171-189`) was the only production writer. The original 2026-07-14 `policy` command also had only `status|list|explain|revoke` (`git show 190efcaf1:src/cliPolicyCommands.ts`). The project-lifetime *elicited* grant was removed on 2026-07-17 over "authority and TOCTOU gaps" (#96 comment).
- **INFERRED.** Project grants could be listed, explained and revoked, but never created except by hand-writing JSON that contains a SHA-256 of the realpath.

### 3.4 Stateful authority and connection overlays (`src/application/PermissionAuthority.ts@4f099a10f`)

- **OBSERVED. One authority object** owns evaluation, consumption, reload and revoke (`:133-152`).
  - `createConnectionAuthority()` (`:155-161`) creates a per-MCP-connection overlay holding `once` and `session` grants. The overlay is evaluated against the configured authority's live ceilings.
  - `createServer` builds one overlay per connection and clears it on `onclose` (`src/server/createServer.ts:163-174@4f099a10f`).
  - Shutdown clears session grants (`src/main/shutdown.ts:36@4f099a10f`).
- **OBSERVED. `authorize`** (`:164-174`) canonicalizes, evaluates and consumes a `once` grant on the authority that *supplied* it (`owner.policy = consumePermission(...)`). **`explain`** (`:177-189`) is the same evaluation without consumption. It backs `rea policy explain` and the plan modes of replay and managed-runtime.
- **OBSERVED. Canonicalization** (`:88-105`) uses `realpath`. A write request to a not-yet-existing path falls back to `realpath(dirname)/basename`. A missing *read* path is an error.
- **OBSERVED. Remediation selection** (`:282-304`) carries the comment added in #552 (`:285-289`): "Only a genuine ceiling violation can be fixed by widening configuration. When the scope is inside the ceiling but no grant was ever issued, the `missing` payload describes what a *grant* must cover...".
- **OBSERVED. Reload.** `replaceConfiguredPolicy` (`:342-367`) atomically swaps ceilings plus administrator and project grants on SIGHUP (`src/main/reload.ts:47-48@4f099a10f`).
- **OBSERVED. Error-reason erasure.** `addGrantsToPolicy` (`:408-422`) wraps an "outside ceiling" grant error as `PermissionPathError(grant_id, "io")`, so a policy-shape error is reported as an I/O failure.

### 3.5 Error contract (`src/domain/errors.ts@4f099a10f`)

- **OBSERVED. Category.** `permission_required` is a top-level error category (`:493`, `:503`). The projected remediation is `{action, restart_required, elicitation_supported?}` (`:513-517`).
- **OBSERVED. Remediation.** `PermissionRemediation = "configure" | "elicit" | "restart" | "grant"` (`:530-534`). The doc comment says `grant` is "deliberately distinct from `configure`".
- **OBSERVED. Error class.** `PermissionRequiredError` carries `requested`, `missing`, `ceiling` and `remediation` (`:545-559`). The user-facing action strings are in `src/domain/analysisErrorPresentation.ts:44-55@4f099a10f`. The `grant` text reads: "issue a grant for it (a project grant, or the documented auto-grant setting for an unattended environment)". Neither route was a supported interactive flow.
- **OBSERVED. Content-bound approval.** `ReplayPlanStaleError(approvedDigest, actualDigest)` (`:562-570`) was the separate content-bound approval: "Review the rebuilt replay plan and explicitly approve its new digest."

### 3.6 Per-call approval fields (tool inputs)

All OBSERVED at `@4f099a10f`:

- **Native UI** (`src/domain/nativeUiObservation.ts:6,16-17`): `observation_approved: z.literal(true)`, `actions_approved: z.literal(true)` and `restore: z.literal("leave-as-is")`. These are single-value fields: the only valid input is the constant.
- **Artifact integrity** (`src/contracts/artifactToolContracts.ts:10-19`): a union of `{integrity_policy:"fail", integrity_continue_approved: z.literal(false)}` and `{integrity_policy:"record-and-continue", integrity_continue_approved: z.literal(true)}`. It was gated as well by `REA_ARTIFACT_INTEGRITY_CONTINUE_ENABLED` (`src/application/ArtifactInventory/policy.ts:27-37`), making three keys for one choice.
- **Native mount** (`artifactToolContracts.ts:22`): `native_mount_approved: z.boolean()`. It was also gated by `REA_ARTIFACT_NATIVE_MOUNT_ENABLED` (`policy.ts:13-24`) and by a `native_mount` permission request (`src/application/DirectAnalysis.ts:131-148`).
- **Plan approval.** `approved_plan_sha256` for Node runtime characterization (`src/domain/nodeRuntimeCharacterization.ts:65`, checked at `src/application/NodeRuntimeCharacterizationService.ts:85`). `plan_digest` for controlled replay (`src/domain/javascriptReplay.ts:155,203`). The plan object itself carried `authority: {preparation_approved: z.literal(true), execution_approved: z.literal(false), ...}` (`src/domain/runtimeCharacterization.ts:60-65`).
- **Browser capture facets** (`f0069cbee`; `docs/browser-observation.md` at that commit): `json_body_schema_approved`, `websocket_shape_approved`, `--screenshot-approved`, `include_storage_fingerprints`, among others.
- **Schema descriptions.** `src/contracts/toolSchemaMetadata.ts:7,115-116` auto-described every `*_approved` field as "Explicit operator approval for ...".
- **Before and after.** In non-test `src/`, `_approved` tokens went from 40 to 17 across #555. The 17 left are `not_approved` completeness statuses (§8). `z.literal(true)` in domain and contracts went from 37 to 27.
- **INFERRED.** In MCP, the *agent* fills every one of these fields. To an MCP caller, "explicit operator approval" was indistinguishable from a model typing `true`.

### 3.7 Duplicate, parallel policy layers for process capture

OBSERVED at `@4f099a10f`. One `capture_process_scenario` call passed three checks, all derived from the same env vars:

1. **`PermissionAuthority.authorize(processCapturePermissionRequest(scenario))`.** The request has `roots = [working_directory, ...filesystem_roots]`, `executables = [executable]`, env names `= keys(environment) + inherit_environment`, and `network = "external"` whenever `network_access === "host"`, which was always. The operation identity is `capture_process_scenario:${executable}` (`src/application/ProcessCapturePermission.ts:5-18`).
2. **`authorizeProcessScenario(scenario, policy)`.** A lexical string-prefix check against `ProcessExecutionPolicy` with its own reason strings, such as "executable is outside approved roots" (`src/domain/processScenario.ts:443-495`, called at `src/application/ProcessCaptureLifecycle.ts:441`).
3. **`assertRealPathAuthority`.** A realpath re-check that throws `ProcessCaptureError` (`src/application/ProcessCaptureAuthority.ts:14-41`, called at `ProcessCaptureLifecycle.ts:447`). It uses `canonicalizeConfiguredRoots`, which **silently drops missing roots** (`src/application/ConfiguredRoots.ts:3-16`).

Layer 1's ceiling canonicalization **fails the whole policy load** on a missing read root (`PermissionAuthority.ts:93-97`).

**INFERRED.** The same stale entry in `REA_PROCESS_WORKING_ROOTS_JSON` was tolerated by layer 3 and fatal to layer 1. The three layers gave the same denial three different vocabularies.

## 4. Where checks ran (OBSERVED, `@4f099a10f`)

| Surface | Site | Capability | Notes |
|---|---|---|---|
| MCP process capture | `src/server/registerSessionTools.ts:113-131` | `process_capture` | Uses `authorizeProcessCaptureWithElicitation` when the elicitation helper is wired, else plain `authorize`. **The check is skipped entirely when `permissionAuthority === undefined`** (`:114`). That seam serves embedders and tests |
| CLI process capture | `src/application/ProcessCli.ts:51-59` | `process_capture` | Loads a fresh authority per command (`loadConfiguredPermissionAuthority`). No elicitation and no interactive grant prompt; only the structured error |
| Application: browser | `src/application/BrowserObservationService.ts:228-249` | `browser_observe` | Origins = `[cdp_endpoint, ...allowed_origins]`. Network derived by `browserNetworkScope` (`src/config/permissions.ts:32-40`). Missing authority → `capability_unavailable` |
| Application: others | `BrowserScenarioCaptureService.ts:36`, `ElectronObservationService.ts:85`, `ElectronActiveObservationService.ts:80`, `JavaScriptRuntimeObservationService.ts:85`, `JavaScriptReplayService.ts:167-168` (explain on plan, authorize on execute), `ManagedRuntimeCorrelationService.ts:89` (explain only) | per family | Same `authorize` call shape |
| MCP + direct analysis: artifacts | `src/server/registerEvidenceTools.ts:56-75`; `src/application/DirectAnalysis.ts:111-149` | `artifact_extract`, `native_mount` | Extraction requests `roots:[tmpdir()]`, `operation_identity:"extract_artifact:temporary_output"` (`src/application/ArtifactExtractionDestination.ts:14-22`). REA itself chose that destination, against a `"/"` ceiling that was always auto-granted |
| Tool discovery | `src/server/sessionAvailabilityPolicy.ts` (pre-#555); `src/application/CapabilityClientRequirements.ts` | — | `tools/list` hid tools whose family env gate was off. `capture_process_scenario` listed `elicitation_form` as an *optional* client feature |
| CLI admin | `src/cliPolicyCommands.ts:43-114` | all | `status` (ceilings + store path), `list` (project grants), `explain <cap> --root --executable --origins --network --mount --write` (dry run), `revoke <id>` (TTY `[y/N]` prompt or `--yes`, `:28-40`, `:191-204`) |

INFERRED: there was **one evaluator shared by CLI and MCP**, which is good. But the *call* lived in each adapter and service, and the "no authority → skip" seam in the MCP adapter (`registerSessionTools.ts:114`) made the gate optional at the composition root. The domain checks in §3.7 were the only mandatory ones.

## 5. MCP elicitation for process capture (`e6cfce390`, `src/server/ProcessCaptureElicitation.ts@4f099a10f`)

All items below are OBSERVED unless marked.

- **Protocol.** The continuation is not `elicitation/create`. It is the SDK's `inputRequired`/`inputResponse` multi-round tool result. Policy (`:24-29`): `protocolVersions: ["2026-07-28"]`, `stateTtlSeconds: 600`, `roundTimeoutMs: 600_000`, `maxConsumedNonces: 4096`.
  - `docs/process-capture.md:46-54@4f099a10f`: "On the pinned `@modelcontextprotocol/server` 2.0.0 runtime, this is the official `2026-07-28` `inputRequired` continuation ... The older push-style `elicitation/create` path is not used for this protocol revision."
  - SECONDARY ([MCP 2026-07-28 client concepts](https://modelcontextprotocol.io/docs/2026-07-28/learn/client-concepts.md); [Ruby SDK elicitation](https://ruby.sdk.modelcontextprotocol.io/server/elicitation/)): in this revision, server-initiated requests are not allowed. Elicitation is embedded in an input-required result and the client retries with answers. Capabilities travel per request in `_meta`.
- **Support gate** (`src/server/createServer.ts:104-126`). Elicitation is offered only if the request envelope's protocol version is `2026-07-28` **and** the client capabilities contain `elicitation.form`. Otherwise it falls back to the typed `permission_required` error with `remediation: "configure"`.
- **Preconditions** (`:84-96`). The initial `authorize` must fail with `PermissionRequiredError`, `remediation === "elicit"`, a non-null ceiling, and the request fully inside the ceiling (`isPermissionRequestWithinCeiling`). Elicitation can never widen a ceiling, which `docs/mcp-contracts.md:121-124@4f099a10f` also states.
- **Round 1** (`:130-153`). The server mints signed request state `{nonce: randomUUID(), request: <exact canonical PermissionRequest>}`. The SDK seals it with a per-connection 32-byte key (`createServer.ts:165-169`). The server returns `inputRequired.elicit` with a form schema of `{lifetime: "once"|"session"}`, **default `"session"`** (`:31-33`). `$schema` and `additionalProperties` are stripped to fit MCP's reduced JSON Schema subset (`:35-40`).
- **Prompt text** (`:233-243`): "Allow REA to run this exact process-capture scope?", followed by executable, filesystem roots, environment names, origins, network and mount. **The prompt does not include the command arguments, the terminal input script or the environment values.**
- **Round 2** (`:98-128`). The state must parse and equal the freshly denied request field by field (`sameRequest`, `:186-197`). The nonce is consumed on any `elicit` response; replay or table overflow → deny (`consumeNonce`, `:214-231`). `accept` → `grantAndAuthorize`. `decline`/`cancel` → the original denial.
- **Grant and authorize** (`:156-184`). The grant is `elicited:<uuid>`. For `once`, the operation identity is the request's; for `session` it is `null`, so the grant matches *any* subset request on that connection. The grant is added to the connection overlay and authorization re-runs. If the re-authorization fails or throws, the grant is revoked.
- **Reach** (`docs/process-capture.md:33-44@4f099a10f`). The flow requires `REA_PROCESS_CAPTURE_AUTO_GRANT=false`, the "prerequisite mode for connection-scoped MCP elicitation". Under the default `true`, the administrator grant already authorized everything within the ceiling, so elicitation was unreachable.
- **Clients:**
  - At introduction, the pinned SDK "negotiates the older protocol path, so the interactive flow remains fail-closed" (e6cfce390 docs diff). The 2026-07-17 #96 comment adds: "attempting the legacy automatic shim hung in integration testing".
  - REA's setup targets were `claude_code`, `claude_desktop`, `codex`, `cursor`, `gemini_cli`, `windsurf` and `devin` (`src/application/SupportedClients.ts:15-62@4f099a10f`). No doc or test records any of them completing the round trip.
  - Tests drove `authorizeProcessCaptureWithElicitation` with hand-built `mcpReq` objects (`tests/boundary/mcp/elicitationGrantsHarness.ts:33-43@4f099a10f`).
  - #389 ("run real modern MCP client tamper/expiry proof") was closed as superseded.
  - UNKNOWN: which shipping client, if any, advertised `elicitation.form` on `2026-07-28` during August–September 2026.
  - SECONDARY: one blog's September 2026 table says the TypeScript and Python v2 SDKs support form and URL modes with automatic retry, while the Anthropic Messages-API MCP connector supports tool calls only ([nango.dev](https://nango.dev/blog/mcp-elicitation-explained)).
- **What survives today.** Capability *detection* only. `src/server/mcpClientMetadata.ts:58-67@HEAD` reports `elicitation_form` and `elicitation_url` in `binary_session`; nothing consumes them. `bb6a58c2a` (2026-09-29) had already reduced the server-side elicitation typing to the SDK's canonical type.

## 6. Reasons for removal

### 6.1 Stated (OBSERVED)

- **Commit body of `b5e98915c`:** "Remove REA permission grants, scope ceilings, approval confirmations, and inert settings. Preserve selected local evidence, accept caller-chosen commands and paths, and report actual runtime failures." From the BREAKING footer: "Process capture inherits the host environment and accepts command names".
- **PR #555 body:**
  - Problem: "Browser capture could deny a request whose scope already fit its ceiling. Fixed literals and approval booleans forced callers to restate implementation behavior."
  - Principle: "Tool selection expresses intent. Validate actual inputs and protocol requirements ... report real host/provider failures."
  - Security review: "REA grant restrictions are removed for local tools. Actual OS permissions, target binding, protocol constraints, lifecycle cleanup, and replay confinement remain." (#572 removed replay an hour later.)
- **#96 closing comment:** "Agents invoke local tools directly; the selected operation expresses intent, and execution uses the current user's OS permissions ... Setup approval remains a separate installation workflow."
- **#545 closing comment:** the misleading diagnostic is now "structurally absent" rather than corrected.
- **`AGENTS.md` diff in `b5e98915c`:**
  - Removed: "A configured permission ceiling is distinct from an active grant".
  - Added: "Leave meaningful target, action, capture, and output choices to the agent. A selected operation already expresses intent; do not require approval booleans or repeated permission declarations. Trace each setting to its consumer: remove ignored options and single-value confirmations, derive built-in lifecycle behavior ... Report actual effects and limitations where they help interpret results rather than asking callers to restate them."
  - Privacy changed too: "Do not guess that local evidence is secret from environment-variable names, argument names, or text patterns." `SensitiveTextCapture.ts` (regex redaction) was deleted.
- **`docs/tool-design.md:74-75`:** "truthfully in the contract and permission boundary" became "truthfully in the contract and enforce the target and lifecycle named in the request."
- **`docs/mcp-contracts.md`:** "## Permission policy" (around 60 lines) was replaced by "## Request scope and local effects". At HEAD (`docs/mcp-contracts.md:1298-1311`) it says: "REA runs the declared request directly and does not infer a broader target or action from it. ... REA does not require permission grants or per-call approval flags. Setup still prints its plan and requires confirmation ... MCP clients control their own confirmation UI. Tool annotations describe effects and are hints, not authorization controls."
- **`docs/process-capture.md`:** `filesystem_roots` became `filesystem_observation_paths`, with a new sentence: "Filesystem observation paths select what REA records; they do not constrain what the child process can read or write."
- **ADR 0002 got a banner:** "REA's separate permission-policy, grant, and per-call approval workflow was later removed."

### 6.2 Inferred

1. **Consent covered parameters that did not bind the process.** The process ran as the user with no sandbox (`docs/process-capture.md:56-58@4f099a10f`: "not a security sandbox"). The status of each dimension:
   - roots: did not confine the child (stated after removal);
   - network: always `"host"` (`src/domain/processScenario.ts:147@4f099a10f`), because "adding a loopback enum would be a false security claim" (#96);
   - executable: checked, but the exe can spawn anything;
   - environment names: actually enforced, because REA built the child environment.

   A grant over four mostly unenforced dimensions was closer to ceremony than to a boundary. The rename to `filesystem_observation_paths` is the maintainers admitting this.
2. **Agent-supplied approvals could not resist a confused deputy.** `observation_approved: z.literal(true)` and `actions_approved: z.literal(true)` were filled by the model. Against prompt injection they protect nothing (`survey-safety-rails...` §3.5), and they cost tokens and schema complexity.
3. **The default configuration turned the ceiling into the grant.** That made the two-level design inert for most users. Where it was not inert (browser and Electron automation), it was unusable, because there was no issuer.
4. **Most gates guarded caller-chosen or REA-chosen local paths.** Examples: Evidence read/write, snapshots, reference sources, and the extraction tmpdir. For a local-only tool running with the user's authority these were pure friction, and the first removal wave (2026-09-29) targeted exactly them.
5. **Maintenance cost exceeded the security return.** In 83 days the system needed:
   - atomic reloads, revoke confirmation and cross-connection isolation;
   - nonce tables and signed state, with grant rollback;
   - store locking with same-handle `O_NOFOLLOW`;
   - Windows DACL follow-ups (#388), a `DeferredFileAuthorization` workaround, and the #552 remediation fix.

   It produced no verified end-user interactive flow.
6. **Two other tendencies pointed the same way.** The project's general "remove arbitrary ceilings" tendency (`5f73cb061`, `2f08c7c93`, `5dd3b4655`, `9aec6908b`, `abf7c1142`, `f2dc749c8`, all 2026-09-29) and its agent-first direction (#32) both favoured the agent composing actions over plan/approve ceremony. #572 removed prepare/execute flows for the same reason.

## 7. Failure modes the maintainers hit

1. **Ceiling-vs-grant confusion** (#545, fixed by #552, then moot).
   - What happened: a Windows user had a correct `REA_BROWSER_SCENARIO_*` ceiling and `AUTO_GRANT` unset, and was still denied. `rea policy explain browser_automate` with an *empty* request returned `missing: {}` alongside "Add the exact missing scope beneath the administrator ceiling". With a real scope, every `missing` entry also appeared in the ceiling, and `network: loopback` was reported missing beneath a ceiling of `external`.
   - Mechanism: `missing` was computed against the empty candidate-grant set (`permissionPolicy.ts:173`), while the advice was hard-coded for ceilings. The only CLI-reachable fix was `AUTO_GRANT=true`, which the docs described as appropriate for "a trusted unattended environment".
   - Lesson: **a denial must name the layer that denied it, and must be computed against that layer.** Every remediation must point to a working issuer.
2. **No issuer for a grant type.** Project grants could be listed, explained and revoked, but not created (§3.3).
3. **Inert or unsatisfiable settings:**
   - `REA_PROCESS_ALLOW_EXTERNAL_NETWORK` was required, because `network_access` could only be `"host"`;
   - `artifact_extract` had a ceiling of `"/"` and an automatic grant, checked against a REA-chosen tmpdir, so it could not deny under any config;
   - `restore: z.literal("leave-as-is")` and `execution_approved: z.literal(false)` each had exactly one legal value;
   - `REA_REFERENCE_SECRET_PATTERNS_JSON` and `secret_aliases` were "unused capture/redaction settings" (PR #555).
4. **Three-key approvals for one choice.** Integrity-continue needed an env var, `integrity_policy` and `integrity_continue_approved: true`. Native mount needed an env var, `native_mount_approved` and a `native_mount` grant. After #555 each is one meaningful field, or none.
5. **Layer duplication with divergent semantics** (§3.7): different canonicalization (missing roots dropped vs fatal) and different reason vocabularies.
6. **Containment semantics nobody could see** (#567). An executables entry that is a directory authorizes every binary beneath it, including "anything an attacker can drop there".
7. **Platform assumptions in the boundary schema** (#548). `startsWith("/")` meant no process scenario could be written on Windows. The #552 fix accepts POSIX, drive and UNC paths on every host, because "a boundary schema must not vary with its evaluator". The project store also refused to run without `process.getuid` (`ProjectPermissionStore.ts:96-97`).
8. **Protocol and runtime mismatch.** The elicitation design needed a newer MCP revision than the pinned runtime negotiated. The fallback shim hung (#96). The feature shipped "fail-closed" and stayed effectively dormant.
9. **Concurrency and TOCTOU surfaced during design review** (#96, 2026-07-17):
   - process-global once/session grants that other connections could consume;
   - publishing authority before a durable commit;
   - SIGHUP changing the store under a live signed continuation;
   - path-based store checks.

   All were fixed or descoped, at a significant cost in code.
10. **Consent text described scope, not action.** The elicitation prompt omitted arguments (§5), and the `once` identity was `capture_process_scenario:<executable>`. A user approving `node` approved any `node -e ...` for that one call; with `session` (the default), for the rest of the connection.
11. **Error-reason erasure in plumbing.** A grant outside the ceiling surfaced as an `io` path error (`PermissionAuthority.ts:408-422`). `PermissionFailure.ts:8-13` turned path errors into `AnalysisProtocolError`.
12. **Gates bypassed workflows.** `f840e7074` "defer cache write grants" (`DeferredFileAuthorization.ts`, later deleted in `1b0ab75a8`) existed because a cache write should not require a grant the user never asked for.

## 8. What survived in another form (OBSERVED at HEAD `3484432fb` unless noted)

| Old piece | Survivor | Where |
|---|---|---|
| Per-capability scope vocabulary | **`ToolEffects`**, a static, per-tool, truthful effect record: `mutatesTarget`, `mutatesSession`, `writesFilesystem`, `launchesProcess`, `accessesNetwork`, `changesUiState`, `mayDiscardData`, `executesForeignCode`, `mutatesDevice`, `idempotent`. MCP hints are derived mechanically from it. `2919def82` re-audited effects as the *replacement* for grants: "Do not mislabel session mutations as read-only to suppress prompts." | `src/contracts/toolEffects.ts:2-22`, `:765-786` (since `ee1cd4040`, 2026-07-16, so it coexisted with grants) |
| "Approval" of capture facets | Opt-in facet flags stay (e.g. `include_storage_keys`). Excluded facets are recorded as completeness `not_approved`, so absence is explicit and never read as "nothing there" | `src/domain/browserObservationSchemas.ts:123,136,257`; `src/browser/CdpCaptureEvents.ts:61-65` |
| Confirmation | **Setup only**: plan, disclose, `SetupConfirmationDecision {approved, selectedActionIds, cancelled?}` | `src/application/SetupTypes.ts:216-275`; `AGENTS.md` ("`rea setup` must print its planned changes and require approval") |
| Policy-based tool hiding | Host-based availability: `processCaptureEnabled: platform !== "win32"`, `controlledReplayEnabled: isSupportedControlledReplayHost(...)` | `src/server/sessionAvailabilityPolicy.ts` (`b5e98915c` diff) |
| Permission denial category | Host OS denials (Accessibility, Screen Recording, mount) "reported at the operation that needs them" with no broad fallback | `docs/mcp-contracts.md:1298-1306`; `rea-desktop-...md` §11 item 4 |
| Scope = authority | **Request scope = intent**: "Each tool request names the target and lifecycle it will use ... does not infer a broader target or action" | `docs/mcp-contracts.md:1298-1305` |
| Grant identity binding | Target identity binding (exe SHA-256, PID plus start time, window ID, AX geometry) and ownership-scoped termination | `rea-desktop-...md` §11 items 5-6 (not repeated here) |
| Redaction patterns | Redaction of transport credentials, plus caller-marked `sensitive: true` input events with byte counts | `src/domain/process/processCapture.ts:50-63`; `AGENTS.md` privacy paragraph |
| `approved_plan_sha256` / `plan_digest` | **Gone.** `b5e98915c` renamed it to `plan_sha256`; #572 removed the prepare/execute and replay tools. No `plan_digest` in `src/` at HEAD | `git grep` at HEAD |
| Elicitation | Capability detection only | `src/server/mcpClientMetadata.ts:58-67` |

## 9. Judgment: what an observe+drive authority model should keep, change or drop

This section is INFERRED, grounded in §3-§8. Tier names follow `survey-safety-rails-and-authorization-models.md` §9: observe < launch < drive < attach < inject < mutate-system.

### Why REA's removal does not carry over unchanged

REA's governed operations were mostly *analysis* of user-selected artifacts, plus observation and scenario runs that the agent composed and the MCP client confirmed. agentic-anything's drive tier is different in three ways:

- it sends input into live apps that render untrusted content, so the confused-deputy and injection exposure is higher;
- its effects are often irreversible (sending a message, deleting a record);
- it must run headless, where no client UI is present.

REA's own remaining stance, "MCP clients control their own confirmation UI", assumes a human-in-the-loop client. That cannot be assumed for drive. REA's evidence still shows *how not to gate*.

### Keep

| Piece | Why | agentic-anything tier |
|---|---|---|
| One pure evaluator shared by CLI and MCP (`permissionPolicy.ts`, `PermissionAuthority.explain/authorize`) | Same decision everywhere. `explain` gives a dry run for free | all gated tiers |
| Typed decision with distinct reasons (`outside_ceiling` / `grant_required` / `revoked_or_consumed` / `expired`) and the #552 rule | Fixes the #545 class of bug at the type level | all |
| Canonicalize at use time (realpath; write-path parent fallback) and check identity at use time | TOCTOU defence; applies equally to app identity (bundle ID + code-signature hash + PID start time) | launch, drive, attach |
| Connection-owned `once`/`session` overlays, cleared on disconnect, invisible across connections; `once` consumed atomically on the supplying authority | Correctly solved isolation problems (#96 2026-07-17) | drive, attach |
| Elicitation can never widen the operator ceiling; decline and cancel = deny; non-capable clients get a typed, actionable error and never hang | Sound fail-closed rules | drive+, when the client supports it |
| Signed continuation state bound to the exact request, with TTL and nonce table | Defeats replay of an accept across requests | drive+ |
| Content-bound plan approval (`plan_digest` + `plan_stale` on rebuild; ADR 0002: "cannot be represented by a bare Boolean alone"; "may not auto-approve because the caller requested JSON output or because permission elicitation is supported") | The one approval form REA designed that an agent cannot fake by typing `true` | inject, mutate-system, batched irreversible drive |
| Values never stored in grants or denials (names and paths only) | Privacy; grants are audit records | all |
| `ToolEffects` as static declaration, with hints derived | Already recommended elsewhere; REA chose it as the replacement | all |
| Setup-style "print plan, require approval" for installs and config changes | Survived because a human is present and the effect is persistent | mutate-system, setup |

### Change

| REA did | Change to | Tier |
|---|---|---|
| Consent described a *scope* (exe, roots, env names) and the once identity was `tool:executable` | Consent and grants bind the **full action digest**: tool, target identity, locator, arguments/text, and predicted effect class. The prompt shows the action, not a summary | drive, attach, inject |
| Scope dimensions declared but unenforced (roots, network) | Every dimension carries `enforcement: "enforced" \| "declared-only" \| "not-applicable"`, after REA's `WindowsAuthority` `proof` field. Declared-only dimensions are never shown as constraints in prompts | all |
| Ceilings auto-promoted to grants by default; per-family `AUTO_GRANT` | Make the default explicit and named: `allowlist` (ceiling = standing grant) vs `ask-within` (ceiling bounds what can be granted). Per tier, not per family: observe = allowlist; drive+ = ask-within by default | per tier |
| No grant issuer; project store hand-written | Ship `grant`, `list`, `explain` and `revoke` together. Every remediation names a command that exists. Test "issue → use → revoke" end to end | M3 |
| Session grant is the default elicitation lifetime | Default `once` for drive irreversible, attach and inject; `session` only for reversible drive within one app identity | drive+ |
| Three overlapping process checks with divergent semantics | One enforcement point after canonicalization. Domain validation checks shape only, never authority | — |
| `permissionAuthority?` optional at the MCP adapter (gate skipped when absent) | The authority is a required constructor dependency. Tests inject an allow-all authority explicitly | — |
| Elicitation built for a protocol revision the pinned runtime could not negotiate | Gate the design on a real client round-trip test in CI (a recorded client, or the SDK client at the pinned version), plus an out-of-band confirmation channel (local CLI/TTY or OS notification) for clients without form elicitation | drive+ |
| Directory containment for executables, undocumented (#567) | Exact identity for executables and apps (path + hash or signature); containment only for data roots, labelled in schema and prompt | launch |

### Drop

- **Agent-fillable approval booleans and single-value fields** (`*_approved: z.literal(true)`, `restore: z.literal(...)`). They give no protection against a confused deputy and are "repeated permission declarations".
- **Per-family enable env vars** (about 30). Authority belongs in a policy file with tiers, not in process env that needs a client restart.
- **Gates on REA-chosen or caller-chosen local reads and outputs** (Evidence, snapshot, extraction tmpdir). For observe-tier tools running as the user they add only friction. REA's 2026-09-29 wave confirms this.
- **Network as a grantable dimension** until a real per-OS loopback or egress backend exists. Until then, report it as a declared effect (`accessesNetwork`) only.
- **Regex secret guessing as a privacy control** (`SensitiveTextCapture.ts`). Keep transport-credential redaction and caller-marked sensitivity instead (AGENTS.md).

## What to distill for agentic-anything

1. **Write the authority ADR around the action record, not the capability scope.**
   - Form: `ActionRequest = {tier, tool, target_identity{app_id, code_hash, pid, start_time, window}, locator, payload_digest, effect_class (read | ui_state | data_mutating | irreversible | external), declared_dimensions[{name, value, enforcement}]}`.
   - Grants and approvals reference `payload_digest` (exact) or a pattern over `{tier, app_id, effect_class}` (standing).
   - This fixes REA failure modes 7.1, 7.6 and 7.10 at the data-model level.
2. **Use a four-reason denial union from day one:** `outside_ceiling`, `grant_absent`, `grant_revoked_or_consumed`, `grant_expired`. Add `enforcement_unavailable` for a dimension that the request needs enforced but that is only declarable on this host.
   - `missing` is computed against the layer that denied.
   - Each reason maps to exactly one remediation command, and a test asserts that the command exists in the CLI catalog. This is the #545/#552 lesson as a CI check.
3. **Ship the grant lifecycle as one unit:** `aa grant` (interactive or with `--yes`), `aa grants list`, `aa policy explain <action.json>` (dry run with the same evaluator), `aa grants revoke <id>`. Add no grant lifetime without a production writer: a lint or test fails if a lifetime has zero non-test producers.
4. **Default modes per tier.**
   - observe: allowlisted app identities, no prompt.
   - launch: allowlist.
   - drive: reversible actions run under a session grant per app identity; irreversible or external actions need a per-action approval bound to the payload digest.
   - attach, inject and mutate-system: a content-bound plan digest, with `plan_stale` on rebuild, never a session grant.
   - Headless (`dontAsk`): anything above reversible drive is denied with `grant_absent`.
5. **Approval provenance must come from outside the agent.** An approval is valid only if it arrived through one of: MCP form elicitation (`2026-07-28` `inputRequired`, signed state, nonce, TTL, as in REA's `ProcessCaptureElicitation.ts`); a local out-of-band channel (CLI prompt or OS notification); or a pre-issued grant file written by `aa grant`. Never accept an approval through a tool argument. Record `approval_source` in the action log.
6. **Make honesty about enforcement a contract field.** Each declared dimension states whether the host enforces it. Prompts and Evidence render declared-only dimensions as "recorded, not enforced". This follows REA's `filesystem_observation_paths` rename and its `WindowsAuthority` proof status.
7. **Use one enforcement point and no optional gate.** The authority is a required dependency of the tool executor, after canonicalization and identity re-check and immediately before side effects. Adapters (CLI, MCP) never call `authorize` themselves.
8. **Bind connection-scoped grants as REA did.** Per-connection overlay; atomic consume-once on the supplying overlay; clear on disconnect; reload ceilings atomically and keep live overlays evaluated against the new ceilings. These parts worked; copy their tests (cross-connection, concurrent once, reload collision).
9. **Allow no setting without a consumer.** Adopt REA's AGENTS.md rule ("Trace each setting to its consumer; remove ignored options and single-value confirmations") as a CI check over the policy schema and tool input schemas. Any `z.literal` field in tool input fails review unless it is a discriminant.
10. **Prove the confirmation path against a real client before claiming it.** Use a CI lane with the pinned SDK client negotiating the target protocol, plus a fixture that records a real client's capabilities. Without it, document the feature as "fail-closed, not interactive". REA's flow never left that state.
11. **Keep observe cheap.** Do not gate reads of caller-selected files or owned outputs. REA's first removal wave shows that gating these produces workarounds (`DeferredFileAuthorization`) and user bypass (`AUTO_GRANT=true`).

## Open questions

1. Which shipping MCP clients (Claude Code, Claude Desktop, Codex, Cursor, Gemini CLI) negotiate `2026-07-28` and advertise `elicitation.form` as of November 2026? REA never recorded this. agentic-anything needs a capability matrix before relying on in-protocol confirmation.
2. Should session grants for drive be keyed to app identity alone, or to app identity plus window or document? A per-document key resists injection better but prompts more often.
3. How should a payload digest be computed for GUI actions whose effect depends on live UI state (the same click on a different focused element)? Should the digest include a pre-action observation hash, so that drift between approval and execution yields `plan_stale`?
4. Is there any per-OS mechanism that makes "network: none" or "filesystem roots" enforceable for a *driven GUI app* without a VM? If not, should agentic-anything drop these dimensions entirely for drive and keep them only for the container or VM lane?
5. Should agentic-anything accept REA's position that the MCP client's own confirmation UI is enough for reversible drive, and gate only irreversible actions server-side? What client behaviour (auto-approve modes, classifier approvals) would invalidate that?
6. Can REA's `ToolEffects` table be extended to per-step effect classes (G6 in the desktop report) and still be derived mechanically, or does per-step classification inherently require adapter-specific heuristics?
7. Is the right persistence model for standing grants a project file that the user edits (REA-style, with store locking and handle checks), or OS-native consent records (TCC, portal `restore_token`)? This matters most on Windows, where REA's store refused to run.

## Sources

- REA checkout `/Users/nhanclie/rea`. Commits `190efcaf1`, `9526389f8`, `7a4cfb4fb`, `6397e5416`, `3ae7109fa`, `49e843e8f`, `f840e7074`, `e6cfce390`, `2e5b44365`, `f0069cbee`, `035e1144b`, `8f229ede7`, `0d96275b6`, `c34efca4c`, `81996df7a`, `7eb1a9500`, `1b0ab75a8`, `bb6a58c2a`, `e793e124a`, `53fe7d6d3`, `b5e98915c` and its parent `4f099a10f`, `2919def82`. HEAD `3484432fb`.
- GitHub (read via `gh`): morluto/rea [#96](https://github.com/morluto/rea/issues/96), [#114](https://github.com/morluto/rea/issues/114), [#388](https://github.com/morluto/rea/issues/388), [#389](https://github.com/morluto/rea/issues/389), [#545](https://github.com/morluto/rea/issues/545), [#548](https://github.com/morluto/rea/issues/548), [#567](https://github.com/morluto/rea/issues/567), [PR #555](https://github.com/morluto/rea/pull/555), [PR #572](https://github.com/morluto/rea/pull/572).
- SECONDARY: [MCP 2026-07-28 client concepts](https://modelcontextprotocol.io/docs/2026-07-28/learn/client-concepts.md); [Ruby SDK elicitation](https://ruby.sdk.modelcontextprotocol.io/server/elicitation/); [nango.dev, MCP elicitation explained](https://nango.dev/blog/mcp-elicitation-explained).
- Related reports, not repeated here: `survey-safety-rails-and-authorization-models.md` (§3.3, §4, §9), `rea-runtime-observation-surfaces.md`, `rea-desktop-applications-and-agent-environments.md` (§11-12), `rea-contracts-and-evidence-model.md` (action-record proposal).
