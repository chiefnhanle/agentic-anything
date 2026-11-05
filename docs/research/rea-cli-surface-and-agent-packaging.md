# REA research: CLI surface and agent packaging (one capability set, two front doors)

Research date: 2026-11-05

Area: `cli-surface-and-agent-packaging`.
Source checkout: `/Users/nhanclie/rea` (the owner's fork of `morluto/rea`; `package.json` says `rea-agents@6.8.0`; HEAD `3484432fb`). That is newer than the `52ff312` commit the earlier REA reports used, at the same package version. I read the files and did not build or run anything. Paths are relative to the checkout.

Evidence legend: **OBSERVED** = read in the REA repo or a primary page, cited as `path:line` or a URL. **SECONDARY** = from search summaries or third-party pages. **INFERRED** = my interpretation. **UNKNOWN** = not established.

Earlier reports cover adjacent topics, so this one does not repeat them:
- `rea-mcp-server-adapter.md`: the MCP runtime, deferred vs eager boot, schema profiles, HTTP transport details, catalog digests.
- `rea-setup-distribution-supply-chain-licensing.md`: the `rea setup` transaction, the 25 client registrations, release pinning, `install.sh`.
- `salvaged/capability-registry-and-composition.json`: the family registrar map and module-boundary rules.
- `salvaged/tool-design-philosophy-and-verification.json`: the agent-eval harness and the "skills are subordinate to contracts" rule.

This report covers the CLI side of the CLI/MCP split, the agent-facing I/O conventions, how parity is enforced, and how one artifact becomes a plugin, a skill and a registry entry. It ends with a design for generating all of these from one contract.

---

## Summary

- **The CLI is hand-written; the MCP surface is generated from contracts** (OBSERVED). MCP tools are `ToolContract` objects (`src/contracts/toolContractTypes.ts:37-66`) projected into SDK registration. CLI commands are about 84 hand-written registrar modules in `src/cli/` (14,771 lines in total, plus `src/cli.ts` at 629 lines). They sit on the third-party **Incur** CLI framework (`incur@0.6.1`, `package.json:374`; `src/cli.ts:27`). Each command re-declares its own Zod args and options. Some reuse individual contract fields (`inspectSqliteDatabaseInputSchema.shape.worker_timeout_ms`, `src/cli/sqliteCommands.ts:28`). Each command maps camelCase options back to the snake_case MCP input by hand (`:60-78`), then calls the same application service the MCP handler calls.
- **Parity is enforced by tests, not by generation** (OBSERVED). `tests/boundary/cli/cliToolCoverage.test.ts:102-134` requires every `TOOL_CONTRACTS` name to satisfy one of three conditions:
  - it equals a CLI command name with `-` replaced by `_`;
  - it appears in an explicit alias table (14 commands, `:9-24`, e.g. `decompile` maps to `procedure_pseudo_code`);
  - it appears in an explicit, justified `MCP_TOOLS_WITHOUT_DEDICATED_CLI` list of 68 tools (`:27-99`).

  Of the roughly 172 MCP tools, 68 are therefore MCP-only. They are almost entirely session-, navigation- or annotation-bound: `open_binary`, `goto_address`, the `*_gdb_session` and `*_frida_session` tools, `record_unknown`, `update_analysis_settings`. A second test pins the Incur router's real command inventory to the frozen `CLI_COMMANDS` table (`tests/boundary/filesystem/productCatalog.test.ts:127-136`). It also requires a description on every arg and option.
- **The output layer was designed for agents** (OBSERVED, `src/cliOutput.ts:23-35`, `docs/cli.md:1181-1396`):
  - The default format is **TOON**, a token-lean notation. `--json` is indented on a TTY and compact when piped.
  - `--format` also accepts `yaml`, `md`, `jsonl` (with `$rea` control and metadata records), and `csv`/`tsv`. The delimited formats require an explicit row path and columns, plus a `# rea-delimited-v1 {json}` metadata header.
  - `--filter-output` takes key paths with slices. `--full-output` returns the `{ok, data|error, meta}` envelope.
  - `--token-count`, `--token-limit` and `--token-offset` give approximate token windows over the rendered text. Combinations that would corrupt structured output are rejected *before* the command runs, with a structured error code (`INVALID_OUTPUT_VALUE`, `UNSUPPORTED_OUTPUT_COMBINATION`, `MISSING_OUTPUT_VALUE`, `INVALID_OUTPUT_FILTER`).
- **Exit codes are coarse and documented** (OBSERVED): 0 = completed (possibly partial), 1 = could not complete, 128+N = signal (130 SIGINT, 143 SIGTERM, set after cleanup settles), 141 = the downstream reader closed the pipe (`docs/cli.md:1278-1282`; `scripts/rea.mjs:41-42,254-264`; `src/cli/commandCancellation.ts:9-32`).
  - Commands *return* error projections as values. A single classifier (`src/cliLogging.ts:37-80,104-148`) recognises a failure by validating the value against the canonical error-projection schemas, then sets `exitCode = 1`.
  - Documented exceptions exist: setup cancellation exits 0; `inspect-artifact` cancellation exits 1. That inconsistency is what you get from hand-wiring the routing.
- **JSON input is uniform** (OBSERVED, `src/cliJsonInput.ts:31-125`):
  - Any "JSON" argument accepts inline JSON, a file path, or `-` for stdin. Stdin is limited to 8 MiB and checked for heap headroom before parsing (`:127,144-164`).
  - The CLI resolves operator-relative path fields to absolute paths *before* the shared contract validation, because MCP contracts reject relative paths (`:273-313`).
- **Paging lives in the domain, not the transport** (OBSERVED). Evidence results carry `offset`/`limit`/`next_offset`/`coverage`. Large results are saved to a file and re-projected with `inspect-analysis-view` (a 334 MB Evidence file yields a ~10 KB summary, `docs/cli.md:26-62`).
  - The CLI's `detail: "summary"` delivery *requires* `--evidence_output`, so the complete Evidence is never lost in a stateless process (`src/cli/analysisEvidenceDelivery.ts:43-53,70-80`).
  - Token windows are a presentation-level fallback that refuses structured formats.
- **Packaging: one npm tarball is every front door** (OBSERVED; extends the setup report):
  - CLI `bin` (`package.json` `bin.rea`).
  - stdio MCP (`rea mcp`) and loopback HTTP MCP (`rea mcp --http`).
  - Claude Code plugin and Codex plugin. Both run `scripts/plugin-mcp-launcher.mjs <exact version>`, which refuses a mismatched pin (`scripts/plugin-mcp-launcher.mjs:8-35`).
  - MCP Registry `server.json`.
  - A skills.sh skill.

  The skill is authored in `.agents/skills/reverse-engineer-anything/` and *generated* into `skills/` (the plugin's `"skills": "./skills"`). Generation stamps the MCP tool count and rewrites `rea-agents@latest` to the exact pinned specifier (`scripts/generate-skill-metadata.mjs:19-40`).
  - Incur's own `skills add` sync is disabled (`sync: false`, `src/cli.ts:148`). Incur's own `--mcp` (CLI-commands-as-MCP-tools) mode is pre-empted by the dispatcher, which routes `mcp`/`--mcp` to REA's contract-first server (`scripts/rea.mjs:9-11,31-33`).
- **Two doctors** (OBSERVED):
  - `rea doctor` is a read-only host-readiness audit with explicit scopes (`--client`, `--provider`, `--skill`, `--target`; `src/cli/setupCommands.ts:117-149`).
  - `rea mcp doctor` spawns the production stdio server as a real MCP client. It checks `initialize` name, version and protocol, exact tool and prompt inventory (missing, unexpected and duplicate names), the server identity catalog digest, and a target-free `binary_session` request (`src/mcpDoctor.ts:236-347`). It retains at most 64 KiB of child stderr (`src/mcpStartupPolicy.ts:12-17`).
- **Prompts and skills reach hosts that lack the feature** (OBSERVED). Six MCP prompts are also reachable as a `guided_workflows` tool (for clients without `prompts`) and as `rea guided-workflows` on the CLI. The packaged skill bundle itself is searchable through `search_guidance`/`read_guidance_section` and `rea guidance` (`docs/mcp-prompts.md:1-50`). One guidance corpus serves three delivery channels.
- **Recommendation** (INFERRED): copy REA's *contract-first MCP* and its *parity test*, but invert REA's CLI approach. Generate the CLI, MCP registration, skill route table, plugin manifests and doctor expectations from one capability contract. Make "no CLI projection" a typed field with a reason, not a test-side list. For agentic-anything, which drives stateful software, the stateful/mutating capabilities should be MCP-first (sessions, approvals, annotations). Observe and batch capabilities should be equally first-class on the CLI (zero catalog cost, pipes, CI).

---

## 1. Entry dispatch: who owns which front door

OBSERVED, `scripts/rea.mjs` (earlier report §1 covers the MCP branches):

| argv shape | Route | Notes |
|---|---|---|
| `mcp` / `--mcp` (exactly one arg) | `dist/mcp/main.js runEntrypoint()` stdio | `:31-33,63-65`. This pre-empts Incur's own `--mcp` mode. |
| `mcp --http …` (only flags after `mcp`) | `runMcpHttpEntrypoint` | `:37-40,66-68`. `mcp add --http` still goes to Incur's `mcp add`. |
| `mcp doctor …` | `runProductionMcpDoctorCli` outside Incur | `:69-91`. Handles `--timings` itself. |
| `COMPLETE=<shell>` env | shell completion protocol | `:26-27`; `src/cliBuiltinArguments.ts:241-248` |
| anything else | Incur CLI (`dist/cli-runtime/cli.js`) | `:92-241` |

Before dispatch, the launcher performs these checks (OBSERVED):
1. It verifies that the per-mode compiled files exist, and prints a remediation with exit 1 when they do not (`:45-62`).
2. It validates global config. An invalid `REA_LOG_LEVEL` fails every command except `doctor`, which must be able to report it (`:122-138`; `src/cliConfiguration.ts:8-15`).
3. It validates output flags *before importing command code* (`:139-141`).
4. It answers a formatted `--version` without loading providers (`:111-121,142-143`). `rea --version --json` returns `{name, version, runtime, platform, architecture, supported_runtime}`, with no paths or environment values (`docs/cli.md:1135-1180`).

INFERRED: the launcher is a thin, dependency-free router. It does all argument admission that must not depend on the heavy runtime, so a broken build, a bad env var or a bad output flag still produces a structured answer.

---

## 2. Is the CLI generated from contracts? No. It is hand-written with contract reuse and test-enforced parity

### 2.1 Construction

OBSERVED, `src/cli.ts:113-253`:
- `Cli.create("rea", {version, description, globals: {timings}, mcp: {command, instructions}, sync: false, update: {install: throw}})` (`:132-158`).
  - `mcp.command` is the client-registration command that Incur's `rea mcp add` writes (`:141-147`).
  - `sync: false` removes Incur's built-in skill-sync commands (`:148`).
  - The Incur self-updater is disabled in favour of REA's verified `rea update` (`:149-157`).
- `--timings` middleware writes one `{"rea_timings":{"version":1,"command_ms":N}}` line to **stderr**, which keeps stdout parseable (`:160-172`; `docs/cli.md:64-72`).
- Registration goes through `registerToolFamilies({...})`, the same compile-time-complete family map MCP uses (`:178-247`; salvaged registry report). The `enhanced` family has no CLI registrar: "Enhanced workflows are MCP tools. Their CLI entry points live with core analysis" (`:186-187`).
- After registration, cross-cutting wrappers walk every registered command (`visitRegisteredCommands`, `:248-253`):
  - `presentOmittableDefaults`;
  - `streamCliCommandResults`, the shared result formatter (`src/cli/cliResultOutput.ts:84-140`);
  - `rejectSurplusCliArguments`, which enforces strict positionals;
  - `presentCliArgumentErrors`, which rewrites Incur `ValidationError` into field errors (`src/cli/argumentErrors.ts:28-40`);
  - `excludeDeclaredSensitiveArguments`, which redacts `--sensitive-value` literals from diagnostics (`src/cli/explicitSensitiveArguments.ts:17-25`);
  - `encodeCliArgumentErrors`.

### 2.2 A typical command

OBSERVED, `src/cli/sqliteCommands.ts:15-89`. Every command module repeats this pattern:
1. It names the command from the frozen `CLI_COMMANDS` table (`src/cliCommandNames.ts:2-188`, 185 kebab-case names; `CLI_COMMAND_NAMES` at `:191`).
2. It declares `args` and `options` as Zod, mostly re-declared with CLI-specific descriptions. Some fields reuse the contract field schema (`workerTimeoutMs: inspectSqliteDatabaseInputSchema.shape.worker_timeout_ms`).
3. In `run`, it wraps the work in `withCommandCancellation`, then `logCliCommand`, then `finishCliEvidenceCommand`. The last of these resolves caller paths and binds the Evidence caller path.
4. It maps each camelCase option to the snake_case canonical input field by hand (`...(options.rowLimit === undefined ? {} : { row_limit: options.rowLimit })`).
5. It calls the shared service (`createSqliteDatabaseService(environment).inspect(...)`) and projects the result (`projectCliAnalysisResult`).

There is a "capability module" pattern that pairs a contract with a CLI name, but only a few modules use it (OBSERVED: `src/composition/wasm.ts:18-20`, `evm.ts:25`, `apkSignature.ts:13`). It has the shape `{contract: INSPECT_WASM_ARTIFACT_CONTRACT, cliCommand: CLI_COMMANDS.inspectWasmArtifact, create(env)}`. The doc comment says "CLI/MCP share this binding" (`wasm.ts:13-17`).

Commands that take one JSON document use generic registrars (OBSERVED, `src/cli/applicationCommands.ts:187-225`). They accept one positional `inputJson` ("Inline workflow JSON or JSON file path"), parse it with `parseCliJsonInput`, validate it against the canonical schema, run the workflow and project errors. This is REA's closest thing to a generated CLI: "accept the MCP input object verbatim".

### 2.3 Parity and drift controls

OBSERVED:
- `tests/boundary/cli/cliToolCoverage.test.ts:102-134`: every MCP tool is CLI-reachable (by name, or through the alias table at `:9-24`) or listed as intentionally MCP-only (`:27-99`). The test also checks the reverse: an MCP-only entry must not secretly have a CLI command, must be unique, and must name a real contract. The listed reasons:
  - "Runtime settings belong to the long-lived MCP server";
  - "MCP session control selects a resource root; CLI resource commands take paths directly";
  - "Investigation reset acts on the long-lived MCP server's retained history".
- `tests/boundary/filesystem/productCatalog.test.ts:127-157`:
  - the Incur router's primary names equal `CLI_COMMAND_NAMES`, with zero aliases;
  - every arg and option has a non-empty description;
  - every deep-analysis command exposes `--provider`;
  - `trace` must *not* have a private `--limit`.
- `scripts/lib/catalog-cli.mjs:5-21` reads the inventory from Incur's actual router (`Cli.toCommands`). `scripts/lib/product-catalog.mjs:8-15` asserts that inventory equals `CLI_COMMAND_NAMES` when generating `docs/public/product-catalog.json`.
- CLI command names are hashed into the catalog identity digest (`src/catalogIdentity.ts:11,124`), so a CLI rename changes the advertised server identity.
- `tests/boundary/cli/cliOptionAliases.test.ts:54-91`: short flags are single characters only, because Incur stacks `-abc`. Every option that `--help` lists has a description.

### 2.4 Observed costs of the hand-written approach

OBSERVED unless marked:
- **Naming drift between and within commands.** `inspect-wasm-artifact` has `commandTimeoutMs` (camelCase, which surfaces as `--command-timeout-ms`) alongside `glue_paths`/`candidate_paths` (snake_case) in the same options object (`src/cli/wasmCommands.ts:74-80`). Evidence delivery uses `--evidence_output` (`src/cli/analysisEvidenceDelivery.ts:48`; `docs/cli.md:1343`). Incur accepts both spellings (`cliOptionAliases.test.ts:93-106`), which hides the inconsistency rather than removing it. INFERRED.
- **Per-command glue for every field.** A field added to a contract does not appear on the CLI until someone edits the command module. The parity test checks only that a command exists, not that its fields match. INFERRED from §2.2 and from the absence of any field-parity test in what I read. UNKNOWN whether one exists elsewhere.
- **Framework workarounds dominate the root file.** `src/cli.ts:259-627` is ~370 lines of guards around Incur's `mcp add`:
  - a trailing valued flag swallows the next flag (`:312-372`);
  - the `--agent=amp` form is silently ignored (`:373-384`);
  - Amp registration overwrites the `env` map and reports false success, so REA backs up, verifies the readback and restores `env` (`:385-613`).

  `src/cliBuiltinArguments.ts:79-121` *mirrors* Incur's internal builtin-flag pass so that REA can predict how Incur will tokenize.
- **Inconsistent cancellation exit status.** It is documented rather than fixed: `inspect-artifact`'s logging wrapper sits outside the cancellation handler, so it exits 1 instead of 130 (`docs/cli.md:1368-1372`).

INFERRED conclusion: REA gets parity through architecture (shared services, shared error projection, shared family map) and tests. It does not get it through generation, and it pays in glue code and naming drift. A framework whose purpose is "make it easy" should generate the CLI projection.

---

## 3. Output formats for agents

OBSERVED, `src/cliOutput.ts` and `docs/cli.md:74-167,1181-1396`.

| Control | Behavior | Source |
|---|---|---|
| default | **TOON** (Incur's token-oriented notation) | `cliOutput.ts:123`; `docs/cli.md:1195` |
| `--json` / `--format json` | Indented on a TTY, compact when piped ("keeps large results smaller for agents"). Streams incrementally. | `cliOutput.ts:46-53`; `docs/cli.md:1196-1206` |
| `--format yaml\|md\|jsonl` | `md` escapes table syntax and refuses NUL and unpaired surrogates. TOON also refuses unpaired surrogates. | `docs/cli.md:1208-1224` |
| `--format jsonl` | One record per row of the *single* clear `rows`/`items`/`records`/`occurrences` array. A trailing `{"$rea":"jsonl/v1","type":"metadata","records":{path,emitted,source,filter},"context":{…non-row fields…}}` record follows. Errors are `{"$rea":"jsonl/v1","type":"error","ok":false,…}`. A missing final metadata record means "interrupted, do not trust as complete". | `docs/cli.md:74-167`; `src/cli/jsonLines.ts` |
| `--format csv\|tsv` | Requires `--filter-output <row-array>` and `--columns a,b`. The first line is `# rea-delimited-v1 <JSON metadata>` (coverage, limitations, unknowns, command, duration). Formula-leading cells are refused. Nested values are never flattened. | `cliOutput.ts:311-318`; `docs/cli.md:1222-1253` |
| `--filter-output a.b,rows[0,2].name` | Prunes success data. Classified failures keep their full typed projection even if the filter would remove it. | `docs/cli.md:1264-1276` |
| `--full-output` | Adds the outer `{ok, data\|error, meta:{command,duration}}` envelope. | `docs/cli.md:1267-1268`; `src/mcpDoctor.ts:381-390` |
| `--token-count` | Estimate (the `tokenx` library) of the exact emitted document. | `docs/cli.md:1380-1390` |
| `--token-limit/--token-offset` | Approximate token window over rendered text, sliced on tokenx segment boundaries and never splitting a surrogate pair (`src/cli/tokenWindow.ts:1-40`). **Rejected for json/jsonl/yaml/csv/tsv** ("Token windows cannot preserve structured output. Remove --token-limit/--token-offset and use --filter-output or command pagination.", `cliOutput.ts:18-19,326-343`). | |
| `--timings` | One JSON line on stderr. | `src/cli.ts:160-172` |

Admission rules (OBSERVED, `src/cliOutput.ts:266-354`):
- Output flags are parsed and validated at the executable boundary before Incur dispatch.
- Errors render in the *requested* structured format (a JSON or YAML error object, or a JSONL error record), with codes `INVALID_OUTPUT_VALUE`, `INVALID_OUTPUT_FILTER`, `MISSING_OUTPUT_VALUE` and `UNSUPPORTED_OUTPUT_COMBINATION` (`:84-95,372-382`).
- "A formatting refusal exits nonzero even when the underlying operation succeeded" (`docs/cli.md:1203-1204`).

Agent detection (OBSERVED, `src/cli/cliResultOutput.ts:96-140`): the result wrapper passes `renderOutput: context.agent || inheritedPolicy !== "agent-only"`. Incur exposes an `agent` flag on the command context, so commands can render differently for agents. INFERRED: probably a non-TTY heuristic; I did not read Incur's code. UNKNOWN which REA commands use `agent-only`.

Incur features REA inherits (SECONDARY, Incur README at https://github.com/wevm/incur, fetched 2026-11-05):
- `--llms` (agent-readable command manifest, Markdown or JSON schema);
- `--schema` (JSON Schema of a command's args, options and output);
- `--mcp`;
- `mcp add`;
- `skills add`;
- the `{ok,data,meta}` envelope;
- call-to-action and `hint` next steps on errors;
- `_meta.error` carrying `code`, `retryable` and `exitCode` for MCP calls.

The vendor claims "up to 3× fewer tokens per session" vs MCP and "up to 60% fewer tokens" for TOON vs JSON. These are unverified vendor numbers.

REA keeps `--llms`, `--llms-full`, `--schema` and `--help` as builtins (`src/cliBuiltinArguments.ts:18-32`; completions include `--llms` at `src/cli/completions.ts:44-45`). It refuses `--schema` only for `mcp add` (`src/cli.ts:278-311`). INFERRED: `rea --llms` and `rea <cmd> --schema` work as Incur defines them. I did not run them.

---

## 4. Exit codes and error contract

OBSERVED:
- **Table** (`docs/cli.md:1278-1282`):
  - `0`: completed; the result may include partial evidence, warnings or unknowns.
  - `1`: could not complete; structured output names the cause.
  - `128+N`: signal; 130 and 143 are assigned by handled cancellation after cleanup.
  - `141`: the pipe reader closed early; REA prints nothing more (`docs/cli.md:1362-1366`; `scripts/rea.mjs:212-215,254-264`).
- **Errors are values.** Commands return `{error: "Analysis failed", ...projectAnalysisError(e)}` (`src/cliEvidenceCommands.ts:123-126`) or a raw projection. `logCliCommand` classifies the value by validating it against `analysisErrorProjectionSchema` or `analysisCliErrorEnvelopeSchema`, or by a nested `error` or `healthy:false` field. On a match it sets `process.exitCode = 1` and logs `status:"error"` (`src/cliLogging.ts:37-80,104-148`). A thrown `AnalysisError` is converted into a projection too (`:140-145`). The CLI and MCP therefore share one error vocabulary (`code`, `category`, `retryable`, `details`); the earlier contracts report covers the projection.
- **Cancellation** (`src/cli/commandCancellation.ts:9-32`): SIGINT/SIGTERM trigger an `AbortController`, the operation is awaited to settle, and only then is `exitCode` set to 130 or 143. A structured `cancelled` result may be printed first. The docs list exactly which command families use the handler (`docs/cli.md:1318-1341`): "This describes the current source routing; it is not a claim that every command…has been independently exercised."
- **Per-command exceptions** (documented, `docs/cli.md:1368-1378`):
  - `setup --dry-run` returns `planned` and exits 0;
  - a cancelled setup exits 0;
  - `needs_confirmation`/`needs_human` exit 1;
  - `doctor` exits 1 only when checks in its selected scope fail.
- **Long-running commands**: `rea mcp --http` and `environment-serve` exit 130 or 143 after closing listeners, and 1 if cleanup fails. `environment-serve` prints one `{"status":"stopped",…}` document only after cleanup is confirmed (`docs/cli.md:1284-1316`).
- **Recommended scripting**: `set -o pipefail` (`docs/cli.md:1380-1386`). "Treat the structured result and process status as separate signals."

INFERRED: the useful contract is that `0` does not mean `complete`. Completeness lives in the payload (`coverage`, `limitations`, `unknowns`), and the exit code only says "the operation ran to a result". This matches the Evidence model, and an agent framework should keep the same split.

---

## 5. Input: positionals, flags, JSON, stdin

OBSERVED:
- **Strict positionals.** Surplus positionals are rejected before execution (`docs/cli.md:1183-1185`; `src/cli/strictPositionals.ts`).
- **Flag value forms.** `--flag value` and `--flag=value` both work. REA normalizes `=` forms for output flags because Incur only reads the spaced form (`src/cliBuiltinArguments.ts:123-162`).
- **JSON input** (`src/cliJsonInput.ts:31-125`): one value may be inline JSON, `-` (stdin), or a file path. Diagnostics distinguish `invalid-json`, `too-large`, `read-failed` and access-denied (`EACCES`/`EPERM`), always with `input_path` and `input_reason`.
  - Inline JSON that looks like JSON (`{`/`[`), is not an existing file and has no `.json` extension is reported as invalid JSON rather than "file not found" (`:111-116`).
  - An optional `snapshot` mode seals the parsed value (`sealOwnedJsonInput`) so later mutation cannot change what was validated (`:59-69,238-240`).
- **Stdin limits** (`:127-245`): 8 MiB by default. Heap headroom is checked against `nativeJsonParseRequiredBytes` during reading and again before concatenation and parse. Abort destroys the stream to wake a pending read. Stdin is read once as a single UTF-8 document without a BOM (`docs/cli.md:43-52`).
- **Path relativity bridge** (`:273-313`): `resolveCliJsonPaths(value, [["source","path"], …])` makes named string fields absolute against the operator's cwd. URLs and blank values pass through so the shared schema can reject them. The comment states the reason: "Shared MCP contracts reject relative paths, so CLI workflows resolve operator-relative values before validation." Files referenced from saved requests behave the same way (`docs/cli.md:36-40`).
- **Empty paths and NUL bytes** are rejected before any resolution. "An empty string never selects the working directory" (`docs/cli.md:54-57`).
- **Sensitive literals.** A repeatable `--sensitive-value X` declares literals that diagnostics must not echo (`src/cli/explicitSensitiveArguments.ts:17-25`; `src/cliBuiltinArguments.ts:212-239`).

INFERRED pattern: **canonical input = the MCP input object**. The CLI is a convenience projection, and three escape hatches keep it complete: an inline/file/stdin JSON document, path-field resolution, and the same validator. That is the generation rule recommended in §10.

---

## 6. Paging and large results

OBSERVED:
- **Domain paging.** Views and inventories take `offset`/`limit` and return `coverage {status, examined, total, next_offset, exhausted}`. The JSONL metadata record preserves that coverage (`docs/cli.md:107-125`).
- **Save, then project.** The CLI is stateless across invocations: "Each CLI invocation is a separate process; save results when you want to use them in a later command" (`docs/cli.md:3-6`). The canonical flow is `analyze-… --json > ev.json`, then `inspect-analysis-view '{"source":{"kind":"evidence-file","path":"ev.json"},"view":{"kind":"summary"}}'`. The file is authenticated, and file-sourced Evidence is "bounded by the file reader rather than charged as inline input" (`docs/cli.md:26-41`).
- **Summary requires a sink.** `--detail summary` is refused unless `--evidence_output` is given: "Summary delivery requires --evidence_output so complete source Evidence remains available" (`src/cli/analysisEvidenceDelivery.ts:43-53,70-80`). Publication is atomic. Cancellation before commit leaves no file. A commit can win over a later cancellation, in which case the result carries the path and byte count (`docs/cli.md:1343-1349`).
- **MCP counterpart.** Oversized MCP results become an `AnalysisResourceConstraintError` with a `nextStep` (export the bundle, use `inspect_analysis_view`, or use CLI JSON). Results are never truncated (`rea-mcp-server-adapter.md` §overflow). The skill tells the agent not to repeat a call "merely to request the same oversized response" (`.agents/skills/reverse-engineer-anything/SKILL.md:441`).
- **Token windows** are the last-resort transport pager, and only for text formats (§3).
- **Completion has no paging** (MCP `completion/complete` returns at most 100 values plus `total`/`hasMore`; `docs/mcp-prompts.md:155-170`).

INFERRED: REA's rule is **page in the domain (stable identities and coverage), never truncate silently, and give stateless callers a file-backed handle**. The CLI's `--evidence_output` plays the role of the MCP server's retained Evidence ledger.

---

## 7. State: why 68 tools are MCP-only

OBSERVED: the MCP-only list (`cliToolCoverage.test.ts:27-99`) falls into these groups:
- **Session/target lifecycle**: `open_binary`, `close_binary`, `binary_session`, `open_bundle_resources`, `reset_investigation`.
- **Cursor navigation**: `current_address`, `goto_address`, `next_address`, `prev_address`, `current_procedure`, `current_document`.
- **Provider-database annotation (mutation)**: `set_comment`, `set_address_name`, `set_bookmark`, `unset_bookmark`, `set_inline_comment`, `set_addresses_names`.
- **Evidence-ledger operations**: `get_evidence_bundle`, `list_unknowns`, `record_unknown`, `update_unknown`, `verify_unknown_resolution`, `export_evidence_bundle`, `import_evidence_bundle`. The CLI has file-based `evidence-import/export/compare` instead (`src/cliEvidenceCommands.ts:26-120`).
- **Persistent debugger and instrumentation sessions**: `start_gdb_session`, `gdb_session_status`, `close_gdb_session`, and the Rizin and Frida equivalents. The CLI offers one-shot `debug-with-gdb`/`debug-with-rizin`/`instrument-with-frida`. "Persistent debugger sessions exist only for the lifetime of the REA MCP server process" (`docs/reverse-engineering-tools.md:3`).
- **Runtime settings**: `inspect_analysis_settings` and `update_analysis_settings`. The CLI uses launch-time and per-call controls instead.

INFERRED: the CLI/MCP split is in practice a **one-shot vs session** split. Anything that needs a live handle (a cursor, an attached debugger, a retained ledger, a mutable settings scope) stays on the long-lived server. For agentic-anything this matters more than it did for REA: *driving* software is mostly stateful (an attached app, a focused window, an open document). A CLI-only design would need a daemon (see §10.4).

---

## 8. Discoverability surfaces

OBSERVED:
- **CLI**:
  - `rea --help` and `rea <cmd> --help` (descriptions are required by test, §2.3);
  - shell completions for bash, zsh, fish and nushell. Completion "is a discovery aid; use `rea COMMAND --help` to confirm" (`docs/cli.md:169-203`);
  - Incur `--llms`/`--schema` (SECONDARY/INFERRED, §3);
  - `rea capabilities` and `rea providers` (`src/cliCommandNames.ts:30-31`).
- **MCP**:
  - `tools/list` is the complete stable catalog;
  - a target-free `binary_session {}` call returns `result.tool_availability` with per-operation availability, reasons and client requirements for the current state (`docs/installation.md:370-378`);
  - schema profiles and family scoping bound the catalog's size (`rea-mcp-server-adapter.md`).
- **Prompts in three channels** (`docs/mcp-prompts.md:1-15,55-75`): six workflow prompts. The same contracts and renderer serve MCP `prompts/list|get`, a `guided_workflows` tool (`action: list|render`) and `rea guided-workflows`. Under family scoping, a prompt is advertised only if *every* tool it suggests is in scope, "so a rendered prompt never directs the agent to a disabled call".
- **The skill as a searchable corpus** (`docs/mcp-prompts.md:20-50`): `search_guidance` and `read_guidance_section` (CLI: `rea guidance --query …`, then `--action read --locator …`) index the *installed* skill bundle. Locators are hash-bound to the bundle, and admission is bounded (1 MiB per file, 4 MiB per bundle, 4,096 sections). An agent can therefore pull only the relevant section of a 539-line skill plus 7 references instead of loading everything.
- **Client naming caveat** (`docs/client-configuration.md:88-104`): hosts rename tools (e.g. `mcp__rea__binary_session`), and REA cannot enforce prefixes or approval-wildcard isolation.

---

## 9. Packaging for agent hosts

### 9.1 Front doors from one artifact

OBSERVED (some of this table restates the setup report; the plugin and skill mechanics are new here):

| Front door | File | Mechanism |
|---|---|---|
| CLI | `package.json` `bin.rea`, `bin.rea-agents` = `scripts/rea.mjs` | npm global or `npx -y rea-agents@…` |
| stdio MCP | `rea mcp` | registered by `rea setup` as `<abs runtime> <abs entry> mcp`, or by `rea mcp add` (Incur) |
| HTTP MCP | `rea mcp --http [--host --port --allowed-hosts]` | loopback by default, `Host`/`Origin` checks, one client session at a time, client URL announced as one JSON line on stderr (`docs/cli.md:979-1016`) |
| Claude Code plugin | `.claude-plugin/plugin.json:18-27`; `marketplace.json:9-20` | `"skills": "./skills"`; `mcpServers.rea = node ${CLAUDE_PLUGIN_ROOT}/scripts/plugin-mcp-launcher.mjs 6.8.0`; marketplace source `{source:"npm", package:"rea-agents", version:"6.8.0"}` |
| Codex plugin | `.codex-plugin/plugin.json:18-39`; `.codex-plugin/mcp.json:1-13`; `.agents/plugins/marketplace.json:1-21` | same launcher with `cwd: "."` and `startup_timeout_sec: 30`. `interface` block: `displayName`, `capabilities: ["Interactive","Read","Write"]`, three `defaultPrompt` examples, icons. Marketplace `policy: {installation: "AVAILABLE", authentication: "ON_INSTALL"}` |
| MCP Registry | `server.json:1-30` | schema `2025-12-11`, `name: io.github.morluto/rea` (= `package.json` `mcpName`), npm package, `runtimeHint: npx`, `transport: stdio`, positional `mcp` |
| skills.sh | `.agents/skills/reverse-engineer-anything/` | `npx skills add morluto/rea --skill reverse-engineer-anything` installs instructions only |
| curl installer | `install.sh` | brace-group truncation guard (`:1-10`), Node engine floor mirrored from `package.json` ("change both"), then `rea setup` only when a TTY is present (`:177-181`) |

### 9.2 Plugin launcher

OBSERVED (`scripts/plugin-mcp-launcher.mjs:8-47`):
1. It takes exactly one argument, an exact semver pin, which Release Please rewrites.
2. It verifies that the bundled `package.json` is `rea-agents@<pin>`. On mismatch it says "version pin does not match its installed rea-agents package; reinstall the plugin".
3. It verifies that `dist/mcp/main.js` exists.
4. It then sets `process.argv = [node, rea.mjs, "mcp"]` and imports the dispatcher in-process: "The runtime and skill come from the same installed artifact, without another npx child".

INFERRED: this guarantees that skill text and server version cannot drift inside a plugin install, which a raw `npx …@latest` registration cannot do.

### 9.3 How a capability becomes skill content

OBSERVED:
- **Authoring.** The skill is hand-authored in `.agents/skills/reverse-engineer-anything/SKILL.md`: YAML front matter `name`, `description` and `metadata.version: "60"`, a 539-line body and 7 reference files. The body is routing (target type to first tool), connection and recovery, and epistemic rules. It refers to MCP tool names and gives one concrete CLI fallback (`SKILL.md:1-110`).
- **Generation.** `npm run artifacts:skills` runs `scripts/generate-skill-metadata.mjs`, which:
  - reads the built `FULL_CATALOG_IDENTITY` and stamps the MCP tool count into front matter (`addSkillManifestToolCount`, `src/domain/skillManifest.ts`);
  - rewrites the `@latest` package specifier to the exact registration specifier ("A shipped skill must run the version it ships with", `:31-34`);
  - rebuilds `skills/reverse-engineer-anything/` from scratch so removed references cannot survive;
  - offers a `--check` mode that fails on drift (`:19-66`).
- **Verification.** `scripts/verify-repository-skill.mjs` checks that the Skills CLI installs the authored bundle byte-for-byte (setup report). The product-catalog test rejects skill links that resolve only inside the repo, not inside an installed bundle (`tests/boundary/filesystem/productCatalog.test.ts:160-175`).
- **Policy.** Skills carry judgment, contracts carry discoverability, and the skill tells the agent to trust the live catalog over the skill text (`SKILL.md:27-30`; salvaged tool-design report).
- **Separate skills for maintainers.** `.agents/skills/rea-tool-design`, `rea-ci-tests-audit`, `rea-analysis-optimization` and `rea-changelog-update` are maintainer skills. They point into repo docs (`rea-tool-design/SKILL.md:1-34`) and are not shipped.

So in REA a capability does **not** become a skill automatically. A new tool becomes visible to the skill only through the stamped tool count and the live catalog. Routing lines are written by hand. INFERRED: this is deliberate, because the skill encodes "which first tool for which target", and that knowledge does not exist in any single contract. A generator can still produce the mechanical parts: the route index from per-contract `use_when` hints, pins, counts and the CLI fallback lines.

### 9.4 Doctors and startup policy

OBSERVED:
- **`rea doctor`** (`src/cli/setupCommands.ts:117-149`; `src/doctorRuntime.ts:24-120`; `docs/installation.md:363-410`): read-only. Repeatable `--client`, `--provider` and `--skill` define a scope, which yields `scope.mode:"explicit"`. `scope_checks` decide `healthy` and the exit code; everything else goes to `informational_checks`; `environment_healthy` summarizes the full audit.
  - It cannot see the client-launched server's environment, so a mismatch is reported as a non-blocking `registration_environment:<client>` check with `classification: "unknown"`.
- **`rea mcp doctor`** (`src/mcpDoctor.ts:84-200,236-347,353-405`):
  - spawns `node <dispatcher> mcp` through an owned stdio transport with a target-free environment and a deadline budget;
  - connects a real SDK `Client`, lists tools and prompts (prompts only if advertised, because the SDK otherwise logs to stdout and corrupts output, `:241-249`) and calls `binary_session {}`;
  - checks `initialize` (name, version, protocol), `inventory:tools` and `inventory:prompts` (expected vs observed, with missing, unexpected and duplicates), `server-identity` (package version plus catalog digest) and `request-flow`;
  - closes the client and the transport separately and aggregates cleanup failures;
  - returns `{output, exitCode}`, with exit 0 only when healthy.
- **Startup policy** (`src/mcpStartupPolicy.ts:12-65`): Codex startup timeout 30 s, Kimi 30,000 ms, tool-operation headroom 120 s, doctor stderr cap 64 KiB. `recommendedMcpToolTimeoutMs` = the maximum of the selected provider startup and decompile budgets, the JEB MCP timeout and the provider request timeout, plus headroom. `setup` uses this to write client tool timeouts. The Codex figure is labeled "client guidance REA does not enforce".
- **Client deadline vs server deadline** (`docs/client-configuration.md:77-86`): `update_analysis_settings` cannot extend the *client's* tool-call deadline. Operators must raise both.

### 9.5 Token cost data REA measured

OBSERVED (`docs/client-configuration.md:230-265`): with Claude Code tool search off, 172 tools cost about 496 KB per request with full input schemas and about 290 KB with compact ones, roughly 120k vs 68k tokens. Compaction "cannot remove tool definitions", which leads to autocompact thrashing. The remedies are tool search (deferred loading), the compact input profile, `--tool-families` scoping, and an output-schema profile `compact|off` for clients whose stdio reader rejects the catalog frame.

INFERRED: this is the strongest quantitative argument in the repo for having *some* zero-catalog-cost path (the CLI, or a small router-tool set) alongside a large MCP catalog.

---

## 10. Recommended design: one contract → CLI + MCP + skill (+ manifests, doctor)

Everything in this section is INFERRED design, grounded in the REA observations cited above.

### 10.1 The contract (single source)

```ts
defineCapability({
  id: "document.export",                 // canonical; MCP name document_export, CLI `aa document export` or `export-document`
  title: "Export the active document",
  summary: "Export the focused document to a file in a chosen format",   // ≤120 chars: --llms, skill index, tool description lead
  description: "...",                    // full MCP description
  authority: "mutate",                   // observe | launch | attach | inject | mutate (safety-rails report)
  effects: { target_write: true, host_fs_write: true, network: false,
             idempotent: false, irreversible: false },             // → MCP annotations, CLI confirmation policy
  lifetime: "session",                   // one-shot | session  (session ⇒ needs MCP server or daemon)
  input: z.object({ session_id, format: z.enum([...]), output_path: absPath }), // canonical, snake_case, absolute paths
  output: actionRecordSchema(exportResult),                                     // Evidence/ActionRecord envelope
  paging: undefined,                     // or { collection: "items", offset: "offset", limit: "limit", next: "next_offset" }
  errors: ["target_unavailable", "permission_denied", "invalid_request", "cancelled"],
  returns_target_content: false,         // → untrusted-content marking on MCP, stderr notice on CLI
  cli: {                                 // projection hints only, never a second schema
    positionals: ["output_path"],
    path_fields: [["output_path"]],       // resolved against cwd before canonical validation (cliJsonInput.ts:273-313)
    short: { f: "format" },
    // or: cli: { omit: "requires live cursor; use MCP or `aa session`" }  ← typed, replaces REA's test-side list
  },
  skill: { use_when: "user wants a file copy of what is open", follow_up: ["document.inspect"] },
  examples: [{ title: "PDF export", input: {...} }],     // validated by test; rendered in --help, docs, skill
})
```

The rules REA motivates:
- **Canonical input is the MCP object.** The CLI is derived from it (REA §5). Never declare a second CLI schema, which is where REA's camelCase/snake_case drift comes from (§2.4).
- **Lifetime is part of the contract.** REA's 68 MCP-only tools are implicit `lifetime: session` (§7).
- **`cli.omit` carries a reason string.** This moves `MCP_TOOLS_WITHOUT_DEDICATED_CLI` (`cliToolCoverage.test.ts:27-99`) into the contract, where the generator and the docs can show it.

### 10.2 Projections (generators)

| Target | Derived from | Generation rule |
|---|---|---|
| MCP tool | id, title, description, input, output, effects, authority | `registerTool(snake(id), {inputSchema, outputSchema, annotations: deriveAnnotations(effects, authority)})`. Advertised-schema profiles (compact/full) and family scoping as in REA. Server-side canonical validation is always on. |
| CLI command | id, input, cli hints | Name = kebab(id) (or `group verb`). Each top-level scalar or enum field becomes `--kebab-flag` (bool → `--x/--no-x`; array → repeatable). Each object or union field becomes `--<field>-json <inline\|file\|->`. **There is always `--input-json <inline\|file\|->`** carrying the whole canonical object, which makes the CLI complete by construction (REA's `registerJsonCommand` pattern, `src/cli/applicationCommands.ts:187-225`). Flags override `--input-json` fields. `path_fields` are resolved before validation. |
| CLI I/O | shared runtime, not per-command | REA's output stack: `toon` default; `--json` compact when piped; `jsonl` with a metadata trailer; `--filter-output`; `--full-output` envelope; token windows only on text; `--timings` to stderr. Admission runs before dispatch (`src/cliOutput.ts:266-354`). |
| Exit codes | error category | Fixed map: 0 = result produced (completeness lives in the payload); 1 = operation failed (typed error on stdout in the requested format); 2 = usage/validation error *before* any target I/O (REA folds this into 1, and separating it helps agents decide whether to retry); 3 = needs confirmation or authority (REA's `needs_confirmation`); 130/143 after cleanup; 141 on EPIPE. Generate the table into docs. |
| Paging | `paging` | MCP: the fields as-is plus `coverage`. CLI: identical flags (`--offset/--limit`); JSONL trailer carries `next_offset`; optional `--all` that iterates pages and streams JSONL (REA lacks this, so agents loop manually). |
| Large output | `output` size class | MCP: never truncate; return a resource or handle plus `nextStep` (REA overflow). CLI: `--output-file` for the complete result, with `--detail summary` allowed only when a file sink is set (`analysisEvidenceDelivery.ts:70-80`). |
| Skill | summary, `skill.use_when`, examples, cli.omit | Generate a **route table section** ("when X → `tool_id` / `aa cmd`") and a **CLI fallback section** between marker comments in SKILL.md. Hand-written judgment and recovery prose sits outside the markers. Stamp version, pinned specifier and tool count (`scripts/generate-skill-metadata.mjs:19-40`). Expose the bundle via `search_guidance`-style tools and a CLI command (`docs/mcp-prompts.md:20-50`). |
| Prompts | declared workflows referencing capability ids | Advertise only if every referenced id is in scope (`docs/mcp-prompts.md:66-72`); mirror as a `guided_workflows` tool and a CLI command. |
| Manifests | product identity (name, version, mcpName, icon, prompts) | Generate `.claude-plugin/plugin.json`, `.codex-plugin/{plugin,mcp}.json`, marketplaces and `server.json` from one `product.ts`. Plugins launch a version-pinned in-process launcher (`scripts/plugin-mcp-launcher.mjs:8-47`), never `npx @latest`. |
| Doctor | contracts plus identity | `aa mcp doctor`: spawn self, compare exact inventory with the generated expectation, verify the catalog digest via a target-free identity tool, run one target-free request (`src/mcpDoctor.ts:236-347`). `aa doctor`: per-adapter readiness probes declared next to the adapter, scoped as in REA. |
| Tests | everything | Generated: every contract is either CLI-projected or `cli.omit` (inverse of `cliToolCoverage.test.ts`); every field reachable on the CLI; examples validate; descriptions non-empty; catalog digest includes the CLI inventory (`src/catalogIdentity.ts:124`). |

### 10.3 Choosing a CLI framework

REA chose Incur and spent ~370 lines plus a mirrored tokenizer working around it (§2.4). For agentic-anything:
- **Option A**: generate directly onto a small owned parser. This gives full control of exit codes, `=` forms and value-swallowing, and the dependency surface stays small.
- **Option B**: use Incur, which supplies TOON, `--llms`, `--schema`, `--filter-output` and token windows for free. Pin it exactly, test parsing edge cases (the REA tests above are a ready checklist), and keep its `--mcp` and `skills add` disabled so the contract-first server and generated skill stay authoritative (`scripts/rea.mjs:31-33`; `src/cli.ts:148`).

Recommendation: start with B for speed, behind a thin `CliBackend` interface so the generator can retarget.

### 10.4 Stateful capabilities on the CLI

A CLI cannot hold a session between invocations. Options:
1. MCP-only, as REA does (`cli.omit` with reason).
2. **Daemon bridge.** `aa session start` launches the same server on loopback streamable HTTP with a random token (REA's HTTP transport: one client, Host/Origin checks). It prints `{session_url, session_id}` and exits. Session-lifetime CLI commands take `--session <id|url>` and act as thin MCP clients. `aa session stop` ends the session. The contract is identical, and the CLI becomes a generic MCP client for session tools.
3. File-backed state (REA's `--evidence_output` and `evidence-file` sources), suitable only for read-only derivations.

Recommendation: implement 2 for drive/mutate capabilities, because agents in shell-only sandboxes and CI still need to drive. Keep 3 for observation re-projection.

### 10.5 CLI-first vs MCP-first for agents

| Dimension | CLI-first | MCP-first | Evidence |
|---|---|---|---|
| Upfront token cost | ~0. The agent pays per `--help`/`--llms`/`--schema` lookup on demand. | The whole catalog every request unless the host defers (REA: 172 tools ≈ 120k tokens full / 68k compact with tool search off). | OBSERVED `docs/client-configuration.md:230-246` |
| Per-call token cost | Output lands raw in context unless the agent filters. Mitigated by TOON, `--filter-output`, jq, files and token windows. | Structured result plus `structuredContent` duplication; schema-validated. Mitigated by retained handles and views. | OBSERVED §3, §6; vendor 3× claim SECONDARY |
| Composability | Pipes, files, `jq`, loops, CI and `xargs`; results can bypass the model entirely (`> ev.json`, then project). | Data must round-trip through the model unless the server retains it by id (REA Evidence ids, retained views). | OBSERVED `docs/cli.md:26-52` |
| Discoverability | Needs a skill or `--llms`; names are greppable; shell completion is for humans. | Typed schemas and descriptions are in front of the model; `tool_availability` gives live state; prompts and completion. | OBSERVED §8 |
| Safety/approval | Coarse: the host approves "run `aa …`" via a Bash allowlist, with no per-tool annotations. Authority must be enforced inside the CLI (confirmation flags, exit 3). | Per-tool annotations (`destructiveHint`, `readOnlyHint`) drive host approval UIs; untrusted-content marking (`returnsTargetContent`). | OBSERVED `toolContractTypes.ts:58-64`; INFERRED for CLI |
| State | One-shot; needs a daemon or files. | Natural sessions, cursors, attached debuggers. | OBSERVED §7 |
| Progress/cancel | stderr lines; SIGINT → 130 after cleanup. | `notifications/progress`, `notifications/cancelled`, threaded `AbortSignal`. | OBSERVED §4; adapter report |
| Host coverage | Any agent with a shell, plus CI and humans. | Only MCP-capable hosts; per-host config dialects (REA supports 25) and schema-dialect limits (compact profile). | OBSERVED setup report; `docs/client-configuration.md` |
| Failure modes | PATH and runtime differences in child shells; output corruption by stray stdout. | Startup timeouts, frame size limits, host renaming of tools, approval policies blocking calls. | OBSERVED `client-configuration.md:9-50,88-104`; `connection-and-recovery.md` |

**Recommended stance for agentic-anything:**
- Contract-first. Generate both surfaces.
- Distribute **MCP as the default for interactive driving**, because it is stateful, approval-aware and typed.
- Keep the **CLI as a first-class equal for observe, batch, CI and shell-only hosts**.
- Use the **skill** as the router that teaches both and names the CLI fallback.
- Keep the MCP catalog small with a router or meta tool plus family scoping, so the per-request cost does not reproduce REA's 120k-token problem.

---

## What to distill for agentic-anything

1. **One capability contract and generated projections.** Fields: `id`, `title`, `summary`, `description`, `authority`, `effects`, `lifetime`, `input`, `output`, `paging`, `errors`, `returns_target_content`, `cli` (hints or `omit: reason`), `skill.use_when`, `examples`. Generate the MCP registration, CLI command, docs, skill route section, manifests and doctor expectations from it (§10.1-10.2). Do not hand-write CLI commands per capability, which is REA's main maintenance cost (§2.4).
2. **The canonical input is the MCP object, and the CLI always has `--input-json <inline|file|->`.** Derived flags are sugar. Resolve declared path fields against cwd before the shared validator (`src/cliJsonInput.ts:273-313`). Cap stdin (8 MiB) and check heap headroom (`:127-164`).
3. **Copy the parity test, then make most of it unnecessary.** Assert that every contract is CLI-projected or explicitly `cli.omit` with a reason. Assert that the router's real inventory equals the generated list, and that every field and option has a description (`tests/boundary/cli/cliToolCoverage.test.ts:102-134`; `productCatalog.test.ts:127-136`). Hash CLI names into the server identity digest (`src/catalogIdentity.ts:124`).
4. **A shared CLI output stack, admitted before dispatch.** TOON default; `--json` compact when piped; `jsonl` with `$ns` control records and a mandatory trailing metadata record ("missing trailer = incomplete"); csv/tsv only with an explicit row path, columns and a metadata header; `--filter-output`; `--full-output` envelope; token windows refused for structured formats; `--timings` on stderr. Output-flag errors are structured in the requested format and exit nonzero (`src/cliOutput.ts:23-35,266-382`).
5. **An exit-code contract where 0 ≠ complete.** Completeness lives in payload coverage. Use 0/1/2(usage)/3(needs confirmation or authority)/130/143/141. Set the signal status only after cleanup settles (`src/cli/commandCancellation.ts:9-32`). Handle EPIPE quietly with 141 (`scripts/rea.mjs:254-264`). Route *every* command through the same wrapper to avoid REA's `inspect-artifact` and `setup` exceptions.
6. **Errors as typed values with a single classifier** that validates against the canonical error schema to decide the exit status (`src/cliLogging.ts:37-148`). The same projection goes to MCP `isError` results.
7. **Paging in the domain, plus file sinks for stateless callers.** `offset`/`limit`/`next_offset`/`coverage` in every collection result. A summary view only when the complete record is persisted (`--output-file`; `analysisEvidenceDelivery.ts:70-80`). A save-then-project command that reads an authenticated saved record (`docs/cli.md:26-41`). Add an `--all` JSONL page-streamer, which REA lacks.
8. **Lifetime in the contract; daemon bridge for session capabilities on the CLI.** Expose `aa session start|stop` over loopback streamable HTTP (one client, Host/Origin checks, token), and give session commands a `--session` flag (§10.4). REA keeps 68 tools MCP-only for exactly this reason (§7).
9. **A thin launcher that owns admission.** Route MCP stdio, MCP HTTP, doctor and CLI before importing heavy code. Check that build files exist (remediation, exit 1). Validate global env and output flags first. Answer `--version --json` without providers (`scripts/rea.mjs:12-143`).
10. **Packaging from one product identity.** Generate the Claude and Codex plugin manifests, marketplaces and `server.json`. Plugins launch an in-process, exact-version-pinned launcher that verifies the bundled package version (`scripts/plugin-mcp-launcher.mjs:8-47`). Release tooling rewrites the pins atomically (setup report).
11. **Generated skill with hand-authored judgment.** Source in `.agents/skills/<name>/`, generated into `skills/<name>/`. Stamp the version, tool count and pinned specifier, and rebuild the output directory from scratch with a `--check` drift mode (`scripts/generate-skill-metadata.mjs:19-66`). Add marker-delimited generated sections (route table, CLI fallbacks). Expose the installed bundle via search/read tools and a CLI command so agents load sections, not the whole skill (`docs/mcp-prompts.md:20-50`). Keep the rule "trust the live catalog over the skill".
12. **Prompts with fallbacks.** Native MCP prompts, plus a `guided_workflows` tool, plus a CLI command, all from one renderer. A prompt is advertised only when all of its referenced tools are in scope (`docs/mcp-prompts.md:1-15,66-72`).
13. **Two doctors.** A read-only scoped host doctor (`scope_checks` vs `informational_checks`; unknowns for environments it cannot observe), and an MCP doctor that spawns the real server as a client and checks initialize, exact inventory, identity digest and one target-free request, with bounded stderr retention (`src/mcpDoctor.ts:236-347`; `src/mcpStartupPolicy.ts:12-17`).
14. **Deadline guidance generated from adapter budgets.** Compute recommended client tool timeouts from declared startup and request budgets plus headroom, and write them into client configs at setup (`src/mcpStartupPolicy.ts:34-65`). Document that server settings cannot extend client deadlines.
15. **If adopting Incur, pin it and disable its parallel paths.** Disable `--mcp` and `skills add`, and route `update` to your verified updater. Reuse REA's parser edge-case tests as a checklist: valued flag at the end, `=` forms, stacked short flags, `--schema` on builtins (`src/cli.ts:259-384`; `src/cliBuiltinArguments.ts:79-162`; `cliOptionAliases.test.ts`).

---

## Open questions

- How does Incur's `context.agent` decide agent mode (TTY, env var, explicit flag), and which REA commands use the `agent-only` render policy (`src/cli/cliResultOutput.ts:127`)? I did not read Incur's code.
- What do `rea --llms` and `rea <cmd> --schema` actually emit for REA's 185 commands, and how large is that manifest in tokens compared with the MCP catalog? I did not run them. This is the key measurement for a CLI-first token-cost claim.
- Is there any test asserting *field*-level parity between a CLI command's options and its contract's input (beyond `--provider` and the `trace` no-`--limit` checks)? None found. UNKNOWN whether drift exists beyond the naming inconsistencies in §2.4.
- How do client hosts apply approval to CLI invocations compared with MCP annotations in practice (Claude Code Bash allowlist patterns vs per-tool `destructiveHint`)? This affects whether `authority: mutate` capabilities should be CLI-projected at all, or only behind a confirmation flag.
- For the daemon bridge (§10.4): does REA's single-client HTTP takeover model ("a client that connects while another session is open takes it over", `docs/cli.md:1009-1012`) suit an agent plus a human CLI sharing one session? Agentic-anything probably needs multi-client sessions with an owner token.
- Should generated skills include CLI or MCP examples per capability, or only a route table? REA keeps the skill largely hand-written (539 lines, version 60), and the agent-eval harness (salvaged tool-design report) is what validates routing. Generated content needs the same eval.
- The Codex plugin `interface.capabilities: ["Interactive","Read","Write"]` and marketplace `policy.authentication: "ON_INSTALL"` (`.codex-plugin/plugin.json:26-30`; `.agents/plugins/marketplace.json:14-17`). Their semantics on the Codex side were not verified, and they may be a place to declare an authority level.
- Release Please rewrites version pins in manifests (setup report). Would a generator from `product.ts` conflict with that, or replace it?

---

## Files read

`src/cli.ts`, `src/cliCommandNames.ts`, `src/cliOutput.ts`, `src/cliJsonInput.ts`, `src/cliConfiguration.ts`, `src/cliEvidenceCommands.ts`, `src/cliBuiltinArguments.ts`, `src/cliLogging.ts`, `src/cli/commandCancellation.ts`, `src/cli/sqliteCommands.ts`, `src/cli/wasmCommands.ts` (part), `src/cli/applicationCommands.ts`, `src/cli/analysisEvidenceDelivery.ts` (part), `src/cli/cliResultOutput.ts` (part), `src/cli/projectCliAnalysisResult.ts`, `src/cli/tokenWindow.ts` (part), `src/cli/explicitSensitiveArguments.ts` (part), `src/cli/argumentErrors.ts` (part), `src/cli/setupCommands.ts` (doctor), `src/composition/wasm.ts`, `src/contracts/toolContractTypes.ts`, `src/mcpDoctor.ts`, `src/doctorRuntime.ts` (part), `src/mcpStartupPolicy.ts`, `src/domain/skillManifest.ts` (part), `scripts/rea.mjs`, `scripts/plugin-mcp-launcher.mjs`, `scripts/generate-skill-metadata.mjs`, `scripts/lib/product-catalog.mjs`, `scripts/lib/catalog-cli.mjs`, `tests/boundary/cli/cliToolCoverage.test.ts`, `tests/boundary/cli/cliOptionAliases.test.ts`, `tests/boundary/filesystem/productCatalog.test.ts` (part), `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `.codex-plugin/plugin.json`, `.codex-plugin/mcp.json`, `.agents/plugins/marketplace.json`, `server.json`, `install.sh` (part), `.agents/skills/reverse-engineer-anything/SKILL.md` (part), `.agents/skills/reverse-engineer-anything/references/connection-and-recovery.md` (part), `.agents/skills/rea-tool-design/SKILL.md`, `docs/cli.md`, `docs/client-configuration.md` (parts), `docs/mcp-prompts.md`, `docs/reverse-engineering-tools.md` (part), `docs/installation.md` (parts).
External: https://github.com/wevm/incur (README, fetched 2026-11-05).
