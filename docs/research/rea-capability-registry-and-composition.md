# REA capability registry and composition

Research date: 2026-11-05

Area: `capability-registry-and-composition` (full report; replaces the lost original, whose structured summary survives at `docs/research/salvaged/capability-registry-and-composition.json`).
Source: the owner's local REA fork at `/Users/nhanclie/rea`, HEAD `3484432fb` (read only; nothing built or run; `dist/` and `node_modules/` absent, so generated artifacts were not inspected and no tests were executed).
All `src/...`, `scripts/...`, `tests/...`, `docs/...` paths are relative to that checkout.

Evidence legend: **OBSERVED** = read in the REA source/docs (cited `path:line`). **SECONDARY** = REA's own verification report or a third-party summary, not re-derived here. **INFERRED** = my interpretation. **UNKNOWN** = not determinable from what was read.

Related reports (not repeated here):
- `docs/research/rea-contracts-and-evidence-model.md`: ToolContract shape, Evidence record, error union, retained summaries.
- `docs/research/rea-mcp-server-adapter.md`: EvidenceMcpServer, transport interception, deferred runtime internals, server identity, `binary_session` discovery.
- `docs/research/salvaged/engine-bridges-and-process-supervision.json`: owned processes and bridges behind providers.

---

## Summary

- REA has **no single registry object**. "The registry" is a set of statically typed tables that are cross-checked at three different times: when TypeScript compiles, when the host is built, and when the catalog generators and tests run. The tables are: the contract inventory `TOOL_CONTRACTS` (`src/contracts/toolContracts.ts:48-94`); the contract-array owners `TOOL_CONTRACT_GROUPS`, 47 groups (`src/contracts/toolFamilies.ts:52-100`); 18 advertising families (`toolFamilies.ts:116-245`, ids in `src/contracts/toolFamilyIds.ts:8-27`); the effect audit `TOOL_EFFECTS`, 234 entries (`src/contracts/toolEffects.ts:54-762`); static provenance `CAPABILITY_DECLARATIONS`, 66 keys and 70 provider bindings (`src/composition/capabilityDeclarations.ts:116-528`); runtime bindings `modules`, 49 keys (`src/composition/capabilityServices.ts:225-590`); and the CLI names `CLI_COMMANDS` (`src/cliCommandNames.ts`). (OBSERVED; counts from grep over those files.)
- **Capability ≠ tool.** A *capability* (`elfNotes`, `hopper`, `webNetworkCapture`) is a static fact: which provider identities produce Evidence for which named operations. A *tool* is a `ToolContract` keyed by its operation name. The two link only by operation name strings, and those strings are type-checked against `TOOL_CONTRACTS` names (`capabilityDeclarations.ts:95-100`). (OBSERVED)
- `CAPABILITY_DECLARATIONS` has exactly **two runtime consumers**: the `Undeclared capability` guard in `createCapabilityServices` (`capabilityServices.ts:615-617`) and the product-catalog provider projection (`scripts/lib/catalog-builders.mjs:27-77`, loaded from `dist/` via `scripts/lib/catalog-core.mjs:42`). It does **not** route calls or select handlers. (OBSERVED by grep: no other importer.)
- Only **49 of the 66** declarations have a runtime `modules` entry. The other 17 (`hopper`, `ghidra`, `ida`, `kuna`, `nativeMacOS`, `managedStatic`, `artifactGraph`, `analysisView`, `byteEntropy`, `applicationWorkflows`, `javascriptApplication`, `javascriptRuntimeReconciliation`, `managedWorkflow`, the three `*ApplicationGraph` keys, `webScriptExport`) are wired by explicit source owners: the deep-engine registry in `src/composition/binary.ts`, lazy auxiliaries, or workflow construction inside the MCP/CLI registrars. The table is `satisfies Partial<Record<CapabilityDeclarationId, …>>` (`capabilityServices.ts:585-590`) for exactly this reason. REA's own verification report gives the same split: "66 static declarations have 49 runtime keys and 17 explicit source-owned workflow/engine groups; 70 bindings union to 67 identities" (`docs/verification/capability-composition-2026-11-01.md:160-162`). (OBSERVED + SECONDARY)
- Tool metadata is **derived, never hand-authored per field**. `toolContractMetadata(name)` produces `title` (snake_case → Title Case), `effects` (looked up in the `TOOL_EFFECTS` audit; it throws if the entry is missing), `annotations` (computed by `annotationsFromEffects`) and `returnsTargetContent` (true unless the tool is on a 7-entry allowlist) (`toolEffects.ts:765-831`). Contract authors write `name`, `kind`, `description`, the schemas and `examples`, and spread in the derived metadata (example: `src/contracts/native/binaryDiagnosticsToolContracts.ts:110-123`). (OBSERVED)
- Binding is **by name, typed end to end**: `toolContract("inspect_elf_notes")` returns the exact contract type (`toolContracts.ts:97-112`). Registrars live in per-family closures passed to `registerToolFamilies`, which demands a complete `{[Id in ToolFamilyId]: () => void}` map, both in MCP (`src/server/createServer.ts:358`) and in the CLI (`src/cli.ts:178`). A lint rule forbids selecting a contract by its array position (`scripts/lint/rules/no-positional-tool-contract.mjs:36-48`). (OBSERVED)
- **Catalog identity** is a SHA-256 over an RFC 8785-canonical projection of every tool's `{name,title,surface,description,effects,returns_target_content,annotations,input_schema,output_schema}`, plus the prompts and CLI command names (`src/catalogIdentity.ts:43-56,111-129`). It is computed per (input profile × output profile × family scope). Digests for the six default profiles are recorded after `tsc` into `dist/recordedCatalogDigests.js`. A separate byte-hash of the recorded `tools/list` catalog lets the deferred stdio runtime answer discovery without loading zod or composition (`src/recordedMcpCatalog.ts:26-29`, `src/main/deferredRuntime.ts:26-47`, `src/main.ts:105-116`). (OBSERVED)
- **Tests guard the catalog mainly through set equality**, with no snapshot files:
  - TOOL_EFFECTS keys equal the TOOL_CONTRACTS names.
  - Every contract belongs to exactly one family.
  - The live `createServer` `tools/list` equals the TOOL_CONTRACTS names, with matching title, description and annotations.
  - Every MCP tool is reachable from the CLI or explicitly listed as MCP-only.
  - CLI_COMMAND_NAMES equals the real Incur router's commands.
  - Declarations reference only known groups and operations, and providers have no conflicting identities.
  - A golden list of the 22 destructive tools is kept, and descriptions are linted against declared effects.

  Generated catalogs are produced from compiled code. They are checked for staleness in `--check` mode and are not committed (`.gitignore:43,72`; `docs/mcp-contracts.md:2049-2054`). (OBSERVED)
- **Answer to "minimum files to add a capability"** (detail in §11). A new standalone, provider-backed tool like `inspect_elf_notes` touches about **12 source files** (not counting new tests or docs):
  1. the domain schema
  2. the provider implementation
  3. the provider's identity constant
  4. the port and application service
  5. the composition factory
  6. `capabilityDeclarations.ts`
  7. `capabilityServices.ts`
  8. the contract array file
  9. `toolEffects.ts`
  10. the MCP registrar plus the `createServer.ts` wiring
  11. the CLI command module plus `cliCommandNames.ts`
  12. a `toolEffects.test.ts` exception entry if the description uses run/launch/execute wording

  Adding one operation to an *existing* group whose providers bind the whole group needs only the contract, the effect audit and the registrar, plus the CLI. The provider catalog picks up the new operation automatically (`tests/conformance/capabilityDeclarations.test.ts:35-64`). (OBSERVED + INFERRED)

