# Walkthrough: how a new target/provider is added to REA, end to end

Research date: 2026-11-05

Source: local fork `/Users/nhanclie/rea` (identical to upstream `morluto/rea`; HEAD `3484432fb`, 6504 commits). All REA paths are repo-relative (`src/...`). Commit hashes are abbreviated.

Evidence legend: **OBSERVED** (read in the repo or git history; cited as `path:line` or a commit hash), **SECONDARY** (third-party summaries), **INFERRED** (my interpretation), **UNKNOWN**.

Related reports, not repeated here: registry mechanics in `salvaged/capability-registry-and-composition.json`; contract/Evidence shapes in `rea-contracts-and-evidence-model.md`; tool registration and transport in `rea-mcp-server-adapter.md`; process ownership in `salvaged/engine-bridges-and-process-supervision.json`; effects vocabulary and the verification ladder in `salvaged/tool-design-philosophy-and-verification.json`; setup and licensing in `rea-setup-distribution-supply-chain-licensing.md`. This report covers the **procedure and cost** of adding a target, not the design of each layer.

---

## Summary

1. **Examples chosen.** OBSERVED. I reconstructed two complete vertical slices from git history. Between them they cover the two shapes agentic-anything needs most:
   - **JEB** (`749e58772`, PR #1536, 2026-10-10). REA talks to an application through an MCP server that the application already ships. REA does not launch the application, and the user brings their own licensed copy.
   - **WASM/WABT** (`ffaf2c12f`, PR #1533, 2026-10-11). REA wraps a pinned external CLI toolchain (`wasm-validate`, `wasm-objdump`, `wasm2wat`) as owned one-shot child processes.

   I measured sqlite (`aeee5074d`), pcap (`9581e067a`) and EVM (`7af7bc88c`) for comparison only. Rizin and Cutter were left out because history refactors blur where they started. Kuna was left out because its introduction is split across `933f21669` (readiness/doctor) and `c1fd496e6` (operations).
2. **The first commit is mid-sized and consistent.** OBSERVED.
   - JEB: 48 files, +2051 lines.
   - WASM: 46 files, +2309 lines.
   - sqlite: 59 files, +5757. pcap: 69 files, +4082. EVM: 62 files, +3250.

   In each case about 1.0–1.2k lines are new production `src`, 0.45–0.7k are tests, 150–370 are a live real-tool verifier, and only **~50–75 lines are spread across 7–10 shared "seam" files**.
3. **The tail after the first commit costs 3–5x the first commit.** OBSERVED.
   - JEB: 40 follow-up commits touching JEB files (+5612/−1037) over 26 days. JEB-named files went from 17 to 34; total from 2051 to 6514 LOC.
   - WASM: 84 follow-up commits (+10541/−2075). This includes one added operation, `compare_wasm_interfaces` (#2464). Total went to 10674 LOC across 59 files.

   The tail is dominated by cleanup/ownership, deadlines, refusal preservation, input admission and format-fidelity fixes, not new features. **Making a target correct costs far more than scaffolding it.**
4. **Real-software verification found bugs that mocks missed.** OBSERVED. The squashed JEB PR body records that testing against a real Signal 8.30.3 APK "exposed a boundary bug the scripted provider tests missed": engine refusals were raised as protocol errors instead of being preserved. Testing against a real signed APK also changed the CLI option surface (`749e58772` body).
5. **Boilerplate is real, and REA extracted it after the fact.** OBSERVED. The day after JEB, adb and apktool landed with near-identical services and registrars, the `jscpd` duplicate gate forced `a97bd5ed2` ("share provider workflows to restore static gates"). That commit added `executeProviderOperation` (`src/application/ProviderOperationExecution.ts:46-55`) and `createContractToolHandler` (`src/server/contractToolHandler.ts:69`). The JEB service shrank from ~46 to 31 lines. In the current tree, `registerJebTools.ts` (49 lines) is four identical `toolContract → registerTool` stanzas.
6. **There is no scaffolder.** OBSERVED. There is no generator script among the 177 in `scripts/`, and no "add a provider" checklist in `AGENTS.md`/`CONTRIBUTING.md`. A human or LLM finds the seams by imitating a sibling provider, and gates catch omissions after the fact: type errors, the `Missing effect audit` throw, `Undeclared capability`, test-discovery, module boundaries and jscpd. The process gate comes before any code: `CONTRIBUTING.md:192-199` requires an explicit maintainer scope decision for new providers.
7. **Steps split three ways.**
   - Mechanical, generatable: ~15 seam edits and ~8 skeleton files.
   - Judgment: effects audit, authority/locality rules, result schema, coverage and limitation semantics, error mapping, wording.
   - Reality verification: upstream operation mapping, version banner and producer profile, envelope/refusal shapes, pagination caps, deadlines, cleanup on the real process.

   The live-verifier lane (`scripts/verify-jeb.mjs`, `scripts/verify-wasm-artifact.mjs`) was the step that most changed the code.

---

## 1. The vertical slice, layer by layer

### 1.1 Layer map

LOC = line count of the file in the introducing commit / at HEAD. All rows are OBSERVED from `git show --numstat` and `wc -l`.

| # | Layer | JEB file (intro / now) | WASM file (intro / now) | What it holds |
|---|---|---|---|---|
| 1 | Domain types | `src/domain/jeb/jebAnalysis.ts` (139 / 139) | `src/domain/wasm/wasmArtifact.ts` (126 / 193) | Zod input schemas per operation, a discriminated `Request` union, portable result schemas. JEB: `jebInputSchemas` `:10-59`, `jebRequestSchema` `:72-89`, `jebResultSchemas` `:104-139`. WASM: `inspectWasmArtifactInputSchema` with `command_timeout_ms` (`:21-37`). |
| 2 | Provider identity / pin | (inline at intro) → `src/jeb/JebProviderIdentity.ts` (– / 8, added by `e18594b24`) | `src/wasm/WabtRelease.ts` (12 / 12) | Frozen `{id,name,version}`. WABT also pins `WABT_VERSION="1.0.42"` and byte limits (`WabtRelease.ts:1-12`). |
| 3 | Contracts | `src/contracts/jeb/jebToolContracts.ts` (73 / 73) + test (46) | `src/contracts/wasm/wasmToolContracts.ts` (29 / 48) | `ToolContract[]`: name, `...toolContractMetadata(name)` (effects + derived annotations), description, `kind`, input/output schema (`evidenceResultOf(...)`), examples (`jebToolContracts.ts:10-73`). |
| 4 | Application port | `src/application/jeb/JebAnalysisPort.ts` (23 / 30) | `src/application/wasm/WasmArtifactPort.ts` (17 / 28) | Provider-neutral interface: `inspectAvailability` / `execute` / `close` (`JebAnalysisPort.ts:21-30`). WASM adds `commandTimeoutMs` options (`WasmArtifactPort.ts:16-18`). |
| 5 | Application service | `JebAnalysisService.ts` (46 / 31) + test (100 / 113) | `WasmArtifactService.ts` (100 / 84) + test (79 / 312) | Admission, then execute, then Evidence. JEB is now a single call to `executeProviderOperation(..., "derived" \| "observed")` (`JebAnalysisService.ts:13-31`). |
| 6 | Provider adapter | `src/jeb/JebProvider.ts` (421 / 601) + test (274 / 937); `JebMcpConnection.ts` (101 / 386) + test (86 / 517); `JebMcpEndpoint.ts` (56 / →`src/config/jebMcpEndpoint.ts` 63) | `src/wasm/WabtArtifactProvider.ts` (439 / 748) + test (194 / 1378); `WabtFailures.ts` (111 / 330); `WabtOutput.ts` (66 / 122) + test (92 / 566); `WabtWat.ts` (124 / 165); `WasmGlueAdapter.ts` (71 / 102) | The real integration. JEB maps REA operations to upstream tools (`JEB_ENGINE_TOOLS`, `JebMcpConnection.ts:28-33`), parses success/refusal envelopes (`JebProvider.ts:57-152`) and derives `supportedOperations` from the live `tools/list` (`:195-219`). WABT resolves tools from `REA_WABT_BIN_DIRECTORY`, hashes each executable, requires `--version` == `1.0.42` (intro `WabtArtifactProvider.ts:87-196`), re-hashes the input snapshot before each command, and runs via `runOwnedCommand` (`src/process/OwnedCommand.ts:163`). |
| 7 | Composition | `src/composition/jeb.ts` (13 / 63) | `src/composition/wasm.ts` (7 / 33) | Factory from environment. JEB now degrades to an `unconfiguredJebProvider` whose availability is `not_configured` instead of throwing (`jeb.ts:15-63`). WASM exposes a `wasmArtifactModule` static descriptor (`wasm.ts:18-33`). |
| 8 | CLI | `src/cli/jebCommands.ts` (118 / 97) | `src/cli/wasmCommands.ts` (60 / 111) | `incur` commands that reuse domain schema shapes for args/options (`jebCommands.ts:46-96`), wrapped in `withCommandCancellation → logCliCommand → finishAnalysisCommand` (`:24-44`). |
| 9 | MCP | `src/server/registerJebTools.ts` (58 / 49) | `src/server/registerWasmTools.ts` (32 / 65) | One `server.registerTool(contract.name, toolRegistrationOptions(contract), handler(contract))` per contract (`registerJebTools.ts:25-48`). |
| 10 | Boundary tests | `tests/boundary/cli/jebCli.test.ts` (53 / 171), `tests/boundary/mcp/jebMcp.test.ts` (143 / 398) | `tests/boundary/mcp/wasmArtifactMcp.test.ts` (84 / 922) + `tests/fixtures/wasm/*` (5 files, 96) | Through the real CLI and MCP harness. JEB later added a real HTTP MCP fixture server (`tests/fixtures/jebMcp.ts:19-40`). WASM ships `.wat` sources and a captured `objdump.txt` golden. |
| 11 | Live real-tool verifier | `scripts/verify-jeb.mjs` (153 / 167) + `package.json` `verify:jeb` (`:249`) | `scripts/verify-wasm-artifact.mjs` (313 / 672), `verify-packaged-wasm-artifact.mjs` (57 / 70) + `verify:wasm:*` | Drives the compiled service, or the public CLI plus stdio MCP (`verify-wasm-artifact.mjs:17-18,92`), against the real tool. It emits `{status:"skipped",reason}` unless tools are present or `--require-tools` is passed (`:33-45`). |
| 12 | Docs | `docs/jeb-analysis.md` (36 / 78), `docs/testing.md` (+19), README + 17 translations (+1 each) | `docs/wasm-artifacts.md` (71 / 233), `docs/testing.md` (+22), README (+1), `.agents/skills/.../SKILL.md` (+6/−1, skill version 37→38) | Bring-your-own instructions, semantics section (coverage, implicit selections, refusals), verification lane. |
| 13 | Third-party / licensing | none in the PR. `THIRD_PARTY_NOTICES.md:100` JEB EULA row and `DISCLAIMER.md:80` came in `974f11d01` (#2082) | `third_party/wabt/README.md` (24), listed in `package.json#files` | WABT: upstream URL, admitted release, source commit, licence, runtime tool list, SHA-256 of the verified archive, "version admission … does not certify publisher authenticity" (`third_party/wabt/README.md:1-24`). |
| 14 | Setup / doctor | none at intro. Later `b3c5672ce` and `08f99dbaf` added `REA_JEB_MCP_URL`/`_TIMEOUT_MS` to `src/application/SetupRegistrationEnvironment.ts:70-71`, `SetupHost.ts:213`, `ClientRegistrationEnvironment.ts:43,268` and `mcpStartupPolicy.ts:30,50` | `REA_WABT_BIN_DIRECTORY` in `SetupRegistrationEnvironment.ts:21` | Setup forwards the selected env vars into client registrations. **Doctor is only for deep engines**: `src/application/DoctorProviderIds.ts:2-8` lists hopper/ghidra/ida/jadx/kuna. Light providers report readiness through `binary_session` availability instead (§2). |
| 15 | Changelog | none | none | OBSERVED: `CHANGELOG.md` is curated at release time from conventional-commit titles (release-please, `release-please-config.json:6-14`). WASM appears at `CHANGELOG.md:217`. Feature PRs never edit it; the `feat(scope):` title is the input. |

### 1.2 Order of addition

**Inside the first PR.** INFERRED from the squashed body of `749e58772`. Git cannot show the order inside a squash.

1. Domain, contracts, provider, service, CLI, MCP, boundary tests, docs, and a live verifier, all at once. The body says it was "verified against a real JEB 5.48.0 licensed build".
2. A real signed probe APK was verified, which moved CLI positionals to options.
3. A real Signal APK exposed the refusal-decoding bug. The fix preserves the engine's refusal.
4. Flutter Gallery verification was documented.
5. The README row was translated into every language.
6. Two late security fixes: "enforce local endpoints and preserve cancellation" and "refuse redirects across the local MCP boundary".

**After the first PR**, these JEB files appeared. OBSERVED, `git log --diff-filter=A`:

| Date | Commit | New file(s) | Why |
|---|---|---|---|
| 10-12 | `af29247a7` | `src/jeb/JebMcpEndpoint.test.ts` | URL query/fragment edge cases |
| 10-15 | `a8887a9a8` | `tests/boundary/providers/jebMcpCatalog.test.ts` | live operation discovery deadlines |
| 10-16 | `d67d895c2` | `tests/acceptance/providers/jebConfigurationIsolation.test.ts` | a bad JEB timeout must not break shared startup |
| 10-17 | `a6e8f5269` | `JebMcpConnectionOwner.ts`, `JebMcpDeadline.ts`, `jebMcpCleanup.test.ts` | cleanup ownership + deadlines |
| 10-17 | `d74e72406`, `ece277b30` | `jebMcpTimeout.test.ts`, `tests/fixtures/jebMcp.ts` | timeout attribution, real HTTP fixture |
| 10-19…10-27 | `a0edd145d`, `e97f47b0b`, `da526fc01`, `c4cdb7658` | input-diagnostics, fixture-completion, reply-bytes, failure-envelope tests | fidelity of failures |
| 10-27 | `08f99dbaf` | `src/config/jebMcpEndpoint.ts`, `setupJebEndpointSelection.test.ts` | setup integration |
| 11-01 | `e18594b24` | `src/jeb/JebProviderIdentity.ts` | registry refactor |
| 11-05 | `d854fb155` | `jebMcpConnection.test.ts` | reconnect after stale MCP sessions |

**Canonical order** I recommend for a generator, INFERRED from dependency direction (domain → contracts → providers → application → adapters):

1. Scope decision.
2. Identity/pin.
3. Domain schemas.
4. Effects audit.
5. Contracts.
6. Port.
7. Provider.
8. Service.
9. Composition plus registry entries.
10. CLI and MCP registrars plus family/kind/name seams.
11. Availability/remediation.
12. Boundary tests.
13. Live verifier.
14. Docs, third_party and setup env forwarding.

---

## 2. Shared seams a new target must touch (current tree)

These are the edits outside the target's own directories. "Gate" says what fails if the edit is missed. OBSERVED unless marked.

| Seam | Location (HEAD) | JEB | WASM | Gate if omitted |
|---|---|---|---|---|
| Contract catalog spread | `src/contracts/toolContracts.ts:26,75` (JEB), `:5` (WASM) | ✓ | ✓ | Tool absent; `toolContract(name)` lookups fail to type-check |
| Contract group + family | `src/contracts/toolFamilies.ts:31,82,198-202` | new family | joined existing `artifact` family | Family not selectable |
| Family id | `src/contracts/toolFamilyIds.ts:20` | new id `"jeb"` | – | `registerToolFamilies` takes a complete `{[Id in ToolFamilyId]}` map (`:41-45`), so CLI and MCP call sites fail to compile, as intended (`:37`) |
| Tool kind | `src/contracts/toolContractTypes.ts:22` (`"jeb-provider"`) | ✓ | reused `artifact-provider` | Type error on `kind` |
| Effects audit | `src/contracts/toolEffects.ts:313-329` (JEB), `:163` (WASM) | ✓ (+17) | ✓ (+5) | `throw new Error("Missing effect audit for …")` at `:819-822` |
| Effects wording lint | `src/contracts/toolEffects.test.ts:84-85` (JEB), `:61-64` (WASM) | ✓ | ✓ | Test fails when a description says run/launch/execute/start/install without declaring `executesForeignCode`/`mutatesDevice` or an exemption (`:16-17`) |
| Target-content provenance | `tests/conformance/contracts/targetContentProvenance.test.ts:56` | `decompile_jeb_item` listed | – | Test fails unless `returnsTargetContent` matches the audited set |
| Static capability declaration | `src/composition/capabilityDeclarations.ts:361-365` (JEB), `:483-487` (WASM) | group binding | explicit op list | Catalog generation rejects unknown groups/ops (`tests/conformance/capabilityDeclarations.test.ts:168,185`) |
| Runtime capability service | `src/composition/capabilityServices.ts:510-519` (JEB live probe), `:226-236` (WASM static `wasmArtifactEnabled`) + `CapabilityServiceOptions` `:114-149` | ✓ | ✓ | `throw new Error("Undeclared capability …")` (`:617`) |
| MCP family registrar | `src/server/createServer.ts:235,682-689` (JEB), `:229,581` (WASM) | ✓ | ✓ | Compile error via the complete map |
| CLI family registrar | `src/cli.ts:54,219` (JEB), `:10,195` (WASM) | ✓ | ✓ | Compile error via the complete map |
| CLI command names | `src/cliCommandNames.ts:127-130` (JEB), `:8-9` (WASM) | ✓ | ✓ | Type error in `*Commands.ts` |
| Availability + remediation | `src/application/CapabilityInventory.ts:653-668` (JEB per-op from live catalog), `:573-580` (WASM flag) | ✓ | ✓ | **No hard gate** (INFERRED). The tool shows a generic unavailability with no target-specific repair |
| Live probe wiring | `src/application/CapabilityAvailability.ts:44-46`, `src/server/registerSessionStatusTool.ts:53,66-76,126-128` | ✓ | – | Same, no gate |
| Static-flag wiring | `src/server/sessionAvailabilityPolicy.ts:33-34` | – | ✓ | Same |
| Env schema | `src/config/environment.ts:70-74,136-150` (JEB). `parseConfig.ts:44-53` excludes JEB vars from native config | ✓ | (read in provider) | INFERRED: none at compile time |
| Setup env forwarding | `src/application/SetupRegistrationEnvironment.ts:21` (WABT), `:70-71` (JEB) | later | ✓ | Silent: client registrations drop the setting |
| Module boundaries | `scripts/lib/module-boundaries.mjs:16` (`MIGRATED_PROVIDER_ROOTS`) | – | ✓ | `verify:module-boundaries` |
| Vitest project globs | `scripts/lib/test-projects.mjs:22` (was `vitest.config.ts` at intro) | ✓ | ✓ | `verify:test-discovery`: "every test file was discovered exactly once" (`docs/specs/2356-wasm-interface-comparison/verification.md`) |
| CI test owner map | `scripts/ci/test-selection.mjs:16,20` | ✓ | ✓ | PR subset misses the provider |
| Manual-verifier list | `scripts/ci/scopes.mjs:728,733-734` | ✓ | ✓ | CI lane classification |
| Catalog builders (at intro) | `scripts/lib/catalog-builders.mjs` (+9 / +5), `catalog-core.mjs` (+2 / +2) | ✓ | ✓ | Since `e18594b24` the declarations file feeds the catalog. Neither file mentions JEB or WASM now |
| `package.json` | `verify:*` scripts (`:249`, `:319-321`), `files` (third_party README) | ✓ | ✓ | Lane not runnable / notice not shipped |
| Docs fan-out | README + 30 translations (`docs/readme/README_*.md`), `docs/index.md`, `docs/cli.md`, `docs/mcp-contracts.md`, `docs/testing.md`, `docs/testing-lanes.md:205-224`, SKILL.md (+ `generatedPackageMetadata.ts` skillVersion) | ✓ | ✓ | `tests/conformance/readmeConsistency.test.ts:47` for onboarding commands only. Rows are not gated (INFERRED) |

Count. OBSERVED at intro:
- JEB edited 7 shared `src` files (+62) and 4 glue files (+13).
- WASM edited 10 shared `src` files (+50) and 6 glue files (+13).

INFERRED at HEAD: a new provider with its own family and a live probe touches **~18–22 shared files** before docs fan-out. Most edits are 1–10 lines.

---

## 3. Cost of adding a target, measured

### 3.1 First commit, by category

OBSERVED: `git show --numstat`, bucketed by path. "src" means non-test files under `src/`.

| Commit | Files | +LOC | src (new / shared edits) | tests | fixtures | live verifier / CI | docs | README/skill | glue |
|---|---|---|---|---|---|---|---|---|---|
| JEB `749e58772` | 48 | 2051 | 1110 (10 new = 1048; 7 edits = 62) | 702 (6) | – | 153 (1) | 55 (2) | 18 (18) | 13 (4) |
| WASM `ffaf2c12f` | 46 | 2309 | 1243 (13 new = 1194; 10 edits = 50) | 450 (5) | 96 (5) | 370 (2) | 93 (2) | 44 (4) | 13 (6) |
| sqlite `aeee5074d` | 59 | 5757 | 2365 (30) | 3082 (13) | 77 (2) | 43 (2) | 153 (3) | 10 (2) | 12 (6) |
| pcap `9581e067a` | 69 | 4082 | 2306 (25) | 1370 (10) | 57 (5) | 185 (3) | 126 (2) | 25 (20) | 13 (4) |
| EVM `7af7bc88c` | 62 | 3250 | 1226 (26) | 1023 (11) | 44 (1) | 643 (2) | 174 (6) | 74 (7) | 45 (8) |

### 3.2 Inside new `src`, what is thin and what is thick

| Part | JEB | WASM |
|---|---|---|
| Thin wrappers: port + service + composition + CLI + MCP registrar | 23+46+13+118+58 = **258** | 17+100+7+60+32 = **216** |
| Schemas: domain + contracts | 139+73 = **212** | 126+29 = **155** |
| Provider adapter | 421+101+56 = **578** | 439+111+66+12+124+71 = **823** |

The provider adapter is ~55–65% of new src, and almost all of the judgment.

### 3.3 Tail to "current"

OBSERVED.

| | Follow-up commits on slice files | Churn on slice files | Files now | LOC now (src / tests) |
|---|---|---|---|---|
| JEB | 40 (10-10 → 11-05) | +5612 / −1037 | 34 | 6514 (1795 / 4474 incl. `src/**/*.test.ts`) |
| WASM | 84 (10-11 → 11-04) | +10541 / −2075 | 59 | 10674 (3282 / 5842), incl. `compare_wasm_interfaces` and retained views |

Theme of follow-up commit subjects. INFERRED: keyword classification of `git log --format=%s`, approximate.

| Theme | JEB | WASM |
|---|---|---|
| cleanup/ownership | 8 | 11 |
| deadlines/limits | 7 | 7 |
| refusal/diagnostics/recovery | 6 | 11 |
| input admission/identity | 6 | 15 |
| partial evidence / format fidelity | 4 | 19 |
| cancellation | 3 | 3 |
| refactor-to-shared | 2 | 5 |
| setup/config | 1 | 2 |
| other | 3 | 8 |

Examples:
- `a6e8f5269` "retain cleanup ownership and acquisition outcomes"
- `097a4bce6` "make upstream request deadlines configurable"
- `c4cdb7658` "fix(jeb)!: preserve engine refusal diagnostics"
- `9eee709f0` "accept unpadded WABT section rows"
- `1febc2290` "custom section names contain NUL"
- `abc8c6dad` "preserve leading BOM"
- `42667fc1a` "report WABT output past its budget as a resource constraint"

INFERRED takeaway. A scaffold removes maybe 300–500 of the first ~2k lines. The other ~1.5k lines of the first commit, and the 3–5x tail, are about how the real tool behaves at its edges. The kit's biggest leverage is therefore not the generator. It is (a) shared runtime primitives that make the tail unnecessary (ownership, deadlines, refusal envelopes, partial Evidence), and (b) a generated live-verification lane that finds the edge cases on day one.

---

## 4. Classification of every step

Each step is classified as boilerplate a generator can emit (GEN), human/LLM judgment (JUDG), or verification against the real software (REAL). INFERRED from §1–3. Evidence for each row is the cited code.

| Step | Class | Why / what the generator can and cannot know |
|---|---|---|
| Provider identity `{id,name,version}` | GEN (+REAL for version) | Name and id come from the manifest. The version must be observed (`--version` banner, `get_client_information`). |
| Domain request union + `discriminatedUnion` | GEN | `jebRequestSchema` (`jebAnalysis.ts:72-89`) is a mechanical function of `jebInputSchemas`. |
| Input schemas per operation | JUDG (seeded by REAL) | Field names and descriptions can be seeded from the upstream `tools/list` JSON Schema (JEB) or `--help` (CLI). Bounds come from probing the real tool, e.g. `count.max(100)` "the engine protocol caps pages at 100" (`:33-41`). Locality rules (`localPathStringSchema`, absolute paths, ≤16 glue paths) are judgment. |
| Result schemas | JUDG + REAL | Which upstream fields become portable facts, which stay `nullable`, and which become explicit unknowns (`unit_path: null` + limitation, `JebProvider.ts:516-522`). Raw upstream shape can be captured from a live call and diffed. |
| Effects audit | JUDG (template GEN) | Ten booleans (`toolEffects.ts:313-329`). A generator can propose `launchesProcess` for a CLI wrap and `accessesNetwork` for HTTP-MCP. Truthfulness is a review item, and `toolEffects.test.ts:16-17` enforces consistency with the wording. |
| Contract objects | GEN skeleton, JUDG description | Name/kind/schema/`toolContractMetadata`/examples are mechanical. Descriptions carry authority claims ("REA never installs or launches JEB", "No … WASM execution or original-source claim") and must be written and reviewed. |
| Port interface | GEN | Same three methods for every target (`JebAnalysisPort.ts:21-30`). |
| Application service | GEN | One `executeProviderOperation` call. The only per-target choice is `observed` vs `derived` confidence (`JebAnalysisService.ts:28`). |
| Upstream operation map | REAL | `JEB_ENGINE_TOOLS` (`JebMcpConnection.ts:28-33`) maps REA names to upstream names. It can be proposed from a live catalog but must be confirmed. Headless JEB 5.48 "do[es] not advertise open_project" (`749e58772` body). |
| Provider adapter: transport / process | GEN from shape template | Shape A (CLI wrap): resolve tool dir → hash → version admission → `runOwnedCommand` with deadline and output budget. Shape B (MCP-over-HTTP): loopback-only endpoint parser (`src/config/jebMcpEndpoint.ts:30-47`), SDK client, catalog probe. Both are reusable once written. |
| Provider adapter: envelope/refusal parsing | REAL + JUDG | `parseJebEnvelope` (`JebProvider.ts:109-152`) was rewritten after a live refusal. Refusals must keep the engine's own reason and the original payload in diagnostics. |
| Coverage / limitations | JUDG + REAL | `coverage: units.length < count ? "complete" : "partial"` (`JebProvider.ts:472`) rests on an observed page cap. "Missing coverage is unknown, not absence" (salvaged tool-design summary). |
| Composition factory | GEN | Env → provider. Degrade to `not_configured` on bad settings (`composition/jeb.ts:47-63`). The pattern is fixed; setting names come from the manifest. |
| Registry declaration + service entry | GEN | `providerBinding(IDENTITY, {group})` and `service(create, observe)`. Only the availability observation (static flag vs live probe) is a choice. |
| CLI commands | GEN (JUDG for UX) | Args/options reuse `schema.shape.x` (`jebCommands.ts:55-95`). Positional vs option choice changed after real use. |
| MCP registrar | GEN | Fully mechanical (`registerJebTools.ts:25-48`). |
| Family/kind/name seams | GEN | Append-only list edits, enforced by compile-time complete maps. |
| Availability + remediation text | GEN skeleton, JUDG text | Remediation strings are instructions to the agent ("Start a JEB client serving MCP … point REA_JEB_MCP_URL at it", `CapabilityInventory.ts:666`). |
| Env settings + setup forwarding | GEN | Names, validators and forwarding-list entries come from the manifest. Range rules come from shared schemas (`providerStartupTimeoutSchema`). |
| Unit/service tests with fake port | GEN | `JebAnalysisService.test.ts` style: fake port, assert Evidence shape. |
| Boundary tests via CLI/MCP harness | GEN skeleton + REAL fixtures | Harnesses exist (`createApplicationMcpHarness`, `tests/fixtures/jebMcp.ts`). Fixture replies should be **recorded from the real tool**, not hand-written. Hand-written mocks missed the refusal bug. |
| Live verifier lane | GEN skeleton, REAL execution | Skip-with-reason unless the tool is present (`verify-wasm-artifact.mjs:33-45`). Build an owned fixture (`wat2wasm`, compile a tiny Java class in `verify-jeb.mjs` help text). Drive the public CLI and stdio MCP. Assert schema-valid Evidence. Verify cleanup. |
| Third-party note | GEN template, JUDG licence | URL/version/commit/SHA fields are mechanical. Licence fit and EULA disclosure limits (`THIRD_PARTY_NOTICES.md:100`) are judgment. |
| Docs page | GEN skeleton, JUDG semantics | Headings recur: bring-your-own, settings, availability, semantics, verification lane (`docs/jeb-analysis.md`). The semantics bullets record REAL findings. |
| README rows ×31 | GEN | One row per language. JEB later translated all of them. |
| Changelog | none | Comes from the conventional-commit title. |
| Scope decision | JUDG (human) | `CONTRIBUTING.md:192-199`. It must cover authority, effects and verification before integration. |

INFERRED tally:
- ~16 of 28 steps are GEN or GEN-skeleton, about 40–45% of first-commit LOC once shared helpers exist.
- ~7 steps are dominated by REAL: operation map, version, envelope, bounds, coverage, fixtures, live lane.
- The rest is JUDG text and authority decisions.

---

## 5. What REA already does that makes the slice cheap

All OBSERVED.

- **Complete-map registrars**: `registerToolFamilies` (`src/contracts/toolFamilyIds.ts:41-45`). Adding a family id breaks every CLI/MCP call site until it is wired.
- **Fail-at-construction registry**: `Missing effect audit` (`toolEffects.ts:822`) and `Undeclared capability` (`capabilityServices.ts:617`) turn omissions into immediate errors.
- **Shared workflows**: `executeProviderOperation` (admission, cancellation, Evidence), `createContractToolHandler` (MCP), `finishAnalysisCommand` (CLI cleanup/cancellation, 166 lines), `runOwnedCommand` (577 lines of process ownership). Each target reuses them instead of re-deriving them.
- **Schema reuse across surfaces**: CLI options take `jebInputSchemas.x.shape.y` directly (`jebCommands.ts:64-67`), so CLI and MCP cannot drift.
- **Duplicate detection as forcing function**: `npm run jscpd` in `check:pr` (`package.json:130,149`) is what produced `a97bd5ed2`.
- **Spec bundle for non-trivial additions**: `docs/specs/2356-wasm-interface-comparison/{spec,plan,tasks,verification}.md`. `verification.md` records the base commit, test counts, real-tool runs, independent review findings, and explicit unmeasured items ("Token usage and full process-family peak are unmeasured. Real-tool verification covers macOS arm64 only").

## 6. What REA does not do (gaps a kit should fill)

INFERRED from the absences observed above.

- No scaffolder or checklist. The ~20 seams are found by imitation, and soft seams (availability remediation, setup forwarding, CI owner maps, docs rows) have no gate. JEB setup forwarding landed 4 and 17 days later (`b3c5672ce`, `08f99dbaf`).
- No machine-readable target manifest. The same facts are spread over identity, env schema, setup list, effects, docs and third_party: tool name, version pin, env vars, effects, host support, licence.
- Fixtures are hand-written, not recorded from the real tool. This is the root cause of the refusal bug the mocks missed.
- No CI lane for bring-your-own commercial or caller-supplied tools. JEB and WASM are both `MANUAL_VERIFIERS` (`scripts/ci/scopes.mjs:722-735`). EVM has a lane because its compiler is an npm package (`.github/workflows/real-evm-interface.yml`).

---

## What to distill for agentic-anything

1. **Define a `target.yaml` manifest as the single source for every mechanical seam.** INFERRED design. Fields:
   - `id`, `display_name`
   - `shape: cli-wrap | mcp-proxy | http-api | plugin-bridge | a11y`
   - `binary` / `endpoint`: locality rule, e.g. loopback-only like `jebMcpEndpoint.ts:36`
   - `version_probe`: command + expected banner, as in `WabtRelease.ts`
   - `settings[]`: env name, validator, forwarded-by-setup
   - `operations[]`: name, upstream mapping, input schema seed, `effects` (closed boolean vocabulary), `confidence` (observed|derived), `returns_target_content`
   - `licence`: upstream URL, licence, BYO flag, disclosure limits
   - `host_support[]`

   The generator emits identity, domain union, contract skeletons, port, service, composition, registry entries, CLI/MCP registrars, env schema, setup forwarding, availability stub, docs skeleton, third-party note, test skeletons and a live-verifier skeleton. Regeneration must be idempotent, and edits to shared seams must be append-only.
2. **Replace REA's ~20 scattered seams with discovery from one registry.** CLI, MCP, availability, setup and docs should iterate the manifest set. A new target should then touch **zero shared files**, versus 7–10 at intro and ~18–22 now in REA. Keep REA's complete-map typing and fail-at-construction checks as the gate for anything that must stay hand-written.
3. **Write shape templates once and reuse them.**
   - `cli-wrap`: tool resolution → executable SHA-256 → version admission → owned child with deadline and output budget → input snapshot re-hash → bounded parse.
   - `mcp-proxy` / `http-api`: loopback-only endpoint, no redirects/credentials, catalog probe → `supportedOperations`, success/refusal envelope that keeps the upstream reason and payload, reconnect after transport failure.

   The tail commits show these primitives are what break, so they belong in the kit runtime, not in each target.
4. **Make "discover → record → verify" the first step.** It replaces hand-written mocks. The `rea discover <target>` equivalent should:
   - (a) probe the version banner or `tools/list`
   - (b) propose `operations[]` and an upstream map with input schemas from upstream JSON Schema or `--help`
   - (c) run each proposed operation once against an owned fixture
   - (d) **record raw replies, including refusals and page caps, as boundary-test fixtures and result-schema seeds**

   This targets the observed failure (Signal APK refusal bug) and the largest tail categories (format fidelity, refusal preservation).
5. **Generate a live lane for every target, with honest skip semantics.** Use `{status:"skipped",reason}` unless the tool is present, `--require-tools` to make it mandatory, owned fixtures built at run time, assertions through the public CLI **and** MCP, and a cleanup check. Emit a `verification.md` record modelled on `docs/specs/2356-*/verification.md`: base commit, tool version + SHA, host OS/arch, what was not verified.
6. **Keep the judgment steps as explicit, reviewable prompts, and do not auto-fill them.** The scaffolder writes `TODO(judgment)` blocks for:
   - effects booleans, with a wording lint like `toolEffects.test.ts:16-17`
   - authority sentence in the description ("never launches/installs", "does not execute target")
   - coverage rule (complete/partial/unknown)
   - limitations for implicit upstream selections
   - remediation text
   - licence fit

   An LLM can draft them, but a reviewer or judge pass must sign off before the target is enabled. This mirrors REA's fail-closed integration rule (`CONTRIBUTING.md:184-190`).
7. **Gate on a scope/authority card before generation.** It is a short form: question answered, inputs/outputs, every execution/network/mutation/persistence effect including upstream behaviour, verification plan with owned samples, non-goals. This is the agentic-anything equivalent of `CONTRIBUTING.md:192-224`, and it sets authority level (observe/launch/attach/mutate) per operation.
8. **Model readiness in two tiers, as REA does.**
   - Cheap static flags (`wasmArtifactEnabled` = absolute path set, `capabilityServices.ts:226-236`) or a live probe (`jebAnalysisAvailability`) feed per-operation availability with remediation.
   - A deep `doctor` check is reserved for heavyweight engines (`DoctorProviderIds.ts`).

   The manifest says which tier applies.
9. **Budget for the tail.** Plan for roughly 3–5x the first commit in hardening. Expose the kit's own counters (follow-up fixes per target, by theme), so recurring fix themes get pulled into shared primitives the way `a97bd5ed2` and `e18594b24` did.
10. **Bound the "minutes" claim.** INFERRED. With the manifest, shape templates and discover/record, a generator plus LLM can produce a working, schema-valid, CLI+MCP-exposed target in minutes: everything up to the REAL steps. "Correct at the edges" still needs the live lane plus review, so present it as a separate state (e.g. `status: scaffolded | live-verified | hardened`).

## Open questions

1. How far can the result schema be inferred from recorded replies (JSON shape inference) before human judgment is needed on what is a fact vs an unknown? REA hand-authored every `nullable` and limitation.
2. Should agentic-anything keep REA's "no generic JSON operation dispatcher" rule (`docs/capability-composition.md:26-27`)? A manifest-driven runtime is close to that. Typed codegen keeps REA's guarantees; a dynamic dispatcher makes "minutes" easier. This needs an ADR.
3. For an `mcp-proxy` target like JEB, should the kit re-expose upstream tools almost 1:1 (cheap), or require REA-style provider-neutral renaming plus Evidence wrapping (costly but portable)? REA chose neutral names with an explicit map (`JEB_ENGINE_TOOLS`).
4. How do we run live lanes for bring-your-own commercial tools (JEB-like) in CI without distributing the tool? Options: self-hosted runner with a licence, recorded-reply replay with periodic manual refresh, or a vendor-provided CE build. REA leaves these as manual verifiers (`scripts/ci/scopes.mjs:722-735`).
5. What is the minimum docs fan-out? REA updated 18–31 translated READMEs per target. For agentic-anything, generated per-target docs from the manifest may be enough.
6. UNKNOWN: the true intra-PR order and elapsed time of the JEB and WASM PRs. They were squash-merged, and the source branches are not in the local clone.
7. UNKNOWN: whether REA maintainers used an LLM to write these slices. Commit co-authors are human handles plus "REA Contributor", so tooling cost cannot be separated out.
