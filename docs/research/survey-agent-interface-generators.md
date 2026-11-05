# Survey: Frameworks that GENERATE agent interfaces for existing software

Research date: 2026-11-05

Technique family: `agent-interface-generators`: tools, methods and research that take existing software (source code, an API spec, a schema endpoint, a GUI, docs) and emit an agent-facing interface (CLI, MCP server, skill bundle, synthesized API).

Evidence legend: **OBSERVED** = read in a primary source (repo file with line numbers, or official docs/paper page, URL given). **SECONDARY** = search-engine summaries, third-party write-ups, aggregator pages. **INFERRED** = my interpretation. **UNKNOWN** = not verified this session.

Sources read locally: HKUDS/CLI-Anything, shallow clone at commit `34f5195` (2026-09-22), in the session scratchpad. Paths below marked `CLI-Anything:` are relative to that repo root. REA paths (`src/...`) are relative to `/Users/nhanclie/rea`, which I read and did not modify.

Related reports, referenced here and not repeated:
- `survey-cli-ipc-protocol-wrapping.md`: runtime CLI, PTY, LSP and DAP wrapping, and static surface discovery (sdef, typelib, D-Bus).
- `survey-plugin-extension-points.md`: in-host bridges.
- `survey-safety-rails-and-authorization-models.md`: MCP annotation semantics, and REA's `annotationsFromEffects`.
- `rea-mcp-server-adapter.md`: REA's tool families and compact schemas.
- `rea-setup-distribution-supply-chain-licensing.md`: REA's `server.json` and skill install.
- `salvaged/tool-design-philosophy-and-verification.json`: the REA rule "skills subordinate to contracts".

---

## 1. Summary

- The generators fall into four families. They differ mainly in **where the truth comes from**:
  1. **Spec-driven, deterministic.** OpenAPI to MCP (FastMCP, openapi-mcp-generator, AWS Labs OpenAPI MCP Server, Speakeasy, Stainless, Postman); GraphQL (Apollo MCP Server); gRPC (grpcmcp, protoc-gen-mcp). Input is a machine-readable contract. Output is mechanical, so verification is mostly schema validation.
  2. **LLM-driven, from source code** (HKUDS CLI-Anything). A coding agent follows a long SOP (`HARNESS.md`) to read a codebase, design a CLI, write the CLI and its tests, and emit a SKILL.md. Truth comes from the source code plus real-backend end-to-end tests.
  3. **LLM-driven, from exploration or demonstration** (AppAgent, AutoDroid, AXIS, SkillWeaver, ToolMaker). The agent explores a GUI, website or repo and synthesizes docs, APIs or "skills", then hones them with feedback or tests.
  4. **Packaging and exposure formats** (Agent Skills SKILL.md, MCP Registry `server.json`, MCPB). Code-execution exposure modes also belong here: Anthropic "code execution with MCP" and Tool Search, Cloudflare Code Mode, Stainless Code Mode. These do not generate capability. They decide how much of it an agent sees, and when.
- Spec-driven vendors are converging on the view that **a 1:1 mirror of endpoints is a bad agent interface**:
  - FastMCP docs say curated servers beat auto-converted ones.
  - Stainless removed every tool scheme except a two-tool "Code Mode" on 2026-01-13.
  - Cloudflare and Anthropic push search plus execute, and deferred tool loading.
- The safety vocabulary is converging too:
  - Speakeasy `x-speakeasy-mcp` carries `readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint` and `scopes`, plus a server-start `--scope read` flag.
  - FastMCP has `RouteMap(... EXCLUDE)`.
  - Apollo has `mutation_mode`.
  - openapi-mcp-generator fails closed when credentials are missing.
  - Cloudflare keeps credentials on the host side of the sandbox.
- **CLI-Anything** (Apache-2.0) is the closest prior art to agentic-anything's "point at software, get an agent interface". Its strongest reusable ideas:
  - Never trust exit 0. Verify the real backend output with magic bytes, structure checks and pixel or RMS analysis.
  - Test the *installed* entry point through a subprocess.
  - Every command has `--json`, `--dry-run`, and introspection commands.
  - One canonical SKILL.md per harness, plus registries.

  Its weaknesses:
  - Validation is an LLM prompt, not executable code.
  - CI does not run the harness test suites.
  - The doctrine has drifted from practice (the GIMP harness falls back to Pillow despite "rule #1").
  - Safety is per-harness and ad hoc.
  - It has no effect or authority model.
  - The hub installer runs registry-supplied shell strings.
- Research systems (SkillWeaver, AXIS, ToolMaker) show that **synthesize, then hone against tests or a reward signal** produces transferable APIs. SkillWeaver reports up to +54.3% for weaker agents on WebArena (SECONDARY). None of them has an authority model. agentic-anything should treat any synthesized tool as `provenance: synthesized` and keep it behind a verification gate.

## 2. HKUDS CLI-Anything (deep dive)