---

## Corrections to the salvaged summary

| Salvaged claim | Status | Evidence |
|---|---|---|
| "~67 keys" in the static table | Corrected: **66 capability keys**, **70 provider bindings**, **67 distinct provider identities** | grep of `capabilityDeclarations.ts`; `docs/verification/capability-composition-2026-11-01.md:160-161` |
| Runtime table maps "the same keys" | Corrected: **49 of 66** keys; 17 are declaration-only and wired elsewhere | key diff of `capabilityDeclarations.ts` vs `capabilityServices.ts:225-584` |
| "ownership is inferred from injection (`outsideHost`, :212-213)" | **Wrong.** `outsideHost` computes `hostUnsupported` only. Ownership is `"caller"` only for the 5 optional observation ports, and only when `optionalObservationProviderOwner === "caller"` (`capabilityServices.ts:217-220`, set at `src/main/transport.ts:152`). Injected services such as `options.goBinary` are still `scope.own`ed and closed by the host (`:631-633`) | `capabilityServices.ts:189-190,212-220,320-322,631-633` |
| "~45 contract groups" | Corrected: **47** | `toolFamilies.ts:53-99` |
| `registerToolFamilies` at `createServer.ts:347` | Line is now **:358** | `src/server/createServer.ts:358` |
| `productCatalog.test.ts` "compares the committed JSON" | Corrected: `docs/public/product-catalog.json` is **gitignored and build-generated**. The test compares the on-disk generated file to a fresh projection (a staleness check) | `.gitignore:72`; `docs/mcp-contracts.md:2049-2054`; `tests/boundary/filesystem/productCatalog.test.ts:72-120` |
| Open Q: "no declaration→registration test found" | Partly answered: operations are covered **transitively**. The catalog builder enforces declared ops ⊆ `TOOL_CONTRACTS`, and `serverWorkflowMcp.test.ts` enforces live `tools/list` == `TOOL_CONTRACTS`. Nothing requires every *contract* to have provider provenance | `catalog-builders.mjs:58-62`; `tests/boundary/mcp/serverWorkflowMcp.test.ts:153-155` |
| Everything else spot-checked (line ranges for combinators, Undeclared throw, TOOL_EFFECTS/metadata, TARGET_CONTENT_FREE_TOOLS, deferred `readCatalog`, module-boundaries, `capability-composition.md:27,102-103`) | Confirmed | cited below |

---

## 1. Vocabulary (what each word means in REA)

| Term | Definition | Owner |
|---|---|---|
| Operation / tool name | snake_case string, e.g. `inspect_elf_notes`; the MCP tool name and the Evidence `operation` | `ToolContract.name` (`src/contracts/toolContractTypes.ts:46`) |
| ToolContract | `{name,title,description,kind,requiresActiveTarget?,retainedSummary?,inputSchema,outputSchema,effects,annotations,returnsTargetContent,examples}` | `toolContractTypes.ts:37-66` |
| ToolKind ("surface") | closed list of 17 adapter families (`official-proxy` … `session`); it becomes the `surface` field in the catalog digest | `toolContractTypes.ts:13-31`; `catalogIdentity.ts:47` |
| Contract group | a named exported contract array (e.g. `BINARY_DIAGNOSTICS_TOOL_CONTRACTS`), the unit a provider can bind wholesale | `TOOL_CONTRACT_GROUPS`, `toolFamilies.ts:52-100` |
| Tool family | an advertising/selection unit (`native`, `artifact`, `browser`, …), the union of groups; `REA_MCP_TOOL_FAMILIES` scopes on it | `TOOL_FAMILY_DEFINITIONS`, `toolFamilies.ts:116-245` |
| ProviderIdentity | frozen `{id,name,version}` constant owned by the adapter module | e.g. `ELF_NOTES_PROVIDER`, `src/domain/investigationProviders.ts:50-54` |
| Capability (declaration) | key → list of `{identity, operations}` where operations = explicit name list or `{group}` | `capabilityDeclarations.ts:98-108` |
| Capability (runtime module) | key → factory returning `{value, close, ownership, availability?}` | `capabilityServices.ts:174-210` |
| CapabilityDescriptor | per-operation availability/effects/target-applicability published by session `AnalysisProvider`s (deep and auxiliary engines) | `src/application/AnalysisProvider.ts:263-300` |

INFERRED: REA has two meanings of "capability". One is the composition key in `capabilityDeclarations.ts`. The other is a per-operation provider descriptor (`CapabilityDescriptor`) used by `BinarySession`/`AnalysisProviderRegistry`. Only deep and auxiliary session providers publish descriptors. Standalone services like ELF notes publish only an identity.

---

## 2. Static declarations: `src/composition/capabilityDeclarations.ts`

OBSERVED:
- Imports are restricted to `import type` for `ProviderIdentity`, `TOOL_CONTRACTS` and `ToolContractGroupId` (`:2-4`), plus value imports of **identity/metadata leaves only** (`*ProviderIdentity.ts`, `*ProviderMetadata.ts`, `investigationProviders.ts`) (`:5-93`). The file header comment says the table "has no execution or host-availability logic" (`:1`). The docstring on the table says "Shared source facts, with no factory, schema or generated-catalog imports" (`:115`).
- `CanonicalOperationName = (typeof TOOL_CONTRACTS)[number]["name"]` (`:95`). An explicit operation list therefore fails to compile if it names a non-existent tool. The `{ group: ToolContractGroupId }` form is type-checked against group keys (`:98-100`).
- `providerBinding(identity, operations)` is a trivial `const` generic constructor (`:110-113`). The table is `as const satisfies Readonly<Record<string, CapabilityDeclaration>>` (`:528`). `CapabilityDeclarationId = keyof typeof CAPABILITY_DECLARATIONS` (`:530`).
- Patterns in use:
  - Whole-group binding: `adbDevice` → `{group:"ADB_TOOL_CONTRACTS"}` (`:117-121`).
  - Explicit subset: `binaryLayout` → PWNTOOLS `["inspect_binary_layout"]` (`:191-195`).
  - One provider under several capabilities: PWNTOOLS also under `recordedCrash` (`:444-448`); CDP_BROWSER under `browserObservation` (`:196-201`) and `webRuntime` (`:509-515`).
  - Several providers under one capability: `webNetworkCapture` has HAR, SAZ, mitmproxy and qlog, all bound to the same group (`:493-508`); `reverseEngineering` has objdump and rizin (`:449-454`).
  - Engine metadata constant: `ghidra` → `GHIDRA_OPERATIONS` from `GhidraProviderMetadata.ts` (`:296-298`).
  - Empty provenance: `artifactResources: { providers: [] }` (`:190`), a host-owned shared resource scope with no Evidence of its own.
- `docs/capability-composition.md:10-20`: group references exist so that "a new group operation is included without updating a copied provider list". "New provider provenance must come from the implementation that actually produces the Evidence, including format-specific capture adapters and derived workflows."

Validation (OBSERVED, `scripts/lib/catalog-builders.mjs:27-77`, `providerCatalog`):
- Merges bindings by `identity.id`. It throws `Provider <id> has conflicting source identities` if `name` or `version` differ (`:40-47`).
- It throws `declares an unknown contract group` (`:52-55`) and `declares an unknown analyst operation` (`:58-62`).
- Output is sorted by id, with capabilities sorted by `localeCompare`. Declaration order is irrelevant; test at `tests/conformance/capabilityDeclarations.test.ts:114-145`.

