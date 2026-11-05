# Survey: Capability discovery and distillation — how to automatically find and characterize what a piece of software can do

Research date: 2026-11-05

Evidence legend:
- **OBSERVED**: read in a primary source (spec, official docs, repo file) or reproduced locally on this machine (macOS 27.0.1, gh 2.102.0, docker 29.6.1, kubectl 1.36.1). Cited by file path with line numbers or URL.
- **SECONDARY**: search summaries, third-party directory listings, paper abstracts read through aggregators.
- **INFERRED**: my interpretation or design proposal.
- **UNKNOWN**: not established.

REA code is cited as `src/path.ts:line` against the local checkout `/Users/nhanclie/rea` (HEAD `3484432fb`).

**Scope.** This survey covers how to *enumerate and characterize* what an app can do, and how to turn that into typed, safe, verified agent tools. It does not repeat these existing reports; it cites them instead:
- `survey-cli-ipc-protocol-wrapping.md`: how to *drive* CLIs/PTYs/LSP/DAP/CDP/D-Bus/COM once found, plus a three-step static → dynamic → negotiate discovery sketch (its "Discovery pipeline" section).
- `survey-os-automation-apis.md`: Apple Events, COM, App Intents, D-Bus as control APIs.
- `survey-accessibility-trees.md`: the UIA/AX/AT-SPI data model.
- `survey-plugin-extension-points.md`: per-app plugin surfaces.
- `survey-computer-use-vision-agents.md` and `survey-prior-art-universal-agent-control-planes.md`: OpenAdapt, UFO2, vendor substrates.
- `survey-safety-rails-and-authorization-models.md`: MCP annotations, confirmation, isolation.
- `rea-desktop-applications-and-agent-environments.md`: REA's HAR → MCP environment synthesis and verifier gates. Its gap G11, "Discovery is classification, not capability enumeration", is the gap this survey addresses.
- `salvaged/tool-design-philosophy-and-verification.json` and `salvaged/capability-registry-and-composition.json`: REA's ToolContract, ToolEffects and catalog pipeline.

---

## 1. Summary

1. **Software has a description layer, and most of it is machine-readable and already there.**
   - OBSERVED on this machine, without installing anything:
     - Cobra CLIs answer `__complete` with subcommands, flags and descriptions (gh, docker).
     - Swift ArgumentParser CLIs dump a full JSON command tree with `--experimental-dump-help` (`shortcuts`).
     - 26 app bundles ship App Intents metadata (`Metadata.appintents/extract.actionsdata`) with typed parameters and semantic `systemProtocols` such as `DeleteEntity`.
     - Scriptable Mac apps ship `.sdef` dictionaries: Word 423 commands, Excel 370, Music 31.
     - The CDP protocol JSON lists 677 commands across 59 domains.
   - The first pipeline step should be a cheap **deterministic harvest of the self-descriptions an app already ships**. LLM exploration comes later.
2. **Self-descriptions carry names and shapes, but they rarely carry effect, and the effect signals that do exist are inconsistent.**
   - Explicit effect or danger fields exist in only a few formats:
     - usage spec `effect` (`read|write|destructive`), where absence means "ask";
     - Fig `isDangerous`;
     - PowerShell `ConfirmImpact` + `SupportsShouldProcess`;
     - App Intents `DeleteEntity`/`CreateEntity` protocols;
     - sdef `requires-access`;
     - COM `INVOKEKIND` (get vs put);
     - D-Bus property `access`;
     - GraphQL query vs mutation;
     - HTTP method semantics.
   - CDP's 677 commands, Windows App Actions and D-Bus methods carry **no** effect field.
   - So effect classification has to be a **separate, conservative, evidence-backed step**: take the maximum over the signals, treat absence as unknown, and confirm in a sandbox.
3. **Discovery probes have side effects.** OBSERVED: `kubectl __complete get ""` tried to reach an API server at `localhost:8080`. Click's and Cobra's completion paths run the program's own code. UI exploration clicks real buttons. Discovery therefore needs an authority level of its own: `static < introspect < probe < explore`. Anything above `introspect` belongs in a sandbox or on a disposable profile.
4. **Prior systems split along one axis: who writes the tool code.**
   - **Deterministic projectors** map a spec to tools with no LLM: FastMCP `from_openapi`, `usage mcp`, REA `environment-manifest`.
   - **LLM synthesizers** write wrapper code:
     - CLI-Anything: source → Click CLI + tests + SKILL.md;
     - WALT: website → tools;
     - SkillWeaver: practice → Playwright APIs;
     - ToolMaker: repo → tool with unit-test loop;
     - AppAgent and AutoDroid: exploration → UI docs/memory.
   - The projectors are safe but shallow. FastMCP itself warns that curated servers beat auto-converted ones.
   - The synthesizers have range, but none of them classifies effects. CLI-Anything's 747-line HARNESS.md never mentions destructive or confirmation semantics.
5. **REA already contains the three distillation disciplines this project needs.**
   - **Mandatory effect audit**: a closed vocabulary of ten booleans, MCP hints derived mechanically, and a build failure if any tool lacks an audit (`src/contracts/toolEffects.ts:1-30,765-787,819-831`).
   - **Confirmed-write admission** for synthesized tools: serve a write only if a later read confirmed it; otherwise withhold it with one of ten reasons (`docs/agent-environments.md:175-186`).
   - **Fidelity comparison** against live probes, with categorized diffs (`docs/agent-environments.md:1067-1184`).
   - REA applies them only to recorded HTTP traffic. agentic-anything should generalize them to every discovery source.
6. **Proposed pipeline**: discover → characterize → synthesize → verify → package.
   - Discovery harvest, schema projection, effect *floors*, admission gates, catalog hashing and packaging are deterministic.
   - LLMs are confined to proposals: grouping and naming, descriptions, wrapper code for untyped surfaces, exploration policy, and test-scenario drafting.
   - Every LLM output compiles to a declarative manifest that deterministic validators and real-app tests must admit.

---

## 2. What "a capability" is (the unit to discover)

INFERRED. A discovered capability is useful to an agent only when all of these are known or explicitly marked unknown:

| Field | Meaning | Example source |
|---|---|---|
| `surface` | Transport/adapter: `cli`, `http`, `graphql`, `grpc`, `dbus`, `com`, `apple-event`, `app-intent`, `app-action`, `app-function`, `cdp`, `lsp`, `dap`, `a11y`, `plugin-bridge` | discovery source |
| `address` | How to invoke: argv template, method + path, D-Bus dest/path/iface/member, sdef command code (`hookPlay`), CLSID + DISPID, CDP `Domain.method` | spec |
| `input_schema` | JSON Schema, closed (`additionalProperties: false`) | spec, inferred |
| `output_schema` | JSON Schema or `unknown` | spec, `--json` field list, sampled outputs |
| `effect` | `read`, `ui`, `write`, `destructive`, `external`, `exec`, `unknown`, with per-signal provenance | §4.3 |
| `preconditions` | Running instance, open document, auth, feature flag, platform version | spec availability fields |
| `identity` | Which binary/bundle/version this was discovered on (hash) | install inspection |
| `oracle` | How to verify the effect happened: read-back capability, state view, a11y predicate | §6 |
| `provenance` | Every source that asserted each field, with confidence `declared`, `observed`, `inferred` | pipeline |

REA's Evidence discipline maps onto this record: "Missing coverage is unknown, not absence" (`/Users/nhanclie/rea/docs/tool-design.md:159`). So does REA's `next_probes` shape: discovery emits *suggested follow-up probes* with `rationale` and `confidence: "inferred"` (`src/domain/desktopApplication.ts:166-179,207`; `src/domain/artifactInspection.ts:114,326-374`).

---

## 3. Discovery sources

### 3.1 Overview table