### 2.1 Identity, licence, maturity
- Self-description (OBSERVED): "CLI-Anything: Making ALL Software Agent-Native" (`CLI-Anything:README.md:1`).
- Licence: Apache-2.0 (OBSERVED `CLI-Anything:LICENSE:1-3`, `README.md:1797`).
- Tech report (OBSERVED, https://arxiv.org/abs/2606.03854, v1 2026-06-02): "CLI-Anything: Towards Agent-Native Computer Use", Yang, Fan, Huang (HKU Data Intelligence Lab). The abstract argues conceptually against GUI agents ("agent-native" means structured commands, explicit state, deterministic feedback). The abstract reports **no** quantitative benchmark, and the README roadmap still lists "Benchmark suite for agent task completion rates" as unchecked (`README.md:1740`).
- Scale (OBSERVED):
  - 69 `*/agent-harness` directories in the repo.
  - `registry.json` has about 160 `"name"` keys, including nested contributor names; updated 2026-06-19.
  - `public_registry.json` lists third-party CLIs installed via npm, brew or pip (updated 2026-08-20).
  - `matrix_registry.json` defines capability-based "CLI matrices" (video-creation, 3d-cad, and so on) that compose several CLIs, schema v2.
- Test-count claims are inconsistent: "2,280+" (`README.md:853`, `:1807`), "2,461" (`:1389`, `:1393`), "TOTAL 2,464" (`:1441`). They are self-reported, and §2.5 shows they are not CI-verified.
- Maturity (INFERRED): a very active community monorepo with a large catalogue and research-lab branding. Production-grade on some harnesses (Calibre, LibreOffice); thin on others (Zoom 22 unit tests, 0 E2E, `README.md:1421`).

### 2.2 Input
- `/cli-anything <software-path-or-repo>` takes a local source tree or a GitHub URL. "Software names alone (e.g., "gimp") are NOT accepted" (OBSERVED `cli-anything-plugin/commands/cli-anything.md:17-23`).
- The README also says to feed "docs or SDK manuscripts" for web services (`README.md:870-873`). The roadmap item "packaging APIs for closed-source software" is open (`:1743`).
- Stated limitations (OBSERVED `README.md:1731-1735`):
  - It needs frontier models ("Claude Opus 4.6, Claude Sonnet 4.6, GPT-5.4").
  - It "Relies on available source code". With binaries only, "harness quality and coverage will degrade substantially".
  - It "May require iterative refinement".

### 2.3 How it analyzes a codebase (methodology)
`cli-anything-plugin/HARNESS.md` is a 747-line SOP that the coding agent must read first: "Do not improvise" (`commands/cli-anything.md:5-7`). It has seven phases plus 6.5. All of this is OBSERVED in `HARNESS.md`:
1. **Codebase analysis** (`:12-23`):
   - Identify the backend engine (MLT for Shotcut, and so on).
   - "Map GUI actions to API calls … Catalog these mappings."
   - Identify the data model and file formats.
   - Find existing CLIs (`melt`, `ffmpeg`).
   - "Catalog the command/undo system … These commands are your CLI operations." (INFERRED: this is the most transferable discovery heuristic. An app's undo-command classes are a ready-made inventory of mutations.)
2. **Architecture** (`:25-47`):
   - Stateful REPL plus subcommands, both recommended.
   - Command groups by domain: project, core ops, import/export, config, session.
   - A JSON session state model.
   - A `--json` output flag.
3. **Implementation** (`:49-109`):
   - Data layer first. Then "probe/info commands — Let agents inspect before they modify". Then mutations, "One command per logical operation".
   - A `utils/<software>_backend.py` that finds the binary with `shutil.which` and calls it through `subprocess.run`.
   - Session file locking (`guides/session-locking.md`).
   - REPL as the default (`invoke_without_command=True`), with a shared `repl_skin.py`.
4. **Test plan first**: `TEST.md` Part 1 is written before the tests (`:111-140`).
5. **Tests** (`:142-217`): unit tests on synthetic data; E2E on intermediate files; E2E on the true backend; subprocess tests against the installed command; round-trip in the GUI; and "Agent test — Have an AI agent complete a real task using only the CLI".
6. **Test results**: the pytest output is appended to TEST.md (`:219-230`).
- 6.5. **SKILL.md generation** (`:232-292`).
7. **Packaging**: PEP 420 namespace `cli_anything.<software>`. `cli_anything/` has no `__init__.py` (`:294-303`, `:702-707`).

Architecture rules (OBSERVED `HARNESS.md:305-551`):
- "Use the Real Software — Don't Reimplement It … This is the #1 rule" (`:307-331`).
- "The Rendering Gap": project-file edits get silently dropped by naive renderers. Render priority is "native engine → translated filtergraph → script" (`:348-366`).
- "Every filter/effect in the registry MUST have a corresponding render mapping or be explicitly documented as 'project-only (not rendered)'" (`:495-496`).
- CLI design rules: "Fail loudly", "Be idempotent where possible", "Provide introspection — `info`, `list`, `status`", "Every command MUST support `--json`" (`:498-529`).
- An **MCP backend pattern**: when software only has an MCP server, the generated CLI is an MCP *client* that spawns the server per command, with an optional daemon mode (`guides/mcp-backend.md:1-64`). CLI-Anything therefore treats CLI as the agent-facing surface and MCP as a backend.

`/cli-anything:refine` (OBSERVED `commands/refine.md:39-60`) does iterative gap analysis:
- Build a coverage map `{function_name: covered | not_covered}`.
- Re-scan the source for public APIs and scripting interfaces.
- Prioritize gaps by impact, easy wins and composability.
- "Present the gap report to the user and confirm which gaps to address". This is a human gate.

### 2.4 What it emits
Per harness (OBSERVED `commands/cli-anything.md:87-116`, `HARNESS.md:672-700`):
- A Click CLI `cli-anything-<sw>`, which is also a REPL.
- `core/{project,session,export,...}.py` and `utils/<sw>_backend.py`.
- `<SOFTWARE>.md`, the app-specific SOP.
- `README.md`, `tests/TEST.md`, `test_core.py`, `test_full_e2e.py`, `setup.py`.
- A canonical `skills/cli-anything-<sw>/SKILL.md` plus a packaged copy.

Cross-cutting conventions:
- `--json` on every command.
- `--project <file>` session state.
- `--dry-run` with auto-save on exit through Click `result_callback`, skipped when there are no changes, in the REPL, on error, or with `--dry-run` (OBSERVED `guides/auto-save-dry-run.md:13-49`).
- An optional **preview protocol** `preview-bundle/v1` (`docs/PREVIEW_PROTOCOL.md`; `HARNESS.md:553-670`):
  - Persistence model: immutable bundle directories, a mutable `session.json` head, and an append-only `trajectory.json` (command-to-preview history).
  - `preview live status --json` is the "agent-cheap" introspection call.
  - "Truthfulness principle": no faked renders and no GUI screen-scraping (`:658-670`).

  (INFERRED: this is effectively an evidence model for visual state. It maps closely onto REA's evidence records and agentic-anything's before/after action records.)

### 2.5 How it verifies
- **Doctrine** (OBSERVED `HARNESS.md:150-163`, `:388-428`, `:534-545`):
  - E2E "MUST invoke the real software".
  - "No graceful degradation — if the software isn't installed, tests fail, not skip".
  - "Don't trust that export works just because it exits successfully". Check magic bytes (`%PDF-`), OOXML ZIP structure, pixel probes at specific frames (fade-in near-black), audio RMS, and duration.
  - Subprocess tests use `_resolve_cli()`. `CLI_ANYTHING_FORCE_INSTALLED=1` forces the installed binary over the `python -m` fallback, and tests must not set `cwd` (`:166-214`).
- **`/cli-anything:validate` is an LLM prompt, not a program.** It lists 52 checks (directory, files, Click usage, `--json`, `handle_error`, type hints, TEST.md parts, 100% pass rate…) and a sample "Overall: PASS (52/52 checks)" report (OBSERVED `commands/validate.md:25-113`). It also hardcodes `/root/cli-anything/<software-name>/agent-harness/` (`:23`).
- **What CI actually checks** (OBSERVED `.github/workflows/*.yml`):
  - The root-skill mirror (`check-root-skills.yml:35`, `validate_root_skills.py`).
  - Codex and Cursor plugin install scripts.
  - The PR labeler.
  - Pages deploy (registry dates, meta-skill generation).
  - Hub publish.
  - **No workflow runs the harness pytest suites.** `verify-plugin.sh` only checks that plugin files exist and that `plugin.json` parses (`cli-anything-plugin/verify-plugin.sh:18-38`).
  - INFERRED: the "100% pass rate" tables are agent- or contributor-reported TEST.md content, not CI evidence.
- **Doctrine–practice drift** (OBSERVED):
  - The GIMP harness renders through "Pillow (PIL) – pure-Python fallback when GIMP is absent" (`gimp/agent-harness/cli_anything/gimp/core/export.py:8`, `:65-95`).
  - Its SKILL.md says it is "built on Pillow" and that GIMP is only "recommended" (`skills/cli-anything-gimp/SKILL.md:3-4`, `:20-22`).
  - Both contradict HARNESS "#1 rule … No Pillow replacements for GIMP" (`README.md:1652`; `HARNESS.md:307-314`).
  - Lesson (INFERRED): an LLM-followed SOP without executable conformance checks drifts.

### 2.6 Plugin/skill packaging and distribution
Agent front doors (OBSERVED `README.md:260-760`):
- A Claude Code plugin marketplace (`/plugin marketplace add HKUDS/CLI-Anything`, `/plugin install cli-anything`); `cli-anything-plugin/.claude-plugin/plugin.json` contains only name, description and author.
- A Pi extension, Codex skill, Cursor plugin, OpenCode commands, Qoder plugin, Hermes skill, Reasonix skill and GitHub Copilot CLI support.
- Each platform gets a copy of HARNESS.md, commands and guides.

SKILL.md generation:
- `cli-anything-plugin/skill_generator.py` (603 lines) **parses the Click CLI with Python `ast`**: `_click_decorator_info` reads `@x.command("name")` and `@x.group` decorators and docstrings (`:34-78`), `extract_commands_from_cli` (`:248`). It also reads the README intro and the `setup.py` version.
- It renders through a Jinja2 template (`templates/SKILL.md.template`).
- Frontmatter is only `name` and `description` (`HARNESS.md:244-252`).
- All OBSERVED. INFERRED: it extracts deterministically from the *generated* code. This is the right direction (doc generated from the contract), but it captures no parameter types or effects.
- Skills install via `npx skills add HKUDS/CLI-Anything --skill <name> -g -y` (`README.md:76`, `:236`).

CLI-Hub (`pip install cli-anything-hub`, setup version 0.4.1, `cli-hub/setup.py:53`):
- `cli-hub list/search/info/install/update/uninstall/launch` (`README.md:201-209`).
- A meta-skill (`cli-hub-meta-skill/SKILL.md`) tells agents to browse the live catalogue, install a CLI, and then read that CLI's own SKILL.md. This is two-level progressive disclosure: a catalogue skill, then a per-tool skill.

### 2.7 Auth and safety
- `SECURITY.md` (OBSERVED, `:7-24`) models prompt-injected agents passing crafted arguments to backends. Its rules (`:60-68`):
  - No `shell=True`.
  - Allowlist subprocess arguments (for example kdenlive/shotcut `ALLOWED_VCODECS` frozensets).
  - Escape Script-Fu and XML.
  - `abspath`/`realpath` on paths.
  - Credential files at `0o600`.
  - Never echo keys.
- News entries show reactive hardening: GIMP Script-Fu injection fix (2026-03-14), `defusedxml` for untrusted XML (2026-05-19), Sketch token-file path traversal and symlink escape (2026-05-21) (`README.md:59-65`, `:153`).
- Good local pattern (OBSERVED `rekordbox/agent-harness/cli_anything/rekordbox/rekordbox_cli.py:96-131`):
  - `_open_db_for_write(force, backup, operation)` refuses writes while Rekordbox is running unless `--force` is given.
  - It refuses a forced write without a backup.
  - It returns `{"backup": [...], "rekordbox_running": ..., "forced": ...}` in the result.
- No global effect, authority or confirmation model exists. Safety lives in each harness's code and README. Generated SKILL.md frontmatter has no `allowed-tools` and no risk metadata (OBSERVED template and frontmatter; INFERRED gap).
- **Supply chain** (OBSERVED):
  - `cli-hub` fetches `https://hkuds.github.io/CLI-Anything/registry.json` (`cli-hub/cli_hub/registry.py:9-10`).
  - It runs each entry's `install_cmd` string with `shell=True` whenever the string contains `|`, `&&`, `;`, `$(` or a backtick, "so that script-type installs like `curl … | bash` work" (`cli-hub/cli_hub/installer.py:67-93`).
  - In-repo entries install `pip install git+https://github.com/HKUDS/CLI-Anything.git#subdirectory=…` with no ref or hash pin (`registry.json`, jumpserver entry).
  - The repo's own SECURITY rule 1 is "Never use `shell=True`".

## 3. OpenAPI to MCP and API-to-tool generators

| Tool | Input | Emits | Mapping / filtering | Auth | Verification | Licence / maturity |
|---|---|---|---|---|---|---|
| **FastMCP `from_openapi` / `OpenAPIProvider`** (OBSERVED https://gofastmcp.com/integrations/openapi.md) | OpenAPI dict + an `httpx` AsyncClient | An in-process MCP server; runtime proxy, no codegen | Every route becomes a Tool by default; since 2.8.0 GETs are no longer Resources "for client compatibility". `RouteMap(methods, pattern, tags, mcp_type, mcp_tags)` is first-match; `MCPType.EXCLUDE` catch-all; `route_map_fn`, `mcp_component_fn`, `mcp_names`. Names come from `operationId` up to `__`, slugified, truncated to 56 chars | Headers on the supplied client. Over HTTP it forwards some inbound headers but **excludes** `Authorization`, `Cookie`, `Host` | Rejects `.`/`..` path segments, including percent-encoded ones; drops undeclared args; default 30 s timeout | SECONDARY Apache-2.0; mature. The docs themselves say curated servers outperform auto-converted ones and recommend conversion for bootstrapping, not mirroring |
| **openapi-mcp-generator** (harsha-iiiv) (OBSERVED https://github.com/harsha-iiiv/openapi-mcp-generator) | OpenAPI 3.x file or URL | A TypeScript project (`package.json`, `src/index.ts`, `.env.example`, a web playground); transports stdio, web/SSE, streamable-http, cloudflare-worker | `x-mcp: true/false` at root, path or operation (operation > path > root); `--default-include false`; programmatic `getToolsFromOpenApi` with include/exclude tags and operations, `filterFn`, `allowExternalRefs`, `maxToolNameLength` | Env vars per security scheme (`API_KEY_<SCHEME>`, `BEARER_TOKEN_<SCHEME>`, `OAUTH_CLIENT_ID_<SCHEME>`…). **Fails closed when credentials are missing** unless `MCP_ALLOW_MISSING_CREDENTIALS=true` | Args validated against JSON Schema 2020-12 (`@cfworker/json-schema` in v5, replacing zod) | MIT, ~644 stars, v5.x |
| **AWS Labs OpenAPI MCP Server** (OBSERVED https://github.com/awslabs/mcp/tree/main/src/openapi-mcp-server) | `API_SPEC_URL`/`API_SPEC_PATH`, `API_BASE_URL` | A runtime server: tools plus generated "operation prompts" (claims 70–75% prompt-token reduction) | GETs with query params become tools; `INCLUDE_TAGS`/`EXCLUDE_TAGS` | `AUTH_TYPE` none/basic/bearer/api_key (the page also mentions Cognito, inconsistently) | **Spec-fetch hardening**: DNS-resolved IP allowlisting against rebinding, no redirects, 10 MiB cap, private/loopback/plain-HTTP blocked by default (opt-outs `--allow-private-networks`, `--allow-insecure-http`), only internal `$ref`, `--allowed-spec-dirs`; extra specs do not inherit credentials | UNKNOWN on page (awslabs/mcp is Apache-2.0 per SECONDARY) |
| **Speakeasy** (OBSERVED https://www.speakeasy.com/docs/standalone-mcp/customize-tools.md) | OpenAPI + `gen.yaml` (`enableMCPServer: true`, TypeScript SDK) (SECONDARY) | An MCP server as a thin wrapper over the generated TS SDK | `x-speakeasy-mcp: {disabled, name, title, scopes, description, readOnlyHint=false, destructiveHint=true, idempotentHint=false, openWorldHint=true}`; **`--scope read`** mounts only matching tools; `--tool`; OpenAPI **Overlays** add scopes by HTTP method without editing the spec | Env var (`API_TOKEN` example) | Typed SDK underneath | Commercial SaaS; generated code licence UNKNOWN |
| **Stainless** (OBSERVED https://www.stainless.com/docs/mcp; changelog https://www.stainless.com/changelog/mcp-servers-now-support-code-mode-only) | OpenAPI + Stainless TS SDK config `targets.typescript.options.mcp_server` | `packages/mcp-server`, npm `<pkg>-mcp`, an **MCPB** file per release, Docker image, Stainless-hosted / self-hosted / Cloudflare Workers remote | **Two tools only**: docs search plus code execution against the SDK. "We've removed all MCP tool schemes except for 'Code Mode'" (2026-01-13). `--code-execution-mode local` runs a Deno subprocess that can read the install dir, reach the API host and read env, but **cannot write files or spawn processes**; the hosted sandbox is deprecated (410) | Env vars; remote supports header or OAuth | Typed SDK; the SDK typecheck is the verifier (INFERRED) | Commercial; "experimental" |
| **Postman MCP Generator** (OBSERVED https://learning.postman.com/docs/postman-ai/mcp-servers/generate/) | Requests picked from **public** Postman API Network workspaces | A zip download; one tool per request; STDIO plus streamable HTTP | Manual selection | UNKNOWN on page | UNKNOWN | Proprietary platform feature |
| **Kong** | — | SECONDARY: only `@kong/mcp-konnect`, a server for *managing* Konnect. No spec-to-MCP generator found | — | — | — | UNKNOWN |

Cross-cutting observations (INFERRED unless noted):
- **Spec quality is the ceiling.**
  - Missing `operationId`s give bad names.
  - Flattened request bodies (FastMCP merges `allOf`, and adds discriminator subtypes as optional fields) lose constraints.
  - One tool per endpoint floods context.
  - That is why the extensions (`x-mcp`, `x-speakeasy-mcp`, Overlays, `mcp_names`) exist: humans curate a thin layer over the generated mapping.
- **Default exposure policy differs.**
  - FastMCP and openapi-mcp-generator default to *include everything*.
  - Speakeasy defaults `destructiveHint: true` and lets you start read-only with `--scope read`.
  - No one infers effects from the HTTP method automatically. Speakeasy's docs show it done by an Overlay rule (GET/HEAD to `read`, POST/PUT/PATCH/DELETE to `write`) (OBSERVED).
- **Runtime proxy vs codegen.**
  - Runtime proxies (FastMCP, AWS, grpcmcp) follow the spec as it changes but give no reviewable artifact.
  - Codegen (openapi-mcp-generator, Speakeasy, Stainless, protoc-gen-mcp) gives a diffable, pinnable artifact.

## 4. GraphQL and gRPC reflection-based generators

- **Apollo MCP Server** (OBSERVED https://www.apollographql.com/docs/apollo-mcp-server/define-tools; licence UNKNOWN):
  - Tools come from *operations*, not the whole schema. Sources:
    - `operations.source: local` (`.graphql` files, hot reload).
    - `collection` (a GraphOS Operation Collection).
    - `manifest` (a persisted-query manifest).
    - `uplink` (GraphOS-managed persisted queries).
  - Descriptions come from config, from `#` comments above the operation, or are generated from schema field descriptions (SECONDARY).
  - Optional **introspection tools** `introspect` (with `minify`), `search` (with `index_memory_bytes`, `leaf_depth`), `validate` (checks an operation against the schema *without executing it*) and `execute`. This is a schema-browse, validate, execute loop.
  - Mutations are gated by `mutation_mode`; the accepted values were not seen this session.
  - Overrides: `descriptions`, `required_scopes`, `annotations`, `param_headers`, `enable_output_schema`.
  - INFERRED: persisted queries are a strong pattern. The allowlist of operations *is* the agent interface, and a "contract variant" limits what AI can reach (SECONDARY).
- **grpcmcp** (adiom-data) (OBSERVED https://github.com/adiom-data/grpcmcp; Apache-2.0; 29 stars, 7 commits, no releases):
  - Gets definitions via `--reflect` (server reflection) or `--descriptors` (a `buf build -o` / `protoc --descriptor_set_out` file). `--services` allowlist.
  - Exposes **only unary** methods; streaming methods are skipped.
  - `--bearer`/`--bearer-env`, `--header`.
  - `--string64` renders int64 as strings for JS clients.
  - Inbound auth is left to middleware.
- **protoc-gen-mcp** (the-protobuf-project) (SECONDARY): a protoc/buf plugin that generates MCP server code from proto definitions, with MCP annotations published to the Buf Schema Registry. v1.7.1. Licence UNKNOWN.
- INFERRED for agentic-anything:
  - Reflection and introspection endpoints (gRPC reflection, GraphQL introspection, D-Bus `Introspect`, COM type libraries, AppleScript `sdef`, OpenAPI at `/openapi.json`, `--help` trees) are the same idea: **a live, machine-readable surface inventory**.
  - One discovery stage should probe for all of them and normalize into one IR. Streaming and long-running calls need a separate tool shape (start/poll/cancel), as grpcmcp's omission shows.

## 5. Packaging formats

### 5.1 Agent Skills (SKILL.md)
(OBSERVED https://agentskills.io/specification)
- A directory with `SKILL.md`, plus optional `scripts/`, `references/`, `assets/`.
- Frontmatter:
  - `name`: required; 1–64 chars; lowercase a-z, 0-9 and hyphens; no leading, trailing or double hyphen; **"Must match the parent directory name"**.
  - `description`: required; 1–1024 chars; "what the skill does and when to use it".
  - Optional `license`, `compatibility` (≤500 chars; environment requirements), `metadata` (string to string map) and `allowed-tools` ("Experimental", e.g. `Bash(git:*) Bash(jq:*) Read`).
- Progressive disclosure: metadata ~100 tokens at startup for all skills; body "< 5000 tokens recommended", loaded on activation; resources on demand. "Keep your main SKILL.md under 500 lines"; references one level deep.
- Validator: `skills-ref validate ./my-skill`.
- How the systems surveyed use it:
  - CLI-Anything emits only `name` and `description`, and its names are quoted strings like `"cli-anything-gimp"`, which is valid.
  - REA's shipped skill uses `metadata.version: "60"` (OBSERVED `/Users/nhanclie/rea/.agents/skills/reverse-engineer-anything/SKILL.md:1-6`).
  - REA's doctrine: skills route and recover, contracts are authoritative (see salvaged JSON).
- Anthropic's code-execution article positions SKILL.md as the way an agent **persists its own working code** (`./skills/<fn>.ts` plus a SKILL.md) (OBSERVED https://www.anthropic.com/engineering/code-execution-with-mcp).

### 5.2 MCP Registry `server.json`
(OBSERVED https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/generic-server-json.md and `official-registry-requirements.md`)
- Schema `https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json`.
- `name` is reverse-DNS (`io.github.<user>/<server>`); `description`, `version`, `title`, `websiteUrl`.
- `repository{url, source, subfolder, id}`.
- `packages[]`:
  - `registryType` npm, pypi, oci, nuget, cargo or mcpb; `identifier`, `version`, `registryBaseUrl`, and `fileSha256` for mcpb.
  - `transport` (stdio, or streamable-http with a URL template), `runtimeHint` (npx, uvx, dnx).
  - `packageArguments`/`runtimeArguments` (positional or named, `isRequired`, `choices`, `format`).
  - `environmentVariables[]` (`isSecret`, `isRequired`, `default`, `choices`).
- `remotes[]` (streamable-http or sse, URL with `{variables}`, `headers[]` with `isSecret`).
- Official registry rules:
  - Publishers must prove namespace ownership (domain/GitHub).
  - "All packages must include metadata proving the publisher owns them".
  - Only public registries are allowed (npmjs, pypi, nuget, crates, Docker Hub, GHCR, Quay, GAR, ACR, MCR; MCPB from GitHub or GitLab releases only).
  - `_meta` is kept only under `io.modelcontextprotocol.registry/publisher-provided`, ≤4 KB.
- REA's own entry (OBSERVED `/Users/nhanclie/rea/server.json:1-30`): npm `rea-agents` 6.8.0, `runtimeHint: npx`, stdio, positional arg `mcp`.
- INFERRED: `server.json` describes *how to launch*, not *what the tools do or risk*. Effects and authority must live elsewhere: tool annotations at runtime, plus agentic-anything's own manifest.

### 5.3 Other packaging seen
- MCPB bundles (Stainless emits one per release).
- Claude Code plugin marketplaces (CLI-Anything, REA).
- `npx skills add <repo> --skill <name>` (CLI-Anything).
- Postman zip.

## 6. Code execution with MCP, CLI-over-MCP, and progressive disclosure

- **Anthropic, "Code execution with MCP"** (OBSERVED, 2025-11-04, https://www.anthropic.com/engineering/code-execution-with-mcp):
  - Present servers as a file tree of typed functions (`./servers/salesforce/updateRecord.ts`). The agent lists and reads only what it needs, or calls `search_tools` with a detail level (name / +description / full schema).
  - Filter data inside the sandbox, so intermediate results never reach the model.
  - Example: "150,000 tokens to 2,000 tokens … a time and cost saving of 98.7%".
  - PII can be tokenized by the client before the model sees it.
  - Drawback stated: it needs "a secure execution environment … sandboxing, resource limits, and monitoring".
- **Anthropic advanced tool use** (OBSERVED, 2025-11-24, https://www.anthropic.com/engineering/advanced-tool-use):
  - Tool Search Tool with `defer_loading: true`: "~77K tokens" falls to "~8.7K"; Opus 4 accuracy on MCP evaluations "from 49% to 74%", Opus 4.5 "from 79.5% to 88.1%".
  - Programmatic Tool Calling (`allowed_callers: ["code_execution_20250825"]`): "43,588 to 27,297 tokens" (37%).
  - Tool Use Examples (`input_examples`): "72% to 90%" on complex parameters.
- **Cloudflare Code Mode** (OBSERVED https://developers.cloudflare.com/agents/model-context-protocol/codemode/index.md):
  - `codeMcpServer()` wraps an MCP server into one `code` tool, with TS definitions in its description.
  - `openApiMcpServer()` exposes `search` (code reads `codemode.spec()`) and `execute` (`codemode.request()`).
  - Runs in a `DynamicWorkerExecutor` isolate that **blocks outbound fetch/connect by default**.
  - **Credentials stay in the host-side request callback; the generated code never sees the token.**
  - Responses are capped at ~6,000 estimated tokens and marked `--- TRUNCATED ---`.
  - "Experimental".
- **Stainless Code Mode**: see §3. Two tools, a local Deno sandbox with no FS writes and no subprocesses.
- **CLI vs MCP measurements** (SECONDARY, not reproduced):
  - Scalekit (75 runs): CLI "10 to 32× cheaper", 100% vs MCP 72% reliability.
  - Smithery (756 runs, March 2026): the interface matters less than good descriptions and native harness integration; CLI exploration overhead sometimes cost more tokens.
  - A single-author GitHub test: MCP schema 9,542 tokens vs CLI 97 tokens on the first call.
  - Mario Zechner's browser benchmark: reportedly both 100% success, MCP 23% faster (unverified).
- **REA's own answer** (OBSERVED):
  - `REA_MCP_TOOL_FAMILIES` scopes registration: "Agents pay context for every advertised tool schema … out-of-scope tools stay registered but disabled" (`src/config/mcpToolFamilies.ts:11-14`).
  - The input-schema profile defaults to `compact` (`src/config/mcpInputSchemaProfile.ts:13-18`).
  - See `rea-mcp-server-adapter.md` for measured sizes.
- INFERRED synthesis: the debate is mostly about **exposure profile, not protocol**. A generator should emit one capability IR and render it three ways:
  - (a) full typed MCP tools for small surfaces or high-risk operations, where per-call confirmation matters;
  - (b) search plus execute (code mode) for large read-mostly surfaces;
  - (c) a CLI plus SKILL.md for shell-native agents.

  Code mode weakens per-call authorization, because one `execute` can perform many mutations. It is safe only if the sandbox's host callbacks enforce authority per underlying operation (Cloudflare's design point).

## 7. LLM-driven "auto-wrap any app" research

| Work | Input | Emits | Verification | Auth/safety | Licence / status |
|---|---|---|---|---|---|
| **AppAgent** (Tencent, arXiv 2312.13771, Dec 2023) (SECONDARY) | Android app + GPT-4V; autonomous trial-and-error exploration *or* human demonstration | A per-app **reference document** of UI elements and their effects, used in a deployment phase | 50 tasks / 10 apps | None beyond the sandboxed device | Code on GitHub; licence UNKNOWN |
| **AutoDroid** (arXiv 2308.15272) (SECONDARY) | Android apps; random exploration builds a **UI Transition Graph**; an LLM summarizes element functions and makes simulated tasks | App memory injected into prompts | 158-task benchmark: 90.9% action accuracy, 71.3% task success | — | Research; AutoDroid-V2 (2412.18116) moves to code generation over an element transition graph |
| **AXIS** (arXiv 2409.17140, ACL 2025, Microsoft authors) (SECONDARY) | An app's existing APIs + support docs + action trajectories | New **"Skills"**: higher-level APIs composed from existing ones; UI only when no API exists | MS Word: −65–70% completion time, 97–98% accuracy | — | Code "aka.ms/haci_axis"; UNKNOWN |
| **SkillWeaver** (arXiv 2504.07079, OSU et al.) (SECONDARY) | A website; the agent proposes skills, practices them, distills **Python APIs** | A reusable API library per site | "Skill honing": auto-generated test cases + reward model; WebArena +39.8% relative; strong-agent APIs give weaker agents up to +54.3% | — | Research |
| **ToolMaker** ("LLM Agents Making Agent Tools", arXiv 2502.11705; KatherLab/ToolMaker) (SECONDARY) | A GitHub repo URL + task description | A Docker image (env setup) + a Python tool function | Self-correction loop; scored by unit tests; 80% of 15 tasks vs OpenHands 20% (unconfirmed) | Docker isolation | Code public; licence UNKNOWN |
| **ToolLLM / ToolBench** (arXiv 2307.16789) (SECONDARY) | 16,464 RapidAPI REST APIs, 49 categories | Instruction data, DFSDT search, an API retriever, ToolLLaMA | ToolEval pass rate / win rate | None | Research dataset |

INFERRED takeaways:
- (1) **Exploration produces documentation first and APIs second.** AppAgent's doc and AutoDroid's UTG are observation artifacts. SkillWeaver and AXIS promote them into callable APIs only after practice.
- (2) **A test or reward signal is what turns a guess into a tool**: SkillWeaver honing, ToolMaker unit tests, CLI-Anything's real-backend E2E.
- (3) **Transfer works.** APIs synthesized by strong models help weaker ones, which argues for a shared, versioned registry of verified synthesized tools.
- (4) None of these models authority, effects or reversibility. Exploration itself mutates the app (AppAgent taps everything), so exploration must run on disposable or snapshotted state for anything not read-only.

## 8. Comparison table

| System | Discovery input | Emitted interface | Verification | Effects/authority metadata | Auth handling | Exposure profile | Licence | Maturity |
|---|---|---|---|---|---|---|---|---|
| CLI-Anything | Source repo (+docs); LLM reads code | Click CLI + REPL, `--json`, SKILL.md, pip package, registry entry | Real-backend E2E + output oracles + subprocess on the installed binary (doctrine); LLM `validate` prompt; CI runs no harness tests | None global; per-harness guards (rekordbox backup/force) | Per-harness; `0o600` key files | CLI + per-tool skill + hub meta-skill | Apache-2.0 | Large, fast-moving, uneven |
| FastMCP from_openapi | OpenAPI | Runtime MCP server | Path-segment checks, arg filtering | `mcp_tags` only; RouteMap EXCLUDE | httpx client headers; strips inbound creds | 1 tool/route (curatable) | Apache-2.0 (SECONDARY) | Mature |
| openapi-mcp-generator | OpenAPI | TS MCP project (4 transports) | JSON Schema 2020-12 arg validation | `x-mcp` include/exclude | Per-scheme env; fail closed | 1 tool/op | MIT | Mid (~644★) |
| AWS OpenAPI MCP | OpenAPI URL/path | Runtime server + prompts | SSRF/rebinding-hardened spec fetch | Tag filters | basic/bearer/api_key | 1 tool/op + prompts | Apache-2.0 (SECONDARY) | Vendor-labs |
| Speakeasy | OpenAPI + extension/Overlay | MCP over a generated TS SDK | SDK types | **Full MCP hints + scopes; `--scope read`** | Env | Scoped subsets | Commercial | Commercial GA (INFERRED) |
| Stainless | OpenAPI | SDK + MCP (2 tools), MCPB, Docker, remote | SDK typecheck (INFERRED) | None per op (code mode) | Env / OAuth remote | **Code mode only** | Commercial | "Experimental" |
| Postman | Public API Network requests | Zipped MCP server | UNKNOWN | UNKNOWN | UNKNOWN | 1 tool/request | Proprietary | Product feature |
| Apollo MCP | GraphQL schema + operations / persisted queries | MCP tools per operation + introspection tools | `validate` without execute | `mutation_mode`, `required_scopes`, `annotations` overrides | Headers, GraphOS key | Allowlisted ops or browse/validate/execute | UNKNOWN | Vendor; parts experimental |
| grpcmcp | gRPC reflection / descriptors | Runtime MCP proxy (unary only) | Proto schema | `--services` allowlist | Bearer/headers outbound | 1 tool/method | Apache-2.0 | Early |
| Cloudflare Code Mode | MCP server or OpenAPI | `code` or `search` + `execute` | Isolate sandbox; 6k-token cap | Host callback can enforce | **Host-side only** | Code mode | SECONDARY MIT | Experimental |
| Agent Skills | (format) | SKILL.md dir | `skills-ref validate` | `allowed-tools` (experimental) | n/a | 3-level progressive disclosure | Open spec | Widely adopted |
| MCP Registry | (format) | `server.json` | Namespace + package-ownership proof | None (launch metadata) | `isSecret` env/headers | n/a | Open | GA-ish (schema 2025-12-11) |
| SkillWeaver / AXIS / ToolMaker | Live site / app APIs + docs / repo | Synthesized APIs/tools | Tests + reward / benchmark | None | None | Library of APIs | Research | Research |

## 9. Adopt vs avoid

**Adopt**
- CLI-Anything's verification doctrine, made **executable**:
  - Real-backend E2E with format oracles (magic bytes, structure, pixel/RMS/duration).
  - Subprocess tests against the *installed* entry point (`FORCE_INSTALLED`).
  - Fail rather than skip when the backend is missing.
  - "Every effect has a render mapping or is declared project-only."
- CLI-Anything's discovery heuristics: backend engine, native data model, existing CLIs, and **the undo/command system as the mutation inventory**. Also its "probe/info before mutate" ordering.
- `--json` everywhere, `--dry-run`, explicit session state, and an append-only trajectory (`trajectory.json`) tied to immutable preview snapshots, which works as an evidence model for visual state.
- Spec-driven deterministic emitters wherever a contract exists: OpenAPI, GraphQL introspection, gRPC reflection, and the static surfaces in `survey-cli-ipc-protocol-wrapping.md`. Use LLM synthesis only for gaps.
- An overlay or extension layer for curation (`x-speakeasy-mcp`-style): name, description, scopes, the hints, disabled. Keep it *outside* the upstream spec (OpenAPI Overlays).
- Read-only-by-default start (`--scope read`), operation allowlists (Apollo persisted queries, grpcmcp `--services`), FastMCP-style EXCLUDE catch-alls.
- Fail closed on missing credentials (openapi-mcp-generator); host-side credentials invisible to generated code (Cloudflare); SSRF-hardened spec fetching (AWS).
- Multiple exposure profiles from one IR (§6). REA's tool families and compact schema already prove the need.
- Packaging: Agent Skills spec plus `skills-ref validate` in CI; `server.json` with `isSecret`; MCPB/npm/PyPI; one version authority (REA pattern).
- Research loop: propose, synthesize, hone with generated tests, then promote. Tag the tool as synthesized and keep the evidence.

**Avoid**
- LLM-only conformance checks (`validate.md` emits "52/52 PASS" with no program behind it) and self-reported test tables.
- SOP rules with no conformance test: CLI-Anything's GIMP harness falls back to Pillow despite its own rule #1.
- 1:1 endpoint mirroring as the final interface (FastMCP's own warning; Stainless dropping it).
- Mapping by HTTP verb alone. FastMCP reversed GET-to-Resource in 2.8; method is a hint for effects, not proof.
- Code mode without per-operation authority enforcement in the host callbacks.
- Registry installers that execute remote shell strings (`cli-hub` `shell=True` for `curl … | bash`), and unpinned `git+` installs.
- Hardcoded environment paths (`/root/cli-anything/...` in `validate.md:23`), and per-platform copies of the SOP that can drift.
- Exploration-based synthesis on live user state without snapshots or a disposable environment.

## What to distill for agentic-anything

1. **A generator pipeline with five fixed stages and typed handoffs**: `discover → model → emit → verify → package`.
   - Each stage writes a versioned artifact (`surface-inventory.json`, `capabilities.yaml`, emitted code, `verification-report.json`, manifests).
   - Stages are re-runnable. `refine` is a diff on `capabilities.yaml` (CLI-Anything's coverage map, made machine-readable).
2. **Capability IR (`capabilities.yaml`), one entry per operation.** Each entry carries:
   - `id`, `title`, `description`, `input_schema`, `output_schema`.
   - `effects` (REA `ToolEffects` flags), with MCP annotations *derived* from them as in `src/contracts/toolEffects.ts:765-786`.
   - `authority` (observe/launch/attach/mutate/destructive), `binding` (openapi-op | graphql-op | grpc-method | cli-argv | script-bridge | a11y-path), and `idempotent`.
   - `reversible` + `undo_binding` (taken from the app's undo system where one exists).
   - `verification` (oracle refs), and `provenance` (spec | introspection | source-analysis | llm-synthesized | exploration) with evidence ids.
3. **Discovery probes as plugins**, each emitting the same inventory shape:
   - OpenAPI at well-known paths, GraphQL introspection, gRPC reflection.
   - `--help`/argparse/Click AST (CLI-Anything's `skill_generator.py` approach), sdef/typelib/D-Bus.
   - Source-code analysis (LLM), and GUI exploration (AppAgent/AutoDroid-style, read-only or on snapshots).
4. **Default emission policy**:
   - Only `authority: observe` operations are enabled.
   - Mutating operations are emitted but disabled, until an operator overlay enables them by scope (Speakeasy `--scope`).
   - Destructive operations always need confirmation or elicitation.
   - Code mode is allowed only when every host callback re-checks per-operation authority.
5. **Verification lanes, executed by CI, never by an LLM**:
   - (a) Schema lane: IR ↔ emitted tool schemas round-trip; `skills-ref validate`; `server.json` schema validation.
   - (b) Contract lane: spec examples replayed against a mock or recorded fixture.
   - (c) Real-backend lane: CLI-Anything oracles; tests fail when the backend is missing.
   - (d) Installed-artifact lane: subprocess/MCP client against the packaged build.
   - (e) Agent-task lane: a few scripted tasks; the success rate is recorded.
   - The verification report is an evidence record; a tool without a passing report cannot be enabled.
6. **Exposure renderers from the same IR**:
   - `mcp-typed`: per-op tools, families, compact schemas.
   - `mcp-codemode`: search + execute, host-side credentials, network-blocked sandbox, output cap.
   - `cli`: `--json`, `--dry-run`, REPL optional.
   - `skill`: SKILL.md ≤500 lines, references/ per family, `compatibility` field filled from the backend requirements.
7. **Packaging**:
   - Generate `server.json` (with `isSecret` env), SKILL.md, plugin manifests and MCPB from one version source. Reuse REA's release-checkpoint validator idea.
   - Never emit installers that run remote shell strings; pin by version plus hash.
8. **Write-safety template for any mutating binding**, generalizing Rekordbox `_open_db_for_write`:
   - Detect whether the target app is running, then refuse or require `--force`.
   - Back up automatically before the write.
   - Return `{backup, forced, target_running}` in the action record.
9. **Synthesized-tool lifecycle**, from SkillWeaver/AXIS/ToolMaker:
   - `proposed → synthesized → honed (tests pass N runs) → promoted`.
   - Promotion needs a human approval plus a passing verification report.
   - Synthesized tools carry `provenance: llm-synthesized`, and the evidence trail of the exploration that justified them.
10. **Generator inputs are untrusted.** Treat specs, READMEs and source as untrusted for the generating agent (prompt injection). AWS's spec-fetch hardening applies; CLI-Anything's SECURITY threat model already lists this.

## Open questions

- Should agentic-anything *wrap* CLI-Anything harnesses as a binding type (`cli-argv` with `--json`), since 69 exist under Apache-2.0, or only borrow its methodology? Their missing effects metadata would have to come from an overlay.
- How should a code-mode `execute` call be represented in the action log and authority model? One record per inner operation (via host callbacks), or one per script?
- Can effects be inferred reliably enough to *propose* defaults (HTTP method, GraphQL mutation vs query, Click option names like `--delete`, undo-command class names), with a human confirming? Or should defaults always be maximally restrictive?
- What is the minimum real-backend oracle set per domain (documents, media, CAD, databases, network services)? Is a shared oracle library worthwhile (CLI-Anything implements these per harness)?
- Exploration safety: what snapshot or disposable-environment mechanism per platform (VM/container snapshot, app profile copy, DB backup) is a prerequisite for AppAgent-style discovery on owned software?
- Do the Smithery and Scalekit CLI-vs-MCP results reproduce with current clients that have tool search or deferred loading? This should be an M4 measurement before choosing default exposure profiles.
- Licences still UNKNOWN: Apollo MCP Server, protoc-gen-mcp, AppAgent, ToolMaker, Postman-generated code, Speakeasy/Stainless generated-code terms. Confirm these before reusing any of their code.
- The CLI-Anything tech report (arXiv 2606.03854) full text was not read. Does it contain quantitative agent-task results that contradict or support the README claims?