Purity guard: UNKNOWN/likely absent. I found no automated check that `capabilityDeclarations.ts` stays free of schema or factory imports. `scripts/lib/module-boundaries.mjs` only constrains `domain`/`contracts`, `application`, `server` and some provider roots (`:6-95`). The verification report states the property was measured ("Static imports reach 44 pure modules without schema, engine or factory runtime imports", `docs/verification/capability-composition-2026-11-01.md:161-162`). (SECONDARY)

---

## 3. Runtime bindings: `src/composition/capabilityServices.ts`

OBSERVED:
- `CapabilityServiceOptions` (`:114-149`) holds the optional injection slots for 33 services and ports, plus `optionalObservationProviderOwner: "server"|"caller"` and `optionalProviderLoadFailures`.
- `CapabilityHostContext` (`:165-172`) is `{environment, providerEnvironment, settings, options, platform, arch}`. Every factory receives this one object.
- Combinators:
  - `capability(create, close, observe?, ownership = () => "host")` (`:181-202`) calls `create(context)` once, binds `close(value)`, records ownership, and evaluates `observe(context, value)` into availability facts **at construction time**.
  - `service(create, observe?)` (`:204-210`) is sugar for values with `close()`. Managers keep `closeAll` via `capability(...)`: `gdbSessions` (`:406-414`) and `fridaInstrumentation` (`:447-459`).
- Fact helpers:
  - `outsideHost(injected, supported) = injected === undefined && !supported` (`:212-213`). An injected implementation overrides host-support refusal.
  - `configuredPwntools` requires host support and `isAbsolute(env.REA_PWNTOOLS_PYTHON)` (`:214-216`).
  - `tsharkLocatable` catches the throw from a relative `REA_TSHARK_COMMAND` and reports `false`, with the comment "configuration the tool reports at execution" (`:151-163`).
- Availability facts are data with three parts:
  - `hostUnsupported.{key}`: booleans.
  - `optionalFeatures.{x}Enabled`: booleans. The allowed keys are pinned by `Pick<AvailabilityPolicy, …>` (`src/application/CapabilityAvailability.ts:13-30`).
  - `probes`: named `(signal) => Promise<ProviderAvailability>` functions, typed by `CapabilityAvailabilityProbes` (`CapabilityAvailability.ts:34-50`).

  A probe is attached only when it can matter; for example, `evmInterface` omits its probe if an implementation is injected or the host is unsupported (`:255-263`). `observeProviderLoadFailures` (`fridaInstrumentation`, `:451-458`) is a deferred observer, not a fact.
- `createCapabilityServices(context)` (`:599-645`):
  - Maps `Object.entries(modules)`, with the comment "a key-preserving map of a source-owned typed declaration, not an external value parser or a generic operation dispatcher" (`:610-611`).
  - For each entry it calls `create(context)`, looks up `CAPABILITY_DECLARATIONS[id]` and throws `Undeclared capability ${id}` if absent (`:615-617`).
  - It merges `hostUnsupported`, `optionalFeatures` and `probes` with `Object.assign` (`:618-626`). Last writer wins on a key collision, with no duplicate detection. INFERRED risk: two modules writing the same `hostUnsupported` key would silently overwrite each other.
  - Host-owned values are wrapped by `scope.own(id, value, close)`; caller-owned values are returned bare (`:629-634`).
  - It returns `{services, availability, probes, observeProviderLoadFailures, close: () => scope.close()}` (`:637-644`).
- `CapabilityServices` is a mapped type that keeps each entry's precise value type (`:592-596`). That is why `createServer.ts:193-237` can destructure `elfNotes` as an `ElfNotesService` with no casts.
- INFERRED: `satisfies Partial<Record<CapabilityDeclarationId, …>>` already rejects an undeclared key at compile time, because `satisfies` performs excess-property checks on object literals. The runtime throw is a second check for compiled or bundled output.

`CapabilityScope` (`src/application/CapabilityScope.ts:1-40`, OBSERVED):
- `own(id, value, close)` rejects registration after shutdown (`:10-11`) and duplicate ids (`:12-13`).
- `close()` stops accepting new registrations and joins a single in-flight attempt (`#closing ??=`).
- Each close runs through `Promise.resolve().then(close)`, so a synchronous throw cannot block its peers (`:23-24`).
- Successful closes are deleted from the pending set; failed ones stay pending for retry. Failures are thrown as an `AggregateError(failureMessage)` (`:28-34`).
- `#closing` is cleared in `finally` so a later call retries (`:35-37`).

Server lifetime (OBSERVED, `createServer.ts:265-332`): `capabilities.close` is attached to both `server.server.onclose`, which logs failures (`:311-323`), and `server.close`, which joins via `Promise.allSettled` and rethrows one failure or an `AggregateError` (`:324-333`). `docs/capability-composition.md:38-39`: "There is no independent server provider-close list to update."

---

## 4. Where the 17 declaration-only capabilities are wired

| Declaration keys | Runtime owner | Evidence |
|---|---|---|
| `hopper`, `ghidra`, `ida`, `kuna` | constructed directly and passed to `new AnalysisProviderRegistry([hopper, ghidra, ida, kuna], config.analysisProvider, …)` inside `composeDeepBinarySession`; "the sole production wiring point" | `src/composition/binary.ts:23-27,63-87` |
| `artifactGraph`, `nativeMacOS`, `managedStatic` | `auxiliaryAnalysisProviderDeclarations()` gives `{identity, capabilities, load: () => import(...)}`, each wrapped in `LazyAnalysisProvider` | `src/composition/auxiliaryAnalysisProviders.ts:18-53`; `binary.ts:88-90`; `src/application/binary/LazyAnalysisProvider.ts:19-23` |
| `byteEntropy` | `createByteEntropyService(artifactResources)` called **inside the MCP `native` registrar**, borrowing the host's `artifactResources` scope ("Its wrapper does not acquire a second ownership claim", `capability-composition.md:58-60`) | `createServer.ts:425-433`; `src/composition/binaryDiagnostics.ts:70-73` |
| `analysisView`, `applicationWorkflows`, `javascriptApplication`, `javascriptRuntimeReconciliation`, `managedWorkflow`, `*ApplicationGraph`, `webScriptExport` | application workflows composed by their registrars over session/evidence ports (derived Evidence, no external engine) | INFERRED from the absence of `modules` entries and the doc text "derived workflows" (`capability-composition.md:19-20`); per-registrar construction not individually traced |

Deep-provider provenance is enforced at runtime, not by the declaration. `AnalysisProviderRegistry` throws `published mismatched capability provenance` when a descriptor's `provider` differs from `provider.identity()`, and throws on duplicate ids and duplicate operations (`src/application/binary/AnalysisProviderRegistry.ts:144-167`). It sorts providers by id (`:168-172`). For standalone services, provenance alignment relies only on **importing the same constant** in both places (`ElfNotesProvider.identity = ELF_NOTES_PROVIDER`, `src/native/elf/ElfNotesProvider.ts:33,44`; declaration at `capabilityDeclarations.ts:254-256`). No runtime check compares them. (OBSERVED)

---

## 5. The rest of `src/composition/` (46 files)