| Source family | Concrete probe | Yields | Effect signal | Probe cost / side effects | Typical confidence |
|---|---|---|---|---|---|
| Framework JSON dump | `X --experimental-dump-help` (Swift ArgumentParser); `usage` KDL; Click `Context.to_info_dict()` (SECONDARY) | Full command tree, arg kinds, optionality, repetition | usage `effect` only | Executes binary; normally pure | High |
| Runtime completion protocol | Cobra `X __complete <words> ""`; Click `_X_COMPLETE=bash_complete`; clap `COMPLETE=<shell>` (SECONDARY); argcomplete `_ARGCOMPLETE=1` (SECONDARY) | Subcommands, flags with descriptions, dynamic enum values | none | **May run app code and network** (kubectl) | High for names, low for types |
| Curated completion specs | withfig/autocomplete (715 TS specs), carapace | Tree, arg generators, `isDangerous` | `isDangerous` | None (static files) | Medium (third-party, may lag) |
| Help/man parsing | `--help`, `man`, zsh `_gnu_generic`, fish `create_manpage_completions.py` | Flags, prose | keywords only | Executes binary (`--help`) | Low to medium |
| Web/RPC self-description | OpenAPI doc, GraphQL `__schema`, gRPC `ServerReflection`, Arazzo workflows (SECONDARY) | Typed ops + schemas | HTTP method, GraphQL root type | Network call to a service you are authorized for | High |
| OS IPC self-description | D-Bus `Introspect`, `org.gtk.Actions.DescribeAll`, COM `ITypeInfo`, `.sdef` | Typed members, properties | property access, `INVOKEKIND`, `requires-access` | D-Bus/COM may activate the app | High |
| OS agent-action registries | App Intents `extract.actionsdata`, Windows App Actions JSON, Android AppFunctions XML | Typed, user-facing actions | `systemProtocols`; `displaysUI` | Static files; invocation is gated | High (opt-in surface) |
| Dev protocols | LSP/DAP `initialize`; CDP `/json/protocol`; MCP `tools/list` | Capability flags, method catalogs | MCP annotations only | Spawns/attaches | High |
| API docs / type stubs | `.d.ts`, `.pyi`, Sphinx/Doxygen, bpy API reference | Signatures, prose | rarely | None | Medium |
| Config schemas | JSON Schema (`$schema`), VS Code `contributes.configuration`, GSettings schemas, plist/registry | Settable state with types | writes by definition | None | Medium |
| UI exploration | a11y crawl, menu-bar enumeration, DroidBot/AppAgent/AutoDroid/WALT | Reachable states, actions, labels | inferred from labels | **Clicks real UI**: sandbox required | Low to medium |
| Demonstration / traffic | OpenAdapt, Playwright codegen, HAR (REA) | Concrete action sequences, API calls | HTTP method; observed diffs | Human-driven | Medium (observed ≠ general) |
| Source analysis | CLI-Anything, REA route inventory, AppFunctions skill | Internal APIs, command/undo systems, routes | inferred from code | Read-only | Medium (needs runtime confirm) |

### 3.2 CLI introspection, ranked by fidelity