OBSERVED (sizes from `wc -l`): almost every file is a 4-30 line **factory leaf** that imports one application service and one provider and returns `new Service(new Provider(env))`, for example `binaryDiagnostics.ts:47-49`, `sqlite.ts` (8), `go.ts` (6), `frida.ts` (10), `cutter.ts` (11). The larger ones:

| File | Lines | Role |
|---|---|---|
| `capabilityServices.ts` | 645 | runtime host table (§3) |
| `capabilityDeclarations.ts` | 530 | static provenance (§2) |
| `binary.ts` | 182 | deep `BinarySession` composition, engine registry, auxiliaries, bundle-resource clients |
| `optionalObservationProviders.ts` | 122 | 5 browser/Electron/V8 ports via `await import(...)` (`:35-63`). `loadOptionalObservationProviders` runs them under `Promise.all` and turns each throw into `{providerId, reason}` (`:66-122`) |
| `directAnalysis.ts` | 93 | CLI one-shot analysis binding |
| `javascriptAnalysisWorkerProcess.ts` | 88 | owned worker process wiring |
| `binaryDiagnostics.ts` | 76 | 12 portable native-file service factories |
| `jeb.ts`, `resolveWindowsProviderExecutable.ts`, `reverseEngineering.ts`, `auxiliaryAnalysisProviders.ts` | 53-63 | engine-specific resolution |

`docs/capability-composition.md:79-87`: concrete construction belongs in `src/composition/` and its "default factory [stays] lazy". One-shot CLI adapters "import the selected factory rather than constructing a complete capability host" (`:31-32`). This is visible in `src/cli/binaryDiagnosticsCommands.ts:62`, where `createElfNotesService()` is constructed per invocation. (OBSERVED)

Dependency direction (OBSERVED, `scripts/lib/module-boundaries.mjs`):
- `domain`/`contracts` may import only `domain`/`contracts` (`:71-77`).
- `application` must not import `composition` (`:78-79`).
- `application`/`server` must not runtime-import the centralized Frida/Cutter/GDB/Rizin constructors (`:27-28,80-85`) or migrated provider roots and concrete providers (`:7-26,86-94`).

This is what forces construction into `composition/`.

---

## 6. How a contract binds to an implementation

OBSERVED chain:
1. A contract array per domain file, `as const satisfies readonly ToolContract[]` (e.g. `binaryDiagnosticsToolContracts.ts:72,287`).
2. `TOOL_CONTRACTS` is the ordered spread of 46 arrays (`toolContracts.ts:48-94`). Its order is the catalog's `tools/list` order and the family-scope filter order (`toolFamilies.ts:258-266`), and **never** a selector.
3. `TOOL_CONTRACT_GROUPS` exposes those same arrays by constant name (`toolFamilies.ts:52-100`). `TOOL_FAMILY_DEFINITIONS` composes groups into 18 families (`:116-245`). `toolFamilyId(name)` is derived from that composition (`:247-253`).
4. `toolContract(name)` is a typed lookup that throws `Missing tool contract` (`toolContracts.ts:97-112`). `toolFamilies.test.ts:24-41` uses `expectTypeOf` to prove the exact output-schema types survive the lookup.
5. Registrar: `server.registerTool(contract.name, toolRegistrationOptions(contract), handler(contract))` (e.g. `src/server/registerElfNotesTool.ts:18-30`). `toolRegistrationOptions` projects `title`, `description`, the advertised input JSON Schema (whose validator checks only the Evidence request frame), `outputSchema` and `annotations` straight from the contract (`src/server/toolRegistrationOptions.ts:18-50`).
6. `registerToolFamilies(registrars)` iterates `TOOL_FAMILY_IDS` (`toolFamilyIds.ts:41-45`). The parameter type `{readonly [Id in ToolFamilyId]: () => void}` makes a missing family registrar a compile error in **both** `createServer.ts:358` and `cli.ts:178`. Out-of-scope families are still registered but disabled (see `rea-mcp-server-adapter.md`; `docs/mcp-contracts.md:1860-1863`).

`docs/capability-composition.md:71-75`: "Named contracts bind handlers and schemas. The complete tool-family registrar map is checked by TypeScript, and catalog order never selects a handler or schema." The lint rule `no-positional-tool-contract` enforces the last clause by AST-walking the `TOOL_CONTRACTS` spread sources (`scripts/lint/rules/tool-catalog-sources.mjs:16-…`, used by `no-positional-tool-contract.mjs:9-11,40-46`). (OBSERVED)

---

## 7. How names, titles, annotations, ToolEffects and the untrusted marker are derived

OBSERVED (`src/contracts/toolEffects.ts`):
- `ToolEffects`: 10 booleans, documented to cover "possible effects across a tool's supported inputs, including temporary work" (`:1-22`). The `effects(overrides)` base is all-false with `idempotent: true` (`:32-44`). Shared presets: `evidence = {mutatesSession:true}`, `nativeEvidence` (+`launchesProcess`), `browserEvidence` (+`accessesNetwork`) (`:46-51`).
- `TOOL_EFFECTS: Record<string, ToolEffects>` is the hand-audited table (`:54-762`; `inspect_elf_notes: evidence` at `:264`).
- `annotationsFromEffects` (`:765-787`):
  - `readOnlyHint = !(any of 9 effect flags)`. Additive Evidence recording therefore makes most tools non-read-only.
  - `destructiveHint = mayDiscardData || executesForeignCode || mutatesDevice`.
  - `idempotentHint = idempotent`.
  - `openWorldHint = launchesProcess || accessesNetwork`.

  No name or family heuristics (comment `:764`).
- `TARGET_CONTENT_FREE_TOOLS` (7 names, `:800-808`); `toolReturnsTargetContent(name) = !set.has(name)`. Unknown names are untrusted by default (`:810-816`).
- `toolContractMetadata(name)` (`:819-831`) throws `Missing effect audit for ${name}`. It returns `{title: name.split("_").map(capitalize).join(" "), effects, annotations, returnsTargetContent}`, so `inspect_elf_notes` gets the title "Inspect Elf Notes". The **MCP tool name is the contract name**. The **CLI command name** is a separate constant (`CLI_COMMANDS.inspectElfNotes = "inspect-elf-notes"`, `src/cliCommandNames.ts:80`). By convention it is the kebab-case form, and `cliToolCoverage.test.ts` enforces that convention (§10).
- `docs/mcp-contracts.md:1311-1323`: annotations "are hints, not authorization controls". `readOnlyHint` "includes session state". Ordinary Evidence recording "is not destructive".

Second vocabulary (OBSERVED): provider `CapabilityDescriptor` effects are `{mutatesArtifact, launchesProcess, mayShowUi, mayAccessNetwork, mayWriteFilesystem, changesPermissions, requiresRoot}` (`src/application/AnalysisProvider.ts:263-271`). They appear in session discovery (`docs/mcp-contracts.md:974-978`: "Session tool effects also retain the applicable provider composition's UI, process, network and filesystem declarations"). UNKNOWN: whether the two vocabularies are reconciled anywhere.

---

## 8. Catalog identity and recorded catalogs

### 8.1 `src/catalogIdentity.ts` (270 lines, OBSERVED)
- `createToolCatalog` (`:43-56`) builds the per-tool digested projection. The comment at `:37-42` says `returns_target_content` must be included "or a stale process would report aligned".
- `presentToolCatalog` (`:58-83`) applies compact input and/or output presentation budgets.
- `createPromptCatalog` (`:85-95`) sorts by Unicode code point. `sortByName` (`:97-102`) puts contracts in code-point order, so **digests are independent of `TOOL_CONTRACTS` order**.
- `digestCatalogs` (`:111-129`) produces `tools_sha256` over `{output_schema_profile, catalog}`, `prompts_sha256`, and `combined_sha256` over `{cli: CLI_COMMAND_NAMES, tools, prompts}`. **Adding a CLI command changes the combined digest.**
- `createCatalogDigestProfiles` (`:161-183`) builds input `{full, compact}` × output `{full, compact, off}` = **6 profiles**, sharing one schema conversion.
- `createCatalogIdentity(options)` (`:193-235`) returns `{counts:{cli_commands, mcp_tools, mcp_prompts}, get digests()}`. The recorded digests are reused **only** when the input profile is built-in **and** `contracts === TOOL_CONTRACTS` **and** `promptContracts === PROMPT_CONTRACTS` (reference identity, `:214-219`). Otherwise the digests are computed lazily on first access (`:226-233`).
- Exports: `FULL_CATALOG_IDENTITY` (`:241`), `DEFAULT_CATALOG_IDENTITY` with compact input (`:244-246`), and `catalogIdentityForToolFamilies(…)`. The last one returns FULL when unscoped/full/full; otherwise it computes over `contractsForToolFamilies(scope)` and `promptsForToolFamilies(scope)` (`:253-270`). Both eager composition and deferred `attachRuntime` call it (`createServer.ts:252-263`), so "both paths derive the same value from the same scope" (`:248-252`).

### 8.2 `src/recordedCatalogDigests.ts` (10 lines) + `scripts/record-catalog-digests.mjs` (32 lines)
- The source holds `RECORDED_CATALOG_DIGEST_PROFILES = undefined`. "Source runs leave this undefined and compute them on first use" (`recordedCatalogDigests.ts:3-10`).
- After `tsc`, the recorder imports `dist/catalogIdentity.js`, calls `createCatalogDigestProfiles()` and **overwrites** `dist/recordedCatalogDigests.js` with frozen hash literals only (`record-catalog-digests.mjs:8-32`). The comment says this runs "after `tsc` and before bundling, so the digests come from the same compiled contracts and pinned schema converter that the runtime serves" (`:5-7`).

### 8.3 `src/recordedMcpCatalog.ts` (29 lines) + `scripts/record-mcp-catalog.mjs` (128 lines)
- Type `RecordedMcpCatalog {version:1, info, capabilities, instructions, identity, tools: Tool[], evidenceReferenceAlternatives: string[], prompts: (Prompt & {parameters})[]}` (`recordedMcpCatalog.ts:12-24`). `RECORDED_MCP_CATALOG_SHA256 = undefined` in source, "Replaced after compilation by the recorder" (`:26-29`).
- Recorder, for each of `compact` and `full` (`record-mcp-catalog.mjs:122-124`):
  - creates a temp HOME/USERPROFILE/XDG_* environment ("independent of the host account and user configuration", `:30-39`);
  - runs the **real** `parseConfig`, `createBinarySession` and `createServer` over `InMemoryTransport` with a real `Client` (`:43-61`);
  - records `listTools()`/`listPrompts()` and joins prompt contract parameters (`:62-70`);
  - writes `dist/mcp-catalog{,-full}.json` and SHA-256s the exact bytes (`:85-94`).

  Cleanup is reverse-ordered with `allSettled`. On failure it throws an `AggregateError` that keeps the temp home for inspection (`:98-119`). It then writes `dist/recordedMcpCatalog.js` with both hashes (`:125-128`).
- Build order: `compile = clean → tsc → record-catalog-digests → record-mcp-catalog → bundle-runtime` (`package.json:327`). The MCP recorder needs the digests first, because `identity` is embedded in the catalog.

### 8.4 Product catalog (facts projection, not served)
- `scripts/lib/product-catalog.mjs:8-53` loads compiled `dist/` modules (`catalog-core.mjs:35-55`). It asserts that the CLI inventory from the real `createCli()` matches `CLI_COMMAND_NAMES` (`:10-15`). It then builds `tools` via `toolFamilyCatalog`, which throws unless family total == `TOOL_CONTRACTS.length` == `FULL_CATALOG_IDENTITY.counts.mcp_tools` (`catalog-builders.mjs:4-24`), `providers` via `providerCatalog`, setup clients, CLI, and `runtime_catalog.digests.providers_sha256` (`product-catalog.mjs:45-51`).
- `scripts/generate-product-catalog.mjs:11-25` writes `docs/public/product-catalog.json`. With `--check` it fails on drift instead of writing (`ensureGeneratedFile`).
- `scripts/generate-mcp-tool-catalog.mjs:28-107` registers every contract on a bare `McpServer` with catalog-only handlers. It **validates every advertised input and output JSON Schema with Ajv2020** (`:29-38`), fails if the SDK omitted a tool (`:41-42`), and writes `.cache/mcp-tool-catalog.json` "for schema conformance tests" (`:19-21,99-107`).

---

## 9. Eager vs deferred registration (four levels)

OBSERVED unless marked:
1. **Process level: deferred stdio MCP.** `run()` picks `runDeferred` only when all of these hold: a recorded hash exists, the transport is not HTTP, no `HOPPER_TARGET_PATH`, the output profile is full/unset, `REA_MCP_TOOL_FAMILIES` is unset, and nothing is injected (`createServer`/`loadOptionalProviders`). Otherwise it dynamic-imports `eagerRuntime.js` (`src/main.ts:98-120`). The comment at `:101-104` explains that a scoped registration "composes the canonical scope itself instead of advertising tools whose identity the runtime cannot verify". `readCatalog` hashes the file bytes, compares them with `RECORDED_MCP_CATALOG_SHA256[profile]`, and requires `version === 1`; otherwise `SERVER_START_FAILED` (`deferredRuntime.ts:26-47`). `DeferredEvidenceMcpServer` registers recorded tools whose validator or callback first `await ensureRuntime()` (`src/server/DeferredEvidenceMcpServer.ts:92-166`). Materialization enforces **name-set equality in both directions**: `bindTool` throws `Unexpected MCP runtime registration` for any tool not in the recorded set or with a schema-presence mismatch (`:214-226`), and `materialize` throws `MCP catalog does not match runtime registrations` if any recorded tool was not re-registered (`:195-203`). Internals: see `rea-mcp-server-adapter.md` §Deferred runtime.
2. **Host level: eager but side-effect-free.** `createServer` always builds all 49 modules (`createServer.ts:185-192`). Constructors "do not run these probes or launch engines" (`capability-composition.md:65-66`). Probes run only from session discovery and only for advertised families (`:66-67`; `src/server/registerSessionStatusTool.ts`).
3. **Provider level: lazy.** Auxiliary session providers expose static identity and capabilities and `import()` their implementation on first use (`auxiliaryAnalysisProviders.ts:25-51`, `LazyAnalysisProvider`). The five optional observation ports are dynamic-imported at process startup, with per-port failure capture (`optionalObservationProviders.ts:35-122`). A load failure surfaces at call time through `runEvidenceToolOperation`'s `loadFailure` branch (`src/server/contractToolHandler.ts:183-187`) and in availability (`src/application/CapabilityInventory.ts:441-446`).
4. **CLI level: per-command construction.** Registrars register commands only. The command `run` constructs its own factory (`binaryDiagnosticsCommands.ts:60-62`). `tests/boundary/cli/cliStartupImports.test.ts:220-227` asserts that `rea --help` and `import("./dist/cli.js")` load **no** deferred third-party packages, and `:271-…` that provider composition loads only when a provider command runs.