**Tier 0: the framework emits a machine-readable spec.**
- **Swift ArgumentParser** (OBSERVED locally). `shortcuts --experimental-dump-help` printed JSON with `serializationVersion: 0`, `command.commandName`, `subcommands[]`, and `arguments[]`. Each argument has `kind` (`flag|option|positional`), `names[{kind:short|long,name}]`, `isOptional`, `isRepeating`, `valueName` and `abstract`. Example: `run` takes a positional `shortcut-name-or-identifier` plus `-i/--input-path` (repeating) and `-o/--output-path`. The flag is "experimental" and has no stability guarantee (UNKNOWN beyond the name).
- **usage spec (jdx/usage, KDL, MIT)** (OBSERVED, https://usage.jdx.dev/spec/):
  - Nodes: `bin`, `cmd`, `flag`, `arg`, `config`, `complete`, `mount` ("Use `mount` when a command's children are discovered at runtime").
  - From one spec it generates completions for five shells, markdown, man pages, SDKs and JSON Schema.
  - It has a first-class **`effect`** with values `read` ("Only inspects state. Running it twice is the same as running it once"), `write`, and `destructive` ("May delete or irreversibly overwrite something. Deserves a confirmation prompt").
  - Rules: "This is a coarse classification, not a permission model"; effect "is **not inherited by subcommands**"; "A flag or an argument can only ever _raise_ the effect, never lower it"; "A command with no `effect` is unknown, not safe — consumers should treat the absence of a value as 'ask'" (https://usage.jdx.dev/spec/#command-effects).
  - `usage mcp` serves a spec over MCP stdio with exactly two read-only tools, `list_commands` ("every command with its effect") and `describe_command`. It is a discovery server, not an executor (https://usage.jdx.dev/cli/reference/mcp).
  - It integrates with clap, Cobra and Effect. A Go framework is in preview.
  - INFERRED: this is the closest existing standard to the CLI half of agentic-anything's manifest. Adopt its effect semantics verbatim: absence = ask, flags only raise, no inheritance.
- **PowerShell** (OBSERVED, about_Functions_CmdletBindingAttribute):
  - `[CmdletBinding(SupportsShouldProcess, ConfirmImpact=...)]`. `SupportsShouldProcess` "adds **Confirm** and **WhatIf** parameters"; "**WhatIf** … lists the changes that the command would make, instead of running the command".
  - `ConfirmImpact` (default Medium) is compared to `$ConfirmPreference`.
  - `Get-Command`/`Get-Help` expose this metadata. INFERRED: PowerShell is the one mainstream CLI ecosystem with both an **effect level** and a **built-in dry run** in its command metadata.

**Tier 1: runtime completion protocols** (the CLI answers questions about itself).
- **Cobra** (OBSERVED locally):
  - `gh __complete pr ""` printed `checkout\tCheck out a pull request in git`, `close\t…`, `merge\t…` and so on, then `:4` and `Completion ended with directive: ShellCompDirectiveNoFileComp`.
  - `gh __complete pr list --` listed 16 flags with descriptions. The `--state` description embeds the enum as text: `{open|closed|merged|all}`.
  - `gh __complete pr list --state ""` returned the enum values directly.
  - `docker __complete container ""` works the same way.
  - Cobra therefore gives the tree, flag names and descriptions, and **dynamic enum values**. It does **not** give flag value types (string/int/bool) through this protocol.
  - Directive `:4` = no file completion. File-valued args show up as file-completion directives, which is a weak type hint.
- **Side-effect warning** (OBSERVED): `kubectl __complete get ""` logged `couldn't get current server API group list: Get "http://localhost:8080/api?timeout=32s"`. Dynamic completion runs `ValidArgsFunction` code, which can make network calls, read credentials or hit rate limits. Treat completion probing as `introspect` authority (executes foreign code). Run it with a scrubbed environment (no `KUBECONFIG`/tokens) unless the user opts in.
- **Click** (OBSERVED, https://click.palletsprojects.com/en/stable/shell-completion/):
  - The program is re-entered with `_{PROG}_COMPLETE={shell}_complete` and `COMP_WORDS`/`COMP_CWORD`.
  - Output items are `type,value` (`plain`, `file`, `dir`).
  - Custom `shell_complete` functions run arbitrary code.
- **Output-schema discovery** (OBSERVED): `gh pr list --json` with no field names prints the list of available JSON fields (`additions`, `assignees`, `author`, `autoMergeRequest`, `baseRefName`, …). Many modern CLIs have a similar self-describing output switch (`--json` field lists, `-o json`, `--format`). Probe for them by looking for `--json`/`--output`/`--format` flags in the harvested tree.

**Tier 2: curated external spec corpora.**
- **withfig/autocomplete** (OBSERVED via GitHub API): MIT, 25.2k stars, 715 top-level `src/*.ts` specs, last push 2025-05-05 (inactive since Fig's sunset). It is consumed by Microsoft `inshellisense` (MIT, 10.7k stars, active).
- The spec type (`autocomplete-tools/types/index.d.ts`, OBSERVED) has:
  - `subcommands`, `options`, `args`;
  - `generators`, which run shell commands to produce suggestions, so they are another side-effect vector;
  - `loadSpec` for lazy subtrees;
  - `requiresSeparator` and `isRepeatable`;
  - **`isDangerous`**: "If true, Fig will not enable its autoexecute functionality … used in the `rm` spec" (`index.d.ts:412-425`).
- **carapace** (OBSERVED via API): carapace-bin is MIT, 2.0k stars, actively pushed. It bridges multiple completion ecosystems (SECONDARY detail).
- INFERRED: Fig specs are a ready **effect prior** for ~700 common CLIs (`isDangerous`), but they are third-party and version-lagged. Use them as a fallback; never let them override the installed binary's own output.

**Tier 3: help and man parsing.**
- zsh ships `_gnu_generic`, which is literally `_arguments '*:arg: _default' --`. `_arguments --` calls `<cmd> --help` and parses long options (OBSERVED `/usr/share/zsh/5.9/functions/_arguments:86-98`).
- fish's `share/tools/create_manpage_completions.py` (72 KB, OBSERVED via API) has **seven** parser classes (`Type1`…`Type4`, `Scdoc`, `Darwin`, `Deroff`). That is a measure of how heterogeneous man-page formatting is.
- MCP wrappers that turn `--help` into tools (`any-cli-mcp-server`, `cli2mcp`, `cli-helper-mcp`; SECONDARY, directory listings) all depend on help-text consistency.
- INFERRED: help parsing is where an LLM adds real value: prose → typed flags, enums from `{a|b|c}`, defaults from "(default: x)". Its output must be cross-checked against tiers 0 to 2 and against the binary's parse errors (§6.2).

**Frameworks with no self-dump.** argparse, docopt and hand-rolled parsers (SECONDARY). If source is available, import the parser object or parse the docopt usage string, which *is* the grammar. Otherwise fall back to tier 3.

### 3.3 Self-describing protocols

| Protocol | Discovery call | What you get | Effect signal | Notes |
|---|---|---|---|---|
| OpenAPI 3.x | Fetch the spec (often `/openapi.json`) | `operationId`, params, request/response schemas, security schemes | HTTP method (RFC 9110 safe/idempotent) | FastMCP `from_openapi` maps **every** route to a tool by default; names come from `operationId` up to `__`, slugified, ≤56 chars; `RouteMap` → `TOOL/RESOURCE/RESOURCE_TEMPLATE/EXCLUDE`; "LLMs achieve significantly better performance with well-designed and curated MCP servers" (OBSERVED https://gofastmcp.com/integrations/openapi) |
| GraphQL | `{ __schema { queryType{name} mutationType{name} types{…} } }` | Full type system | Root type: query = read, mutation = write, subscription = stream; mutations optional ("if it is not provided, the service does not support mutations", spec §3.3.1) | "Disabling introspection in production is common" (OBSERVED https://graphql.org/learn/introspection/), so expect to fall back to SDL files or recorded traffic |
| gRPC | `grpc.reflection.v1.ServerReflection/ServerReflectionInfo` (bidi stream): `list_services`, `file_containing_symbol`, `file_by_filename`, … → `FileDescriptorProto` (OBSERVED reflection.proto) | Services, methods, message types | none (proto options are custom) | Optional server feature |
| D-Bus | `org.freedesktop.DBus.ListNames`/`ListActivatableNames` then `org.freedesktop.DBus.Introspectable.Introspect` → XML `<node><interface><method><arg direction><signal><property access>` | Typed members | Property `access` = `read`/`write`/`readwrite`; annotations `Deprecated`, `Method.NoReply`, `Property.EmitsChangedSignal` (OBSERVED dbus-specification) | Introspecting an activatable name **starts** the service. GApplication apps also export `org.gtk.Actions` (`List`, `Describe`, `DescribeAll`, `Activate`), which is an app-level action catalog, but GLib calls it a private implementation detail (SECONDARY) |
| COM / OLE Automation | Registry `HKCR\TypeLib`, `ITypeLib`/`ITypeInfo` (`IDispatch::GetTypeInfo`), PowerShell `Get-Member` | Interfaces, DISPIDs, param types | `INVOKEKIND`: `INVOKE_FUNC=1`, `INVOKE_PROPERTYGET=2`, `INVOKE_PROPERTYPUT=4`, `INVOKE_PROPERTYPUTREF=8` (OBSERVED learn.microsoft.com oaidl INVOKEKIND); property-get is a read candidate, put is a write | Late-bound objects without a typelib only reveal members via `GetIDsOfNames` guessing |
| AppleScript / JXA (OSA) | `Info.plist` `OSAScriptingDefinition` → bundle `.sdef` XML (the `sdef` tool needs full Xcode; OBSERVED error with CLT only) | Suites, commands with 4-char codes, typed parameters, classes, properties, elements | `<access-group identifier=…>` and `requires-access="r"` on parameters (OBSERVED Music.sdef `play`); `delete`/`duplicate`/`move` from the Standard Suite | Mail's sdef pulls in the Cocoa Standard Suite via `xi:include … xpointer(…)` that *excludes* delete/duplicate/move (OBSERVED `Mail.app/.../*.sdef:13`), so includes must be resolved. JXA uses the same dictionary |
| Windows App Actions | Package manifest → action-definition JSON; runtime `ActionCatalog.GetAllActions` | `id`, `description`, `inputs[{name,kind}]` (`ActionEntityKind`: File, Photo, Document, Text, StreamingText, RemoteFile, Table, Contact), `inputCombinations` with `where`, `invocation` (`uri` or `com` CLSID) | **none**; only `displaysUI`, `usesGenerativeAI`, `contentAgeRating`; `allowedAppInvokers` gates discovery ("If omitted or empty, no apps will be able to discover the action") | OBSERVED https://learn.microsoft.com/en-us/windows/ai/app-actions/actions-json (schema v3) |
| Apple App Intents | `<App>.app/Contents/Resources/Metadata.appintents/extract.actionsdata` (JSON, generator `xcode-tools`) | `actions{identifier,title,descriptionMetadata,parameters[{name,isOptional,valueType{entity|primitive}}],outputType,openAppWhenRun,isDiscoverable,authenticationPolicy,systemProtocols}`, `entities`, `queries`, `enums` | **`systemProtocols`**, OBSERVED distribution across 26 local bundles: `AssistantIntent` 57, `OpenEntity` 29, **`DeleteEntity` 13**, `AudioStarting` 12, **`CreateEntity` 11**, `PropertyUpdater` 6, `ChangeBinarySetting` 4, … `authenticationPolicy` nonzero on 11 of 230 actions | Books exposes 28 actions, Music 8. The metadata is readable, but third-party invocation goes through Shortcuts/Siri/Spotlight. The `shortcuts` CLI only has `run/list/view/sign` for *user-authored* shortcuts (OBSERVED). See `survey-os-automation-apis.md` §3.1 |
| Android AppFunctions | Jetpack `@AppFunctionDeclaration(isDescribedByKDoc = true)` → library "generates an XML schema file" indexed by the OS; callers use `AppFunctionManager`; debug with `adb shell cmd app_function list-app-functions` | Typed functions, KDoc descriptions, `@AppFunctionSerializable` types, app-level metadata | none declared | Experimental; "only a limited number of apps and system agents can access the entire pipeline"; caller needs `EXECUTE_APP_FUNCTIONS`; Android 16+ (OBSERVED https://developer.android.com/ai/appfunctions) |
| LSP / DAP | `initialize`: "The set of capabilities is exchanged between the client and server during the initialize request"; plus dynamic `client/registerCapability` (OBSERVED LSP 3.17) | Feature flags (`codeActionProvider`, …), `executeCommandProvider.commands` | none | Capabilities change at runtime, so re-read after registration events |
| CDP | `GET /json/protocol` on a debugging port, or the devtools-protocol repo JSON | OBSERVED (downloaded `browser_protocol.json` + `js_protocol.json` v1.3): 59 domains, **677 commands** (210 `experimental`, 40 `deprecated`), 232 events, typed params via `$ref` | **none** | Too large to expose 1:1; curate (see REA's closed scenario vocabulary in `rea-runtime-observation-surfaces.md`) |
| MCP | `tools/list`, `resources/list`, `prompts/list` | Tools with schemas and annotations | `readOnlyHint`/`destructiveHint`/`idempotentHint`/`openWorldHint`, which are *hints* and untrusted unless the server is trusted (see `survey-safety-rails-and-authorization-models.md` §3.3) | First-party MCP servers already exist for JetBrains, Unity, Figma and others (`survey-plugin-extension-points.md`) |

### 3.4 Plugin API reference docs and type stubs

- Type stubs are schemas: `.d.ts` (VS Code `vscode.d.ts`, Figma `plugin-typings`), `.pyi` (bpy fake-modules), Java/Kotlin SDK jars, and C# XML docs. INFERRED: parse them deterministically with the TypeScript compiler API, Python `ast`/`inspect`, or javap/reflection. Use an LLM only to choose *which* members become tools and to write descriptions.
- Prose reference docs (Sphinx, Doxygen, docs sites) are a retrieval corpus for an LLM synthesizer, not a source of truth. The verification rule (§6) is that every doc-derived signature must resolve against the stub or the runtime.
- Per-app surfaces and their security models are in `survey-plugin-extension-points.md`. The discovery-specific point is that **a plugin bridge is itself an introspection endpoint**: a bridge script inside the host (REA's Hopper/Ghidra pattern) can enumerate the host's live API (`dir(bpy.ops)`, `vscode.commands.getCommands()`, Unreal `remote/object/describe`). That makes the bridge the source of truth for its own capability manifest.

### 3.5 Config files and schemas

- Settings are capabilities: "set X" plus the oracle "read X back". Sources include JSON Schema (`$schema` in config files, SchemaStore catalog), VS Code `contributes.configuration`, GSettings schemas (`gsettings list-recursively`), dconf, macOS `defaults read <domain>` / plist, and the Windows registry. Detail is SECONDARY and was not reproduced here.
- REA already reads app state through **typed state views** (sqlite, json, xml, keyfile, dconf, text, binary) with diff kinds. It uses role precedence `application_state` → `document` → … (summarized in `rea-desktop-applications-and-agent-environments.md` §5). INFERRED: config discovery should emit *paired* capabilities, a write tool and a read-back view. Writing a config file the running app does not reload is a silent no-op, so it needs a "takes effect on restart" precondition.

### 3.6 Accessibility-tree crawling and UI exploration graphs

- **GUI ripping / UI Transition Graphs** (UTGs).
  - DroidBot (OBSERVED README, https://github.com/honeynet/droidbot) "does not require system modification or app instrumentation". It generates a UTG after testing, has a `-cv` mode for apps without accessibility views (Unity/Cocos games), and takes `-script` for custom inputs in given states.
  - The desktop analogue is the classic GUI-ripping work (Memon et al., GUITAR; SECONDARY).
- **LLM exploration → documentation**.
  - AppAgent (OBSERVED README) has two exploration modes, autonomous exploration and learning from human demonstration. In both it "generates documentation for elements interacted during the exploration/demonstration". `run.py` then uses that doc base, and success rate rises with doc coverage.
  - AutoDroid (OBSERVED arXiv 2308.15272 abstract) uses "exploration-based memory injection" and "functionality-aware UI representation", and reports 90.9% action accuracy and 71.3% task success on 158 tasks.
- **Exploration → callable tools**.
  - WALT (Salesforce, arXiv 2510.01524, ICLR 2026; SECONDARY via abstract/search; repo MIT) "reverse-engineers latent website functionality into … callable tools" (search, filter, sort, post, comment, create/edit/delete). It builds them with "validated input schemas" and URL-parameter promotion, and reports 252 tools discovered on Online-Mind2Web.
  - SkillWeaver (arXiv 2504.07079; SECONDARY) runs three stages: propose skills, synthesize practiced trajectories into Playwright functions, then test and debug them with environment feedback. APIs from strong agents transfer to weak ones.
- **Cheap deterministic inventories before exploring** (INFERRED, grounded in `survey-accessibility-trees.md`):
  - The **menu bar** (AX `AXMenuBar`, UIA MenuBar, dbusmenu) enumerates most document-level commands *with keyboard shortcuts* and enabled state, without clicking anything.
  - Toolbar buttons and context menus come second.
  - This yields a "command palette" capability list per app at `introspect` authority.
- **Static UI → handler mapping when the binary is yours to analyze.** REA `trace_native_ui_action` "joins UI wiring to uniquely matched class/selector implementations and follows bounded typed call references". It is explicit that "A static route does not establish runtime reachability" (`docs/native-investigation.md:421-428`). INFERRED: this can pre-classify a menu item's effect (e.g. the item reaches `-[NSDocument saveDocument:]`) before the item is ever clicked.
- **Risk**: exploration is the only discovery method that *acts*. Run it in a container/VM or a disposable profile with a seeded state. Snapshot and diff state per action (REA's checkpoint machinery, `rea-desktop-applications-and-agent-environments.md` G7/G9). Keep a deny-list of labels (Delete, Send, Purchase, Sign out, Format, Empty Trash) that the explorer must not press outside the sandbox.

### 3.7 Demonstration recording and traffic capture

- OpenAdapt (record → compile → replay) is covered in `survey-computer-use-vision-agents.md` and `survey-prior-art-...md`.
- **Playwright codegen** (OBSERVED https://playwright.dev/docs/codegen) records interactions as code, "prioritizing role, text and test id locators", and refines locators until they are unique. It offers inline assertions (visibility, text, value), and `--save-storage`/`--load-storage` (auth state, flagged as sensitive). INFERRED: the codegen output is a *verified locator plus assertion per step*, which is the right granularity for a golden recording.
- **REA HAR → tool surface** (OBSERVED):
  - The `api-surface` view groups request lines by method, origin and path template, and copies no query values, headers or bodies (`docs/web-network-captures.md:380-388`).
  - The `environment-manifest` view yields one tool per method + path template, with opt-in templating rules `after-uid-literal`, `opaque-token`, `sibling-variance`, `sibling-pair-variance` and `prefix-variance` ("Merged parameters are inferences", `src/domain/analysisView/environmentManifestView.ts:35`).
  - Annotations come from the method: `readOnlyHint: isSafeHttpMethod(method) || isQueryPost(method, path)`, `destructiveHint` only for DELETE, `idempotentHint` for safe/PUT/DELETE, `openWorldHint: false` (`src/application/environment/environmentTools.ts:67-76`).
  - Safe methods are `GET|HEAD|OPTIONS|TRACE` (`src/domain/analysisView/environmentStateRules.ts:274-278`).
  - **Query-POSTs are detected by a name heuristic**: path segments `search|query|list|find|fuzzy|lookup|count|aggregate|stats|_search|_count|_msearch` (`environmentStateRules.ts:280-292,440-443`).
  - INFERRED: so REA does use name heuristics for *synthesized* tools while forbidding them for its own hand-audited tools (`toolEffects.ts:765`, "without tool-name or tool-family heuristics"). agentic-anything should keep that split explicit, with heuristic effects carrying `confidence: inferred`.
- Demonstrations generalize poorly. One recording is one instance. WALT's "URL-parameter promotion" and REA's sibling-variance rules both turn *several* observations into a parameter. INFERRED: require ≥2 distinct observations before promoting a literal to a parameter, which matches REA's sibling rules.

### 3.8 Source-code analysis (when source is available and use is authorized)

- **CLI-Anything** (OBSERVED https://github.com/HKUDS/CLI-Anything, Apache-2.0, 58.3k stars, 69 `*/agent-harness` dirs; methodology in `cli-anything-plugin/HARNESS.md`, 747 lines).
  - **Phase 1, Codebase Analysis**: "Identify the backend engine", "Map GUI actions to API calls — Every button click, drag, and menu item corresponds to a function call", "Identify the data model", "Find existing CLI tools", and "Catalog the command/undo system — If the app has undo/redo, it likely uses a command pattern. These commands are your CLI operations" (HARNESS.md:12-23).
  - **Generated harness**: Click CLI + REPL + `--json` on every command + session undo/redo, then TEST.md, tests, SKILL.md (Phase 6.5), and PyPI publishing.
  - **Rule #1**: "Use the Real Software — Don't Reimplement It … The CLI MUST call the actual software for rendering and export" (HARNESS.md:307-312).
  - **Rendering gap**: project-file edits are silently dropped by naive renderers (HARNESS.md:348-366).
  - **Tests**: unit, native E2E, true-backend E2E (magic bytes, size > 1000, real output), installed-CLI subprocess tests. "Tests must NOT skip or fake results when the software is missing" (HARNESS.md:406-460).
  - **`/cli-anything:refine`** inventories current coverage, runs gap analysis against the source, and "confirm[s] which gaps to address" with the user (commands/refine.md:56).
  - **Gaps relevant here** (OBSERVED by grep): there is no effect or authority classification at all. The only hits for destructive/confirm/read-only are about undo and preview. There is no schema beyond Click, and no manifest other than SKILL.md and registry JSON.
- **REA static route inventory**: "never requests a route, executes scripts, expands a sitemap, or establishes reachability" (`docs/javascript-route-inventory.md:1-8`). This is the right epistemic stance for any source-derived capability: a *candidate* until runtime-confirmed.
- **Android's AppFunctions agent skill** (OBSERVED, developer.android.com/ai/appfunctions) is a vendor-blessed version of the same idea. It has four steps: **Discovery** ("Analyze your codebase to identify and recommend high-value features for AI orchestration"), Implementation, KDoc Refinement ("Optimize function and property documentation for AI agents"), and Testing via `adb shell cmd app_function`.
- **ToolMaker** (arXiv 2502.11705; SECONDARY) goes from repo + task to an installed, working tool, using a closed-loop self-correction against unit tests. Its benchmark is 15 tasks with more than 100 unit tests, and it reports 80% correct implementations.

---

## 4. Characterization: from raw surfaces to typed, classified capabilities

### 4.1 Normalize first (deterministic)
- Every discovery adapter emits the same candidate record (§2) with per-field provenance. Use one adapter per source family, mirroring REA's provider identity `{id, name, version}` (salvaged capability-registry summary).
- Resolve includes and indirections before anything else: sdef `xi:include` with xpointer exclusions, Fig `loadSpec`, usage `mount`, OpenAPI `$ref`, CDP `$ref`, LSP dynamic registration.
- Bind the candidate to the *installed* identity: bundle id + version + executable SHA-256. Do the same for a service: origin + spec digest. Discovery results are invalid after an upgrade (§6.6).

### 4.2 Schema inference
| Source | Deterministic mapping | Where inference is needed |
|---|---|---|
| Typed protocols (OpenAPI, GraphQL, gRPC, D-Bus signatures, COM VARTYPEs, sdef types, App Intents `valueType`, App Actions `kind`) | Direct type mapping to JSON Schema; enums from enum types; entities → `$ref` objects | Opaque `specifier`/`any` types (sdef `specifier`, D-Bus `v`, COM `VARIANT`) |
| Swift ArgumentParser dump / usage spec | `kind` → flag (bool) / option (string) / positional; `isRepeating` → array; `isOptional` → required | Value *types* beyond string |
| Cobra/Click completion | Names + descriptions; enums from `__complete` value lists; file directives → path | Types, defaults, mutual exclusion |
| Help text | Regex for `{a|b}`, `<int>`, `[default: x]`, `=VALUE` | Most of it (LLM) |
| Traffic (HAR) | REA templating; value-free body shapes | Generalization of literals; optional vs required |
| Outputs | `--json` field lists; JSON-sample schema induction (genson-style, SECONDARY) | Nullability, unions; mark `observed` not `declared` |

- Close every generated input schema with `additionalProperties: false`, so that "a misspelled parameter never degrades into an omitted one" (REA, `rea-desktop-applications-and-agent-environments.md` §8.2).
- Validate inferred CLI schemas **against the binary's own parser**. Invoke with each inferred flag and a deliberately wrong type, and expect a usage error with nonzero exit. A flag the parser rejects as unknown is a schema bug. This is cheap and deterministic, but it runs the binary, so use `probe` authority with a sandbox and scrubbed env (INFERRED).

### 4.3 Effect / authority classification

Native signals and how they map (OBSERVED signals; mapping INFERRED):

| Signal | Value → effect floor |
|---|---|
| usage `effect` | `read`→read, `write`→write, `destructive`→destructive; absent → **unknown (ask)** |
| Fig `isDangerous: true` | ≥ destructive |
| PowerShell `ConfirmImpact` High / Medium / Low; `SupportsShouldProcess` | High → destructive; Medium → write; `SupportsShouldProcess` → has dry run (`-WhatIf`) |
| PowerShell approved verb group (`Get-` vs `Set-`/`Remove-`) | weak prior only |
| App Intents `systemProtocols` | `DeleteEntity` → destructive; `CreateEntity`/`PropertyUpdater`/`EntityUpdating`/`ChangeBinarySetting` → write; `OpenEntity`/`ShowInAppStringSearchResults`/`Search` → ui/read; `AudioStarting`/`AudioRecording` → external/device; `authenticationPolicy ≠ 0` → requires-auth precondition |
| sdef | Standard Suite `delete`/`move`/`close saving no` → destructive; `make`/`set`/`duplicate` → write; `get`/`count`/`exists` → read; `requires-access="r"` → read access on that parameter; `access-group` → TCC/permission grouping |
| COM `INVOKEKIND` | `PROPERTYGET` → read candidate; `PROPERTYPUT(REF)` → write; `FUNC` → unknown |
| D-Bus property `access` | `read` → read; `write`/`readwrite` → write (setter); methods → unknown |
| GraphQL root | query → read; mutation → write (≥); subscription → stream/read |
| HTTP method | GET/HEAD/OPTIONS/TRACE → read; PUT/DELETE → idempotent write; DELETE → destructive; POST → write unless query-POST heuristic (inferred) |
| Windows App Actions | no effect field; `displaysUI` → ui; otherwise unknown |
| CDP / LSP `executeCommand` / generic methods | unknown (curate by hand) |
| Name/description keywords (`delete`, `remove`, `purge`, `reset`, `force`, `--yes`, `--force`, `send`, `publish`, `pay`) | can only **raise** |

Combination rules (INFERRED; the first three follow the usage spec and REA):
1. **Max over signals; absence = unknown; unknown = ask.** Flags and parameters can only raise the effect (`--force`, `--hard`, `-rf`, `--delete`). Effect is not inherited down a command tree.
2. **Effects are a vector, not a level.** Store REA's ten booleans (`mutatesTarget`, `mutatesSession`, `writesFilesystem`, `launchesProcess`, `accessesNetwork`, `changesUiState`, `mayDiscardData`, `executesForeignCode`, `mutatesDevice`, `idempotent`; `src/contracts/toolEffects.ts:2-22`). Derive MCP hints mechanically with REA's formula (`toolEffects.ts:765-787`):
   - `destructive = mayDiscardData || executesForeignCode || mutatesDevice`;
   - `openWorld = launchesProcess || accessesNetwork`.
   Add the coarse level (`read|ui|write|destructive|unknown`) as a *projection*, for policy and confirmation UX.
3. **Mandatory audit gate.** A generated tool without an effect decision does not ship. Compare REA's build failure "`Missing effect audit for ${name}`" (`toolEffects.ts:819-823`). An LLM may *propose* a level with a rationale, but it may only raise the deterministic floor, never lower it. Lowering requires sandbox evidence (rule 4) or a human.
4. **Observed effects override declared ones when the observation is stronger.** Run the tool in the sandbox with a state snapshot before and after: file tree hash of owned paths, typed state views, a11y snapshot, network log. An observed mutation from a "read" tool is a hard failure. A "destructive" tool that showed no discard in N varied runs stays destructive, because absence of evidence is not evidence.
5. **Untrusted descriptions.** Names, descriptions and enum labels come from the target. They are data, not instructions. REA marks served-environment tool text as captured data (`serveEnvironment.ts:104-145`, summarized in `rea-desktop-...md` §8.4). Keyword rules may raise effects from them, but nothing in them may lower an effect or grant authority.

### 4.4 Naming and granularity
- **Do not mirror.** CDP has 677 commands. REA's own ~170 tools cost ~120k tokens per request on hosts without tool search (`rea-mcp-server-adapter.md`, Client configuration notes). FastMCP warns that auto-conversion underperforms curation.
- Pattern (INFERRED, consistent with `survey-cli-ipc-protocol-wrapping.md` "Normalized schema"):
  1. Expose a curated **core set** of primitives per app, roughly 10 to 30, chosen by usage frequency, coverage of nouns × verbs, and read/write pairing.
  2. Put the full catalog behind `describe_capabilities(query)` and a typed `invoke(capability_id, args)` escape hatch. The escape hatch is subject to the same effect/policy checks.
  3. Group into families, as REA's `REA_MCP_TOOL_FAMILIES` does, so hosts can scope them.
- **Names**: `<app>.<noun>.<verb>` derived from the address. Examples: `gh.pr.list`, `music.track.play`, `books.book.open`. Keep the native address in metadata. Preserve vendor ids for traceability (`operationId`, sdef code, App Intent identifier). Slugify and truncate deterministically, with collision suffixes (FastMCP: ≤56 chars + numeric suffix).
- **Granularity**:
  - Prefer the app's own semantic units: App Intents, sdef commands, the command/undo system (CLI-Anything Phase 1.5), and Cobra leaf commands.
  - Workflows (multi-step) belong in a separate *recipe* layer, as Arazzo does for OpenAPI and CLI-Anything's REPL sessions do. They should not be primitives. This matches REA's rule "primitives over workflows" (salvaged tool-design summary).

---

## 5. Synthesis: where LLMs help and where they must be fenced

| Task | LLM role | Deterministic fence |
|---|---|---|
| Help/man/doc prose → typed flags | Extract types, enums, defaults, mutual exclusion | Parser-rejection probes (§4.2); must not contradict tier 0 to 2 sources |
| Grouping, naming, core-set selection | Propose families and the ~20 core tools; write descriptions aimed at agents (cf. the AppFunctions "KDoc Refinement" step) | Name grammar lint; every core tool must map to a discovered address |
| Effect proposal | Rationale + proposed level | May only raise the floor; lowering needs sandbox evidence or a human |
| Wrapper code for untyped surfaces (GUI, files, help-only CLIs) | Write the adapter (CLI-Anything: Click harness; SkillWeaver/WALT: Playwright/URL tools; ToolMaker: repo tool) | Must compile to the manifest schema; code runs only in the adapter process with declared effects; tests against the real app (§6) |
| Exploration policy | Pick the next UI element to try, summarize element function (AppAgent docs, AutoDroid memory) | Sandbox only; label deny-list; per-action state diff recorded as evidence |
| Test/scenario drafting | Draft seed state, steps, post-conditions | REA admission gate: the post-condition must fail on the seed and pass on the demonstrated end state; declarative checks, no regex (`rea-desktop-...md` §6) |

INFERRED principle: **the manifest is the product; code is an implementation detail.** LLM-written adapter code is admitted only through:
1. a manifest entry (schema + effects + oracle);
2. passing verification lanes;
3. a recorded catalog digest.

REA follows the same rule for skills: "do not use a skill as the primary repair for a discoverability or contract defect" (salvaged tool-design summary). Generated SKILL.md files (CLI-Anything Phase 6.5) are routing aids derived *from* the manifest, never a substitute for it.

---

## 6. Verification loops

### 6.1 Ladder (cheapest first; each rung gates the next)
1. **Static validation** (deterministic): the schema is valid JSON Schema, closed, and every `$ref` resolves. The effect audit exists. Names are unique. The address resolves against the discovered spec.
2. **Parser conformance** (CLI): valid inferred invocations pass `--help`-free parse checks, and invalid types and unknown flags produce usage errors. For RPC: a schema round-trip through the reflection descriptor.
3. **Native dry run where the app offers one** (detected from the harvested tree): PowerShell `-WhatIf`, `kubectl --dry-run=server`, `terraform plan`, `git … --dry-run`, `rsync -n`, `apt-get -s`. A tool whose source exposes a dry-run flag should expose `dry_run: true` as a first-class parameter (INFERRED).
4. **Sandboxed execution with state diff**: run each tool on a seeded disposable instance (container/VM/profile). Diff the owned paths, typed state views and a11y snapshot. Check `observed_effects ⊆ declared_effects`; a superset is a failure. REA's agentenv `computer_app` export (installs the real app in `ubuntu-24.04` with seed + readiness predicate) is a ready recipe for Linux GUI apps (`rea-desktop-...md` §7).
5. **Read-back oracle**: every write tool needs a read capability that confirms it. Generalize REA's confirmed-write admission ("served only when … a later read confirm[ed] its effect"). A write without an oracle is withheld with a reason, reusing REA's reason vocabulary: `no-observation`, `unconfirmed`, `contradicted`, `side-effect`, `unexplained-change`, … (`docs/agent-environments.md:175-186`).
6. **Golden recordings**: record (args → normalized output/state diff) on a reference instance. Replay on new versions and compare by category, as REA `environment-compare` does: statuses + normalized bodies, ID mapping by natural key, clocks compared by format, per-tool agreement, and diffs as JSON pointers capped at 20 per step (`docs/agent-environments.md:1067-1184`, `:1150-1152`). For UI paths, Playwright-style role locators + assertions per step.
7. **Differential checks**: when two surfaces reach the same capability, run both and compare. Examples: sdef `get name of every playlist` vs App Intent query vs a11y list; CLI `gh pr list --json` vs REST/GraphQL; a CDP command vs the UI. Disagreement flags a schema or effect bug. In REA, simulation vs live (`environment-compare`) is the same idea.
8. **Property-based / generative tests** for typed APIs:
   - Schemathesis "automatically generates property-based tests from OpenAPI and GraphQL schemas" and chains operations via OpenAPI links (OBSERVED schemathesis docs).
   - RESTler "infers producer-consumer dependencies among request types from the OpenAPI definition". Its fuzz mode "may create outages in the service under test" (OBSERVED github.com/microsoft/restler-fuzzer).
   - Only ever run either against an owned or sandboxed instance.
9. **Agent-level evaluation**: does a real agent pick the right tool? REA's `scripts/verify-agent-experience.mjs` scores transcripts on first-tool routing, repeated calls and schema failures. Known-answer rubrics bind claims to Evidence IDs (salvaged tool-design summary). Text heuristics "must not be read as factual assessments".

### 6.2 Rules carried over from CLI-Anything and REA
- **The real software is a hard dependency of the tests.** "Tests must NOT skip or fake results when the software is missing" (CLI-Anything HARNESS.md). REA's equivalent: "A lane must not imply that a host or target is covered when it was skipped" (salvaged).
- **"It ran without errors" is not sufficient.** Verify output artifacts with magic bytes, sizes and content probes (HARNESS.md "Output Verification Methodology").
- **An unreachable app is a run failure, not a score** (REA `verify.py`; `rea-desktop-...md` §6).
- **"A scripted sequence tests that sequence, not an agent's ability to discover it"** (REA `docs/testing.md`, via salvaged). Keep tool-correctness lanes separate from agent-routing lanes.

### 6.3 Drift
- Hash the harvested raw self-descriptions and the generated catalog, as REA records catalog SHA-256s and refuses to start on mismatch (salvaged capability-registry summary).
- On app upgrade (bundle version or exe hash change), re-run discovery and diff. Added, removed and changed capabilities go through admission again. Unchanged ones keep their verification status only if the golden replay still agrees (INFERRED).

---

## 7. Proposed pipeline: discover → characterize → synthesize → verify → package

```
target (install path | bundle id | service origin | repo)
  │
  ├─1 DISCOVER ─ adapters by authority: static → introspect → probe → explore
  │     raw/ (verbatim self-descriptions + digests) + candidates.jsonl (with provenance)
  ├─2 CHARACTERIZE ─ normalize, resolve includes, schema projection, effect floors, oracle pairing
  │     capabilities.jsonl (typed, effect vector + level + confidence, unknowns explicit)
  ├─3 SYNTHESIZE ─ core-set selection, naming, descriptions, adapter code for untyped surfaces, recipes
  │     manifest.json (tools, families, recipes) + adapters/ + SKILL.md (derived)
  ├─4 VERIFY ─ ladder §6.1 in sandbox lanes; admission gates; withheld[] with reasons
  │     verification.json (per-tool lane results, golden recordings, observed effects)
  └─5 PACKAGE ─ catalog digest, MCP server + CLI from the same manifest, client registration
        agent interface (MCP + CLI) + drift baseline
```

| Step | Sub-step | Deterministic? | LLM? | Gate / artifact |
|---|---|---|---|---|
| 1 Discover | Install inspection (Info.plist, `.sdef`, `Metadata.appintents`, MSIX manifest/App Actions JSON, `.desktop` + D-Bus service files, TypeLib registry, package manifests; REA `inspect_desktop_application` families) | Yes | No | `static` authority; raw digests |
| | Self-description calls (`--experimental-dump-help`, `__complete`, `_X_COMPLETE`, `/openapi.json`, `__schema`, gRPC reflection, `Introspect`, `ITypeInfo`, `/json/protocol`, LSP/DAP `initialize`, MCP `tools/list`, `adb shell cmd app_function list-app-functions`, AX menu bar) | Yes | No | `introspect` authority; scrubbed env; network off unless opted in |
| | `--help`/man harvest | Yes | No | raw text kept verbatim |
| | Source analysis (if source) | Partly (AST, route inventory) | Yes (CLI-Anything Phase 1 style mapping) | Candidates marked `inferred` |
| | UI exploration / demonstration / traffic | Recording yes; policy no | Yes (exploration policy) | `explore` authority: sandbox only, label deny-list |
| 2 Characterize | Normalize + resolve includes + identity binding | Yes | No | |
| | Schema projection from typed sources | Yes | No | closed schemas |
| | Schema extraction from prose | No | Yes | must pass parser-conformance probes |
| | Effect floors from native signals (§4.3 table) | Yes | No | max-combine; absence = unknown |
| | Effect proposals | No | Yes | raise-only |
| | Oracle pairing (write ↔ read-back) | Mostly (same entity/noun) | Assist | writes without an oracle → `withheld` |
| 3 Synthesize | Core set, families, names, descriptions | Lint deterministic | Yes | every tool maps to a discovered address |
| | Adapter code for untyped surfaces | No | Yes | runs only in adapter process; declared effects |
| | Recipes (multi-step) | No | Yes | composed only of admitted tools |
| 4 Verify | Ladder rungs 1-3 | Yes | No | |
| | Rungs 4-8 (sandbox, read-back, golden, differential, property-based) | Yes (execution and comparison) | Scenario drafting only | REA gates: confirmed-write; seed-fails/end-passes; observed ⊆ declared |
| | Rung 9 (agent eval) | Scoring yes | Agent under test | separate lane |
| 5 Package | Catalog digest, MCP + CLI projection, SKILL.md derivation, client registration (REA `rea setup` plan/approve/transact) | Yes | No | catalog hash; drift baseline |

INFERRED sizing: steps 1, 2 (typed half), 4 and 5 are ordinary engineering and can be built and tested without models. The LLM-assisted parts are bounded to the untyped half of step 2, step 3, and exploration in step 1. All of them produce artifacts that the deterministic stages accept or reject.

---

## What to distill for agentic-anything

1. **A `Capability` record and a `DiscoveryAdapter` contract** (one per source family). Each adapter returns verbatim raw descriptions + digest, candidate capabilities with per-field provenance (`declared|observed|inferred`), and REA-style `next_probes` (`operation`, `rationale`, `confidence`). Discovery is always a pure function of `(target identity, authority level, adapter version)` → cached Evidence.
2. **Discovery authority levels**: `static` (read files) < `introspect` (call self-description; may execute target code, e.g. completion) < `probe` (invoke with test args) < `explore` (drive UI or write state). Run `introspect` with a scrubbed environment (no `KUBECONFIG`/tokens/proxies) by default. `probe` and `explore` require a sandbox lane and set `Evidence.isolation` truthfully.
3. **Ship these tier-0/tier-1 harvesters first**, since they are high yield and low risk, and each was OBSERVED working here:
   - Swift ArgumentParser `--experimental-dump-help`;
   - Cobra `__complete` (tree + flags + enum values);
   - Click `_X_COMPLETE`;
   - usage KDL;
   - gh-style `--json` field lists;
   - `.sdef` with `xi:include` resolution;
   - `Metadata.appintents/extract.actionsdata`;
   - Windows App Actions JSON;
   - D-Bus `Introspect`;
   - COM `ITypeInfo`;
   - OpenAPI, GraphQL `__schema`, gRPC reflection;
   - CDP `/json/protocol`;
   - LSP/DAP `initialize`;
   - MCP `tools/list`;
   - the AX/UIA menu bar.
   Help/man parsing and Fig specs are fallbacks.
4. **Adopt the usage-spec effect semantics as the CLI effect contract**: `read|write|destructive`, absence = ask, flags only raise, no inheritance. Internally, store REA's ten-boolean `ToolEffects` vector and derive MCP annotations with REA's exact formula (`toolEffects.ts:765-787`). Consider emitting a usage KDL spec as one package output, so that tools like `usage mcp` and shell completions consume the same truth.
5. **An effect-signal table as code** (§4.3), with max-combination, raise-only keyword rules, raise-only LLM proposals, and a build-failing **mandatory effect audit** for every generated tool, as REA's "Missing effect audit" does.
6. **Confirmed-write admission, generalized.** Every write capability needs a paired read-back oracle (app API read, typed state view, or a11y predicate). Without one, it is withheld with REA's reason codes plus hints. Every post-condition must fail on the seed and pass on a demonstrated end state.
7. **`observed ⊆ declared` effects check** in the sandbox lane. Use snapshot/diff over owned paths, state views, a11y and network per call. Any undeclared observed effect fails the tool.
8. **First-class `dry_run`** wherever the source exposes one (`-WhatIf`, `--dry-run[=server]`, `plan`, `-n`, `-s`). Policy can then require a dry run before a destructive call, with the dry-run output attached to the confirmation prompt.
9. **Curate, don't mirror.** Per app: a core set of about 30 or fewer tools, a `describe_capabilities(query)` resource over the full catalog, and a policy-checked `invoke(capability_id, args)` escape hatch. Families for host scoping. Deterministic names `<app>.<noun>.<verb>` that keep the vendor id.
10. **Golden + differential lanes per app**: record goldens on a reference instance; compare with REA `environment-compare` categories; cross-check capabilities reachable via two surfaces. Re-run on version change, and use catalog digests as drift baselines.
11. **LLM synthesis is a proposal stage whose only output is manifest entries + adapter code.** CLI-Anything's SOP is a good template for the source-available case: backend-engine identification, GUI-action → API mapping, the undo/command system as the operation list, "use the real software", and no-skip E2E. Add the effect and oracle columns it lacks. SKILL.md is generated *from* the manifest.
12. **Treat everything harvested from the target as untrusted content**: descriptions, enum labels, help text, a11y labels, exploration screenshots. Keyword rules may raise effects from it; nothing in it may lower effects, add authority, or be forwarded as instructions.

## Open questions

1. Should agentic-anything adopt usage KDL as its CLI manifest format (and contribute effect vectors upstream), or keep a private JSON manifest and *export* usage KDL? Is `--experimental-dump-help` stable enough to depend on? It is still marked experimental (UNKNOWN).
2. How do we invoke App Intents for third-party agents? The metadata is fully readable, but invocation goes through Shortcuts/Siri/Spotlight. Is "generate a user-approved Shortcut per intent, then `shortcuts run`" acceptable UX? Will Apple ship a public MCP path (see `survey-prior-art-...md` open questions)?
3. What sandbox gives exploration-grade isolation per OS? Linux has REA's agentenv container recipe. What are the equivalents for macOS (VM with TCC grants?) and Windows (Windows Sandbox / VM)? What do we do for apps whose state is cloud-backed, where a sandbox cannot isolate server-side effects?
4. How much of the CLI type gap (Cobra and Click completion carry no types) can parser-rejection probing close without unacceptable side effects, and which CLIs have argument validation that itself mutates state (e.g. lazy config initialization)?
5. Effect confidence calibration: how many varied sandbox runs justify lowering an LLM- or keyword-raised effect? Should lowering *always* require a human?
6. Fig specs are inactive since 2025-05. Is there a maintained corpus (carapace, inshellisense forks) with danger flags, and what is its licensing for redistribution inside our package?
7. For GraphQL services with introspection disabled and for gRPC without reflection: is traffic-derived synthesis (the REA HAR path) acceptable, given the authorization scope (the owner's own services only)?
8. How should menu-bar and a11y-derived capabilities be keyed so they survive localization and app updates (AX identifier vs title vs keyboard shortcut vs selector from `trace_native_ui_action`)?
9. Where does recipe (multi-step workflow) verification live? Arazzo-style declarative workflows over admitted tools, or code? How are their effects aggregated (max over steps, with the confirmation point at the first destructive step)?

## Sources

Local observations (this machine, 2026-11-05):
- `gh __complete pr ""`, `gh __complete pr list --`, `gh __complete pr list --state ""`, `gh pr list --json`; `docker __complete container ""`; `kubectl __complete get ""` (network attempt).
- `shortcuts help`, `shortcuts --experimental-dump-help`.
- `/System/Applications/*/Contents/Resources/Metadata.appintents/extract.actionsdata` (26 bundles including `/Applications`); `.sdef` files via `Info.plist` `OSAScriptingDefinition` (Music, Mail, Microsoft Word/Excel/PowerPoint, …).
- `/usr/share/zsh/5.9/functions/_gnu_generic`, `_arguments:86-98`.
- CDP JSON: https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/json/browser_protocol.json and `js_protocol.json`.
- Fig types: https://raw.githubusercontent.com/withfig/autocomplete-tools/main/types/index.d.ts (lines 412-425, 642, 796, 804).
- CLI-Anything: `cli-anything-plugin/HARNESS.md`, `commands/refine.md`, `commands/validate.md` via GitHub API.

REA (local checkout `/Users/nhanclie/rea`):
- `src/contracts/toolEffects.ts:1-30,54,765-787,800-831`
- `src/application/environment/environmentTools.ts:42-76`
- `src/domain/analysisView/environmentStateRules.ts:270-292,440-448`
- `src/domain/analysisView/environmentManifestView.ts:35`
- `src/domain/desktopApplication.ts:9-17,166-179,207`
- `src/domain/artifactInspection.ts:114,148-158,326-374`
- `docs/agent-environments.md:175-186,1067-1184`; `docs/web-network-captures.md:380-400`; `docs/javascript-route-inventory.md:1-8`; `docs/native-investigation.md:421-428`; `docs/tool-design.md:159`

Web (primary unless noted):
- usage spec: https://usage.jdx.dev/spec/ ; https://usage.jdx.dev/spec/#command-effects ; https://usage.jdx.dev/spec/reference/cmd ; https://usage.jdx.dev/cli/reference/mcp
- Click shell completion: https://click.palletsprojects.com/en/stable/shell-completion/
- Cobra completion guide: https://cobra.dev/docs/how-to-guides/shell-completion/
- PowerShell CmdletBinding: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_functions_cmdletbindingattribute
- COM INVOKEKIND: https://learn.microsoft.com/en-us/windows/win32/api/oaidl/ne-oaidl-invokekind
- D-Bus specification: https://dbus.freedesktop.org/doc/dbus-specification.html
- GApplication D-Bus API (SECONDARY): https://wiki.gnome.org/Projects/GLib/GApplication/DBusAPI
- gRPC reflection proto: https://raw.githubusercontent.com/grpc/grpc-proto/master/grpc/reflection/v1/reflection.proto ; https://github.com/grpc/grpc/blob/master/doc/server-reflection.md
- GraphQL: https://spec.graphql.org/October2021/ ; https://graphql.org/learn/introspection/
- LSP 3.17: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/
- Windows App Actions JSON: https://learn.microsoft.com/en-us/windows/ai/app-actions/actions-json
- Android AppFunctions: https://developer.android.com/ai/appfunctions
- FastMCP OpenAPI: https://gofastmcp.com/integrations/openapi
- CLI-Anything: https://github.com/HKUDS/CLI-Anything
- withfig/autocomplete: https://github.com/withfig/autocomplete ; inshellisense: https://github.com/microsoft/inshellisense ; carapace: https://github.com/carapace-sh/carapace-bin
- fish man-page parser: https://github.com/fish-shell/fish-shell/blob/master/share/tools/create_manpage_completions.py
- DroidBot: https://github.com/honeynet/droidbot ; AppAgent: https://github.com/TencentQQGYLab/AppAgent ; AutoDroid: https://arxiv.org/abs/2308.15272
- WALT (SECONDARY): https://arxiv.org/abs/2510.01524 ; https://github.com/SalesforceAIResearch/WALT
- SkillWeaver (SECONDARY): https://arxiv.org/abs/2504.07079
- ToolMaker (SECONDARY): https://arxiv.org/abs/2502.11705
- Playwright codegen: https://playwright.dev/docs/codegen
- Schemathesis: https://schemathesis.readthedocs.io/en/stable/ ; RESTler: https://github.com/microsoft/restler-fuzzer
- Help-to-MCP wrappers (SECONDARY): https://enterprisedna.co/directories/mcp/eirikb-any-cli-mcp-server ; https://lobehub.com/mcp/jugaad-lab-cli-helper-mcp