---

## 10. How tests and generators guard the catalog

| Guard | Fails when | Where (OBSERVED) |
|---|---|---|
| Type: operation names | a declaration names a non-existent tool or group | `capabilityDeclarations.ts:95-100` |
| Type: runtime keys | a `modules` key is not a declaration id | `capabilityServices.ts:585-590` (+ runtime throw `:615-617`) |
| Type: family registrars | a family id lacks a registrar in MCP or CLI | `toolFamilyIds.ts:41-45`; `createServer.ts:358`; `cli.ts:178` |
| Runtime: effect audit | a contract spreads `toolContractMetadata` for an unaudited name (throws at module load) | `toolEffects.ts:819-822` |
| Effects ↔ contracts | `TOOL_EFFECTS` keys ≠ `TOOL_CONTRACTS` names | `src/contracts/toolEffects.test.ts:143-147` |
| Description ↔ effects | the description says run/launch/execute/install without `executesForeignCode`/`mutatesDevice` or a written exception (and stale exceptions fail too) | `toolEffects.test.ts:16-…,149-170` (`inspect_elf_notes` exception at `:39`) |
| Destructive golden list | the set of tools with `executesForeignCode||mutatesDevice` changes (22 names) | `toolEffects.test.ts:172-203` |
| Annotations derived | an advertised annotation ≠ `annotationsFromEffects(effects)` | `toolEffects.test.ts:251-…` |
| Families | a contract sits in 0 or more than 1 family; definitions ≠ ids; registrar order | `src/contracts/toolFamilies.test.ts:44-79` |
| Live registrations | real `createServer` `tools/list` name set ≠ `TOOL_CONTRACTS`, or title/description/annotations differ from the contract | `tests/boundary/mcp/serverWorkflowMcp.test.ts:112-170` |
| CLI coverage | an MCP tool has neither a kebab-case CLI command, an alias, nor an `MCP_TOOLS_WITHOUT_DEDICATED_CLI` entry; stale MCP-only entries | `tests/boundary/cli/cliToolCoverage.test.ts:8-24,27-…,101-135` |
| CLI names | `CLI_COMMAND_NAMES` ≠ the real Incur router's primary commands; undocumented options | `tests/boundary/filesystem/productCatalog.test.ts:123-133`; `product-catalog.mjs:10-15` |
| Provider provenance | unknown group or op; conflicting identity; group growth not inherited; order dependence | `tests/conformance/capabilityDeclarations.test.ts:34-200`; `catalog-builders.mjs:40-62` |
| Product catalog freshness and doc facts | generated JSON ≠ fresh projection; README/AGENTS/install docs missing required catalog links or client names | `productCatalog.test.ts:72-120`; `scripts/lib/docs-facts.mjs:97-…` |
| Identity coverage | flipping `returnsTargetContent`, output schema/profile or examples does not change the digest | `tests/boundary/mcp/serverIdentity.test.ts:126-191` |
| Schema validity | an advertised JSON Schema fails Ajv2020 meta-validation | `scripts/generate-mcp-tool-catalog.mjs:29-38` |
| Deferred integrity | a corrupted recorded catalog; a scoped run must compose instead of serving the recording | `tests/boundary/mcp/deferredRuntime.test.ts:237,718,873` |
| Positional selection | code indexes `TOOL_CONTRACTS[n]` | `scripts/lint/rules/no-positional-tool-contract.mjs` |
| Layering | forbidden imports across layers | `scripts/lib/module-boundaries.mjs:40-96` |
| Packaged verifier metadata | a script imports a provider metadata symbol from an owner that no longer exports it (walks the import closure of `capabilityDeclarations.ts`) | `tests/conformance/verifierMetadataLinks.test.ts:44-61,200-239` |

INFERRED gaps:
- No check that every non-session contract has provider provenance in `CAPABILITY_DECLARATIONS`.
- No check that a standalone service's `provider.identity` equals the declared identity.
- No duplicate-key detection when merging `hostUnsupported`/`optionalFeatures`.
- The CLI passes the provider id as a string literal (`"rea-elf-notes"`, `binaryDiagnosticsCommands.ts:79`) instead of `ELF_NOTES_PROVIDER.id`.

---

## 11. Worked trace: `inspect_elf_notes` / `rea inspect-elf-notes`

All OBSERVED.

**(a) Domain** — `src/domain/native/elfNotes.ts`
- `inspectElfNotesInputSchema = z.strictObject({ path: localPathStringSchema, max_file_bytes: int 64..512 MiB default 64 MiB, max_entries: … })` (`:88-…`). Limit constants at `:10-12`.
- Result schema `elfNotesSchema` (`:197`).

**(b) Provider identity** — `src/domain/investigationProviders.ts:50-54`
```ts
export const ELF_NOTES_PROVIDER = Object.freeze({ id: "rea-elf-notes", name: "REA ELF note parser", version: "1" } as const);
```

**(c) Provider implementation** — `src/native/elf/ElfNotesProvider.ts`
- `class ElfNotesProvider implements ElfNotesPort { readonly identity = ELF_NOTES_PROVIDER; … }` (`:43-44`).
- `inspect()` reads a stable, bounded artifact snapshot (`readStableArtifact(path, max_file_bytes, signal, …)`) and calls `parseElfNotes(bytes, {path, sha256, bytes})` (`:59-75`). The parser is `ElfNoteParser.ts` (911 lines). No engine or process is involved.

**(d) Port + application service** — `src/application/binaryDiagnostics/`
- `ElfNotesPort {identity; close?(); inspect(input, options) → Result<ElfNotes, AnalysisError>}` (`ElfNotesPort.ts:13-20`).
- `ElfNotesService.inspect(rawInput, options)` (`ElfNotesService.ts:31-102`) runs these steps:
  1. Parse input with the domain schema (`:35-41`).
  2. Require an absolute path (`:42-51`).
  3. Call the provider (`:52-53`).
  4. **Re-validate the provider output** against `elfNotesSchema`, artifact-path identity and budgets, else return `AnalysisOutputError` (`:54-69`, `validReport` `:105-125`).
  5. Build Evidence with `this.provider.identity`, `operation: "inspect_elf_notes"`, `confidence: "observed"`, and locations as file-offset ranges (`:82-101`).
- The operation string is a local constant (`:21`).

**(e) Composition factory** — `src/composition/binaryDiagnostics.ts:47-49`
```ts
export const createElfNotesService = (): ElfNotesService => new ElfNotesService(new ElfNotesProvider());
```

**(f) Static declaration** — `capabilityDeclarations.ts:254-256`
```ts
elfNotes: { providers: [providerBinding(ELF_NOTES_PROVIDER, ["inspect_elf_notes"])] },
```
The product catalog projects this as `{id:"rea-elf-notes", name, version:"1", capabilities:["inspect_elf_notes"]}`.

**(g) Runtime binding** — `capabilityServices.ts:75,337`
```ts
elfNotes: service(() => createElfNotesService()),
```
- No availability facts are declared, ownership is `host`, and the scope's close calls `ElfNotesService.close()`, which calls `provider.close()` (`ElfNotesService.ts:27-29`).
- The binding is not injectable (no `CapabilityServiceOptions.elfNotes`).

**(h) Contract** — `src/contracts/native/binaryDiagnosticsToolContracts.ts:109-123`
```ts
{ name: "inspect_elf_notes", ...toolContractMetadata("inspect_elf_notes"), kind: "native-provider",
  description: "Inspect one explicit local ELF32/ELF64 image's PT_NOTE … without executing it …",
  inputSchema: inspectElfNotesInputSchema, outputSchema: evidenceResultOf(elfNotesSchema),
  examples: [{ title: "Inspect ELF GNU build-id and ABI-tag notes", input: { path: "/artifacts/program" } }] }
```
- It is part of `BINARY_DIAGNOSTICS_TOOL_CONTRACTS` (`:72`, closed `as const satisfies` `:287`).
- That array is spread into `TOOL_CONTRACTS` (`toolContracts.ts:53`), registered as a group (`toolFamilies.ts:61`), and included in family `native` (`toolFamilies.ts:127-138`).

**(i) Effects → annotations**
- `inspect_elf_notes: evidence` = `{mutatesSession: true}` (`toolEffects.ts:46,264`).
- That yields `readOnlyHint: false` (Evidence is recorded), `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`; title "Inspect Elf Notes"; `returnsTargetContent: true` (not on the allowlist), so results carry `_meta.provenance.untrusted_content`.
- Its description contains "executing", so it needs the exception at `toolEffects.test.ts:39`.
- `tests/boundary/mcp/elfNotesMcp.test.ts:25-35` asserts the advertised `readOnlyHint:false, idempotentHint:true`. Its test session throws if a deep provider is touched (`:18-20`).

**(j) MCP registrar** — `src/server/registerElfNotesTool.ts:11-31`
```ts
const contract = toolContract("inspect_elf_notes");
const handler = createEvidenceToolHandler(server, (input, options) => service.inspect(input, options), logger, recordEvidence, withAdmittedAnalysis);
server.registerTool(contract.name, toolRegistrationOptions(contract), handler(contract));
```
- `createServer` destructures the typed `elfNotes` from `capabilities.services` (`createServer.ts:210`) and calls `registerElfNotesTool(server, elfNotes, toolLogger, recordEvidence, analysisAdmission)` inside the `native` family closure (`:457-463`).
- At call time, `runEvidenceToolOperation` runs: optional-provider load-failure check → admission → logging → `execute(input, {signal, progress})` → `deliverEvidenceResult` (record Evidence, then encode) (`contractToolHandler.ts:162-209`).

**(k) Discovery/availability** — `CapabilityInventory.ts:585-591` marks `inspect_elf_notes` as always `available`, short-circuiting host and target checks. Without that branch, a `native-provider` tool with no descriptor would still default to `available` (`:865-878`). Other kinds default to `provider_missing`.

**(l) CLI** — `src/cli/binaryDiagnosticsCommands.ts:50-85`
- `cli.command(CLI_COMMANDS.inspectElfNotes, {...})` with a hand-written CLI description and `args.path`.
- Options reuse the domain schema shapes (`inspectElfNotesInputSchema.shape.max_file_bytes` and `.max_entries`, `:56-59`).
- `run` uses `withCommandCancellation`, constructs `createElfNotesService()`, and calls `finishCliEvidenceCommand(args.path, () => elf.inspect({path: absoluteCallerPath(...)…}, {signal}), elf, "rea-elf-notes", "inspect_elf_notes", signal)`, then `projectCliAnalysisResult`. The finisher binds the caller path and closes the service (`src/cli/cliEvidenceCallerPath.ts:39-53`).
- The registrar is called from the CLI `native` family closure (`cli.ts:188-193`). The name constant is at `cliCommandNames.ts:80`. `cliToolCoverage.test.ts` maps `inspect-elf-notes` back to `inspect_elf_notes` via `replaceAll("-", "_")`.

**(m) Catalog/identity effects.** The contract enters `createToolCatalog`, so it affects `tools_sha256`, and the CLI name affects `combined_sha256`. The recorded `mcp-catalog*.json` and digests change on the next `compile`, and the product catalog gains a provider row.

**(n) Docs/tests around it.**
- Docs: `docs/elf-notes.md`, `docs/cli.md:442-447`, the VitePress nav (`docs/.vitepress/config.ts:38`), and a README table row in each translation (`docs/readme/README_*.md`).
- Tests: `src/native/elf/ElfNoteParser.test.ts`, `ElfNotesProvider.test.ts`, `ElfNotesService.test.ts`, `src/domain/native/elfNotes.test.ts`, `tests/boundary/mcp/elfNotesMcp.test.ts`, `tests/boundary/filesystem/elfNotes.test.ts`, `tests/acceptance/elfNotesCli.test.ts`, `elfNotesCore.test.ts`, `elfNotesAbiEndian.test.ts`, and the OS-invalid-path row in `tests/acceptance/tools/localToolContracts.test.ts:240`.

---

## 12. Minimum set of files to add a new capability

INFERRED from the trace and the guards in §10. "Required" means the compiler, a load-time throw or an existing test fails without it.

### Case A: new standalone, provider-backed tool + CLI command (the ELF-notes shape)

| # | File | Why it is required |
|---|---|---|
| 1 | `src/domain/<area>/<x>.ts` (input + result zod schemas) | the contract and the service need them; `domain` must stay pure |
| 2 | `src/<adapter>/<X>ProviderIdentity.ts` or an entry in `src/domain/investigationProviders.ts` | shared identity for the provider and the declaration |
| 3 | `src/<adapter>/<X>Provider.ts` | the implementation (may be in an existing adapter dir) |
| 4 | `src/application/<area>/<X>Port.ts` + `<X>Service.ts` | validation, output re-validation, Evidence provenance; `application` may not construct providers |
| 5 | `src/composition/<area>.ts` (factory) | the only layer allowed to `new` the provider |
| 6 | `src/composition/capabilityDeclarations.ts` | provenance; otherwise the runtime binding throws `Undeclared capability` |
| 7 | `src/composition/capabilityServices.ts` | host-owned instance + close (+ availability facts); otherwise `createServer` has no typed service |
| 8 | `src/contracts/<area>/<x>ToolContracts.ts` (new entry or new array) | the ToolContract |
| 9 | `src/contracts/toolEffects.ts` | `TOOL_EFFECTS[name]`; otherwise `toolContractMetadata` throws at import |
| 10 | `src/server/register<X>Tool.ts` + `src/server/createServer.ts` | otherwise the live `tools/list` ≠ `TOOL_CONTRACTS` (`serverWorkflowMcp.test.ts:153-155`) |
| 11 | `src/cli/<area>Commands.ts` + `src/cliCommandNames.ts` | otherwise `cliToolCoverage.test.ts` fails (or you add the tool to `MCP_TOOLS_WITHOUT_DEDICATED_CLI` there instead) |
| 12 | `src/contracts/toolEffects.test.ts` | only if the description matches the execution wording regex (add an exception) or the tool is destructive (update the golden list) |

Add these when the new array is a new group or family:
- `src/contracts/toolContracts.ts` (spread)
- `src/contracts/toolFamilies.ts` (group + family membership; every contract must sit in exactly one family)
- a new `TOOL_FAMILY_IDS` entry in `toolFamilyIds.ts`, which forces new registrar closures in both `createServer.ts` and `cli.ts`

Add `src/application/CapabilityInventory.ts` if the kind is not `artifact-/managed-/native-provider` or the tool has host or configuration requirements. Otherwise `binary_session` reports `provider_missing`.

Expected but not required to compile: unit/boundary/acceptance tests, `docs/<x>.md`, `docs/cli.md`, README rows. Generated artifacts (`docs/public/product-catalog.json`, `dist/mcp-catalog*.json`, digests) are regenerated, not edited.

**Count: about 12-14 files touched, of which 5-6 are new**, plus tests and docs. `docs/capability-composition.md:79-91` describes the same steps in prose.

### Case B: new operation inside an existing whole-group binding
1. A contract entry in the existing group file.
2. `toolEffects.ts`.
3. The registrar, plus the service method, if the existing service does not already dispatch by name via `createContractToolHandler` (`contractToolHandler.ts:69-84`).
4. A CLI command + name, or an MCP-only entry.

No declaration edit is needed: the group reference grows automatically (`tests/conformance/capabilityDeclarations.test.ts:35-64`). **Minimum about 4 files.**

### Case C: new deep session engine (Hopper/Ghidra-shaped)
- Identity + `*ProviderMetadata.ts` with an `OPERATIONS` list.
- A provider implementing `AnalysisProvider` with `CapabilityDescriptor`s whose provenance equals its identity (enforced at `AnalysisProviderRegistry.ts:148-157`).
- `src/composition/binary.ts` (add to the registry array).
- A `capabilityDeclarations.ts` entry. No `capabilityServices.ts` entry is needed.
- The existing session tools need no new contracts unless new operations are added.

The scope rule applies: engines that execute, inject or mutate need a maintainer scope decision first (`CONTRIBUTING.md:192-200`).

---

## What to distill for agentic-anything

1. **Split the registry into a pure declaration table and a runtime binding table with identical keys.**
   - Declarations: `capability → [{driverIdentity, operations | {group}}]`, importable by generators and discovery without loading drivers.
   - Runtime: `capability → factory(ctx) → {value, close, ownership, availabilityFacts}`.
   - Cross-check keys at compile time (`satisfies Partial<Record<DeclId, …>>`) **and** at host build. Allow declaration-only keys for capabilities wired by a dedicated owner, as REA's 17 are.
2. **Make the operation name the only join key and type it.** Derive `OperationName` from the contract inventory, so declarations, registrars and CLI aliases cannot name a non-existent operation. Resolve contracts by name with an exact-type lookup, and lint against positional selection.
3. **Derive metadata; never hand-write annotations.**
   - Keep a mandatory per-operation effect audit. A missing entry should throw at load time.
   - Derive `readOnly`/`destructive`/`idempotent`/`openWorld` hints from it.
   - Mark output untrusted by default via an allowlist.
   - For a *drive* plane, extend REA's 10 flags with authority-relevant ones: `changesUiState` exists already; add `sendsInput`, `mutatesExternalApp`, `requiresConfirmation`. Keep "drives code we don't own ⇒ destructive".
   - Add REA's description-vs-effects regex test and a golden destructive list, so an effect change cannot slip through review.
4. **Use one complete, typed family→registrar map consumed by every surface.** MCP and CLI both pass `{[Id in FamilyId]: () => void}`, so a new family breaks both builds until wired. Keep family ids in a schema-free module for cheap startup validation and scoping (`REA_MCP_TOOL_FAMILIES`-style).
5. **Use set-equality tests instead of snapshots**:
   - effect audit == contracts
   - each contract in exactly one family
   - live `tools/list` == contracts (title/description/annotations equal)
   - every MCP tool has a CLI command, alias or explicit MCP-only reason
   - CLI names == real router
   - declared operations ⊆ contracts

   Add the two checks REA lacks: every non-session contract has driver provenance, and the runtime driver identity equals the declared identity.
6. **Use catalog identity as a canonical digest over everything a client sees**: name, title, surface, description, effects, untrusted flag, annotations, input/output JSON Schema, prompts, CLI names. Sort by code point so registration order never changes identity. Compute it per scope/profile. Expose it in a `session`/`status` tool with `expected_catalog_digest` so agents can detect a stale server.
7. **Record the discovery catalog at build time and serve it before composition (deferred mode).** Byte-hash the recorded `tools/list` and refuse to start on mismatch. Materialize the real host on the first call, and assert two-way name equality between recorded and materialized registrations. Fall back to eager composition whenever the scope or profile differs from what was recorded.
8. **Keep host construction side-effect free.** Constructors compute static facts (platform, configured absolute paths, injected implementations) and attach named `(signal) => Promise<Availability>` probes. Probes run only from discovery, and only for advertised families. Return `{reason, remediation}` per tool, never a stack trace.
9. **Use `CapabilityScope` semantics for every driver handle**: single owner, refusal of duplicate ids and post-shutdown registration, joined concurrent close, retry of failed closers only, `AggregateError`. Separate caller-owned (borrowed) handles explicitly. REA does this only for observation ports; agentic-anything should make `ownership` a first-class declaration field.
10. **Use lazy drivers with static identity.** Discovery never imports heavy driver code (Playwright, platform a11y libraries, Frida). Optional drivers load independently, failures become `{driverId, reason}`, and they surface at call time and in availability.
11. **Generate a contributor scaffold, not just a checklist.** REA's Case A touches about 12-14 files, which is the main friction for "make it EASY". An `agentic-anything new-capability <name>` generator should emit the identity, port/service, factory, declaration row, runtime row, contract, effect row, MCP registrar, CLI command and test stubs, then run the set-equality tests. Keep REA's hand-wired typed tables as the generator's target format, so type safety is preserved without a generic JSON dispatcher.
12. **Re-validate driver output in the service layer**, as `ElfNotesService` re-parses provider output and checks artifact identity and budgets. For drive actions this becomes post-condition verification: the action record must re-observe state, not trust the driver's claim.

---

## Open questions

- Are ToolEffects (contract audit) and provider `CapabilityDescriptor` effects (`mayShowUi`, `changesPermissions`, `requiresRoot`, `AnalysisProvider.ts:263-271`) reconciled anywhere, or can `binary_session` report a provider effect that the contract audit does not declare?
- Is there an automated guard that keeps `capabilityDeclarations.ts` free of schema and factory imports? Only the verification report's measurement was found (`docs/verification/capability-composition-2026-11-01.md:161-162`).
- How are the 10 declaration-only workflow capabilities (`analysisView`, `applicationWorkflows`, …) constructed inside registrars, and do any of them escape `CapabilityScope` ownership? The doc forbids "fallback constructors that can escape the host owner" (`capability-composition.md:72-73`); this was not traced per registrar.
- `Object.assign` merging of `hostUnsupported`/`optionalFeatures` has no collision detection: is that intentional (keys are typed and unique by convention) or a latent bug?
- Why does `CapabilityInventory.ts` hard-code per-name availability branches (e.g. `:585-591`) instead of reading them from the runtime module's availability facts? It is a second hand-maintained table per tool.
- Could the 12-14 file Case A be shortened in REA itself, for example by co-locating contract + effect + registrar, without losing the layering guards? What would agentic-anything's generator need to emit to stay compatible with REA-style guards?
- Product catalog, recorded MCP catalog and digests were not inspected as built artifacts (`dist/` absent). Exact current counts beyond grep (234 tools, 18 families, 47 groups, 66 capabilities, 70 bindings, 67 identities, about 184 CLI command constants) are UNKNOWN until a build.
- `docs/roadmap.md` and epic #740 (`docs/verification/capability-composition-2026-11-01.md:3,246`): is a runtime plugin boundary planned, or will composition stay compile-time only (`capability-composition.md:102-103`)?
