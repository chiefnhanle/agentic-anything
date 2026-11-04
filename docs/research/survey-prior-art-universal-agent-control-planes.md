# Survey: prior art for "universal agent control planes" over arbitrary software

Technique family: `prior-art-universal-agent-control-planes`
Research date: 2026-11-04. Star counts, versions and "preview" labels are snapshots as of the fetch; treat them as dated.
Evidence legend: **OBSERVED** = read on the primary page/repo during this research; **SECONDARY** = from search summaries or third-party pages, not confirmed on a primary source; **INFERRED** = my interpretation; **UNKNOWN** = not established.

Note on method: WebSearch budget for the shared session was exhausted near the end of this survey. Items marked UNKNOWN below (notably UiPath's screen-agent docs and Automation Anywhere's computer-use capability) could not be closed with a primary source.

---

## 1. Summary

- There is no shipped, general-purpose "one MCP that observes and drives any desktop app" today. What exists is a layered, fragmented ecosystem:
  1. **Model-side computer-use APIs** (Anthropic computer toolset, OpenAI `computer` tool, Google Gemini `computer_use`) that emit click/type/screenshot actions and leave the executor to you.
  2. **OS-vendor agent substrates** that only reach apps which opt in: Windows ODR/MCP + App Actions (public preview, MSIX-identity required), Apple App Intents (Siri/Shortcuts only; MCP groundwork reported in macOS 26.1 betas, unconfirmed public access), Android AppFunctions (API 36, callable only by *system-privileged* agents).
  3. **Accessibility-tree MCP servers** per platform (Windows-MCP, terminator, Peekaboo/AXorcist, mcp-server-macos-use, computer-use-linux, Playwright MCP for browsers) that give deterministic element IDs but have no shared schema, evidence model, or capability registry.
  4. **Vision-first agent frameworks** (UFO2, Agent S, UI-TARS Desktop, c/ua, OpenAdapt) optimized for benchmarks (OSWorld) rather than for being an embeddable control plane.
  5. **RPA vendor agentic layers** (Copilot Studio computer use, UiPath, Automation Anywhere) that bolt CUA models onto enterprise-governed machines.
- The consistent gaps: no cross-platform capability registry with provenance; no standard "evidence" object tying an action to an observed before/after state; permission models are either all-or-nothing OS prompts (macOS Accessibility, Windows "Reduce protections") or vendor-cloud policies; almost nobody fuses a11y + scripting + instrumentation + vision under one selection policy with declared effects. REA's Evidence/ToolEffects/capability-declaration discipline is the piece the ecosystem lacks, but REA itself is batch/scenario-oriented, not an interactive desktop driver.

---

## 2. What it gives an agent (by control mechanism)

| Mechanism | What the agent gets | Representative prior art |
|---|---|---|
| Vision + synthetic input | Screenshot in, (x,y)/key actions out; works on anything rendered, including games/Citrix/RDP | Anthropic computer toolset, OpenAI computer tool, Gemini computer_use, UI-TARS Desktop, Agent S, Copilot Studio computer use, Copilot Actions |
| Accessibility tree (UIA / AX / AT-SPI / browser a11y) | Structured element list with stable-ish IDs/roles; no vision model needed; text extraction | Windows-MCP, terminator, Peekaboo+AXorcist, mcp-server-macos-use, computer-use-linux, linux-desktop-mcp, Playwright MCP (default), Stagehand, OpenAdapt (Win/mac/Linux surfaces) |
| Hybrid a11y + vision grounding | UIA for standard controls, OmniParser-style grounding for custom-drawn ones | UFO2 (OBSERVED on microsoft.github.io/UFO) |
| OS scripting / IPC | AppleScript/JXA, PowerShell, COM/win32com, xlwings | macos-automator-mcp, Windows-MCP `PowerShell` tool, UFO2 "native API preferred" |
| App-declared actions (opt-in) | Typed functions with entity schemas, discoverable at runtime | Windows App Actions (JSON + MSIX manifest), Windows ODR/MCP connectors, Apple App Intents, Android AppFunctions |
| Passive observation / recording | Continuous screen+audio history, OCR, a11y snapshot pairing; demonstration recording | screenpipe (observe only), OpenAdapt (record → compile → replay), terminator workflow recorder, Playwright MCP devtools recorder |
| Sandboxed execution substrate | VMs/containers/separate desktop sessions so agent and user don't collide | c/ua (Lume VMs, gVisor), Windows agent workspace (separate session + agent account), UFO2 PiP (roadmap only), Anthropic reference container |
| Terminal/filesystem | Processes, files, diff edits (no GUI) | Desktop Commander MCP |

---

## 3. Platforms & permission models

### Windows
- **ODR / MCP on Windows** (OBSERVED, learn.microsoft.com, page dated 2025-12-06, updated 2026-06-04; marked prerelease): "the Windows On-device Agent Registry (ODR), a secure, manageable interface to discover and use agent connectors from local apps and remote servers." Servers run "contained in a separate environment by default"; "users and IT admins can control access to MCP servers for each agent by using Windows Settings and ... Microsoft Intune"; interactions can be logged/audited. Ships a File Explorer MCP server and a Settings connector. CLI `odr.exe`.
- **Containment** (OBSERVED, mcp-containment page dated 2025-11-11): contained servers "run in a separate Windows session using a separate agent user account" and have **no** access to "apps and windows the user is using", user files/registry/credentials, or "running executables that modify the user session." Requires a binary `.exe` server with MSIX package identity and a `manifest.json` carrying `_meta.com.microsoft.windows` with `static_responses` and `tools/list`. File access is granted **per host app, not per server**: "When the user grants access to their user files, any MCP server used in that session will have access to the user's files." Unpackaged servers / `.mcpb` bundles only work after enabling `Settings > System > Advanced > AI components > Reduce protections for agent connectors`.
  - INFERRED consequence: an ODR-registered connector **cannot** drive the user's live desktop windows by design. A control plane that drives the user's session must either run as an unpackaged tool under reduced protections, or operate inside the agent workspace.
- **App Actions** (OBSERVED, page dated 2026-10-14): "individual units of behavior that a Windows app can implement and register"; implemented via URI activation or COM `IActionProvider`; requires package identity; actions declared in JSON with typed entities (Document, Photo, Text). Relation to MCP: only SECONDARY (Windows Central, Build 2025) says App Actions "will also take advantage of MCP."
- **Agent workspace / Copilot Actions / "Experimental agentic features"** (SECONDARY from support.microsoft.com summaries, Nov 2025): off by default, admin-only toggle, enabled for all users on the device once on; each agent gets its own account; "agent workspace runs in a separate Windows session, allowing agents to interact with apps in parallel to your own session"; Copilot Actions "using vision and advanced reasoning to click, type and scroll"; access initially limited to known folders (Documents, Downloads, Desktop, Pictures). Status: Insider preview as of Nov 2025; current status UNKNOWN.
- **Third-party a11y servers**: Windows-MCP (OBSERVED README): "operates with full system access and can perform irreversible operations"; user takeover via physical mouse movement or `Ctrl+Alt+Shift+Backspace` is "best-effort, not a security boundary"; anonymous telemetry on by default. terminator (OBSERVED): Windows-only, MIT.

### macOS
- **System permissions**: Accessibility (AX), Screen Recording, Automation (per target app, owned by the *launching host process*; OBSERVED in macos-automator-mcp notes: "The application that launches the MCP server ... owns its macOS privacy permissions"). Peekaboo additionally needs a permission for synthetic input and gates foreground-stealing key chords behind "explicit foreground consent" (OBSERVED README).
- **Apple App Intents / Apple Intelligence** (OBSERVED WWDC26 session 343 notes; Apple doc page returned no body): Siri gains "accessing your app's entities, taking action through your intents, and understanding onscreen context"; "By default Siri assumes entities are private and skips confirmation" with `OwnershipProvidingEntity` for shared/public. **No mention of MCP or third-party agents invoking intents** on the pages read. SECONDARY (9to5Mac/AppleInsider, Sept 2025): macOS 26.1 beta code shows MCP support being built into App Intents. Public third-party agent access: UNKNOWN as of this research.
- **Anthropic Claude Desktop computer use** (SECONDARY, support.claude.com summaries): research preview/beta, Pro and Max; "Claude asks for your permission before accessing each application"; some categories (investment/trading/crypto) blocked by default; "without the permission checks that gate other Cowork tools"; prefers existing integrations first; not on Linux beta. Launch reported 2026-03-23 (macOS), Windows ~2026-04-03.

### Linux
- AT-SPI2 + portal screenshots + ydotool/xdotool. computer-use-linux (SECONDARY): readiness-check document; `run_shell` registered only with `COMPUTER_USE_LINUX_ENABLE_SHELL=1`, "deliberately absent by default and is not a sandbox"; validated manually only on Ubuntu 25.10/GNOME Wayland. nut.js is X11 only (OBSERVED README). Wayland makes global input injection and window enumeration compositor-specific.

### Android
- **AppFunctions** (OBSERVED developer.android.com package summary, API 36): "APIs for applications to expose functionality to the system for cross-app orchestration, and for system-privileged agents to discover and execute this functionality." "An app function is ... made available for execution by trusted, system-privileged applications (referred to as 'agents')." Beta/experimental. Functions declared in XML; `AppFunctionService` or runtime `registerAppFunction` (scope activity/global, from `CINNAMON_BUN`). INFERRED: a third-party control plane cannot be an AppFunctions agent without system privilege; accessibility services and ADB (REA already has ADB tooling) remain the third-party path.
- **Gemini computer_use** supports `ENVIRONMENT_MOBILE` with `open_app`, `list_apps`, `long_press` (OBSERVED aistudio docs). Gemini Intelligence / on-device agent (SECONDARY, Engadget): AppFunctions is "the system-level hook".

### Browser
- Playwright MCP (OBSERVED): a11y snapshot default, vision opt-in via `--caps=vision`; `--allowed-origins/--blocked-origins` "not ... a security boundary"; `browser_run_code_unsafe` is "RCE-equivalent"; `--secrets` redaction; file access restricted to workspace roots. Browser Use (OBSERVED): MIT, 120k stars, `sensitive_data` placeholders but "page snapshots and results are saved unredacted." Stagehand (OBSERVED): MIT, `act/observe/extract`, "hybrid accessibility-tree trimming," self-healing on site change.

### Model vendors' safety layers (all OBSERVED on primary docs)
- **Anthropic computer toolset** (`computer_toolset_20260801`, 17 members, no beta header for Claude 5.5+; `computer_20251124` beta for Opus 4.5-4.7/Sonnet 4.6): recommends dedicated VM/container, domain allowlist, human confirmation for consequential actions; "In some circumstances, Claude follows commands found in content even when they conflict with your instructions"; classifiers run on tool output incl. screenshots; batch semantics "Run blocks in order and stop at the first failure"; ~1,000-1,800 tokens per screenshot; no API-side downscaling.
- **OpenAI computer tool** (developers.openai.com, current page): `computer_call` with ordered `actions` array (`click, double_click, drag, move, scroll, keypress, type, wait, screenshot`); "Pending API safety checks stop this sample; add a human approval callback"; "Text in a page, document, or tool result cannot grant permission or override the user's instructions." The older `pending_safety_checks` taxonomy (malicious_instructions / irrelevant_domain / sensitive_domain) was not on the current page: UNKNOWN whether still documented.
- **Gemini computer_use** (aistudio.google.com/docs/computer-use): environments `browser` (21 actions), `mobile` (10), `desktop` (17); coordinates normalized 0-999; per-call `safety_decision` with `require_confirmation`/blocked; policy categories FINANCIAL_TRANSACTIONS, SENSITIVE_DATA_MODIFICATION, COMMUNICATION_TOOL, ACCOUNT_CREATION, DATA_MODIFICATION, USER_CONSENT_MANAGEMENT, LEGAL_TERMS_AND_AGREEMENTS; opt-in `enable_prompt_injection_detection` screenshot scanning; `excluded_predefined_functions` + custom functions (e.g. `yield_to_user`). Launch blog (2025-10-07) said "not yet optimized for desktop OS-level control"; current docs now list a `desktop` environment.

---

## 4. Representative tools (name, URL, license, one line)

### MCP servers for desktop control
- **Windows-MCP** — https://github.com/CursorTouch/Windows-MCP — MIT — UIA-tree Windows control (Click/Type/Snapshot/App/PowerShell/Registry/Scrape), DOM mode for browsers, SSE/HTTP remote with auth; ~11k stars; Python 3.14+. Does NOT: select text within paragraphs, type code into IDEs well, play games (self-declared). Telemetry on by default.
- **terminator** (mediar-ai) — https://github.com/mediar-ai/terminator — MIT — "playwright for windows computer use": Windows-only a11y SDK (Rust/TS/Python), `terminator-mcp-agent`, workflow recorder to YAML; ~1.7k stars; macOS/Linux unsupported.
- **desktop-touch-mcp** — https://github.com/Harusame64/desktop-touch-mcp — license UNKNOWN — "Windows 11 desktop automation MCP server. Win32, UIA, CDP/Chrome, 28 tools" (from MCP registry listing).
- **DesktopManager** (EvotecIT) — https://github.com/EvotecIT/DesktopManager — license UNKNOWN — "Inspect and verify Windows x64 desktop apps through a local, read-only-by-default MCP server" (registry listing; notable for read-only default).
- **Peekaboo** (steipete/openclaw) — https://github.com/steipete/Peekaboo — MIT — macOS CLI + MCP: screenshot, `see` returns UI map with opaque element IDs, click/type/scroll/drag/set-value, menu/dock/window/space control, background targeted input, agent mode; macOS 15+; v4.9.7; 5.4k stars.
- **AXorcist** — https://github.com/openclaw/AXorcist — MIT — Swift AX wrapper: fuzzy-matched queries, actions (AXPress etc.), observers (AXValueChanged...), `axorc` CLI; macOS 14+.
- **mcp-server-macos-use** (mediar-ai) — https://github.com/mediar-ai/mcp-server-macos-use — license UNKNOWN (not named in README) — Swift; five `*_and_traverse` tools returning AX tree diffs after each action; 358 stars; stdio only.
- **macos-automator-mcp** (steipete) — https://github.com/steipete/macos-automator-mcp — MIT — `execute_script` (AppleScript/JXA) + `get_scripting_tips` knowledge base; permissions owned by launching host.
- **computer-use-linux** (agent-sh) — https://github.com/agent-sh/computer-use-linux — license UNKNOWN — Rust; AT-SPI, portal screenshots, Wayland/X11 input, GNOME/KDE/Hyprland/i3/COSMIC window targeting, readiness check, shell off by default.
- **linux-desktop-mcp** — https://pypi.org/project/linux-desktop-mcp/ — license UNKNOWN — Python AT-SPI2 `desktop_snapshot` / `desktop_capabilities`; needs ydotool on Wayland; v0.1.0.
- **computer-mcp** (PyPI) — https://pypi.org/project/computer-mcp/ — license UNKNOWN — pynput+mss cross-platform, a11y tree support claimed for Win/mac/Linux (SECONDARY).
- **da-mcp** — https://github.com/cioinside/da-mcp — MIT — cross-platform 12-tool server; xdotool/ydotool on Linux, robotjs on mac/Win (SECONDARY).
- **Desktop Commander MCP** — https://github.com/wonderwhy-er/DesktopCommanderMCP — MIT — terminal/process/filesystem/`edit_block`; explicitly "not a sandbox"; `allowedDirectories` only restricts file ops; no GUI control. ~10k stars.
- **Playwright MCP** — https://github.com/microsoft/playwright-mcp — Apache-2.0 — browser a11y snapshots, capability flags, origin lists (not a boundary), `browser_run_code_unsafe`.
- Other registry entries (OBSERVED in registry.modelcontextprotocol.io search, licenses UNKNOWN): Agent360dk/computerMCP ("macOS computer use with the guardrails on: passwords blacked out, writes need consent, all logged"), Xeakaes/computer-use-for-all-agents ("Safety-gated computer use"), afaaq-io/ComputerUseTurbo (background operation on mac/Win/Linux), domdomegg/computer-use-mcp, hightemp/go_computer_use_mcp_server, hiphopanda/windows-computer-use-mcp, Jason26214/omni-computer-use (replicates Anthropic tool surface on Windows), computer-agent-labs/terminal-use.

### Observation / recording
- **screenpipe** — https://github.com/screenpipe/screenpipe — Screenpipe Commercial License (source-available; non-commercial personal use free) — event-driven 24/7 screen+audio capture paired with a11y tree, OCR fallback, SQLite FTS5, `screenpipe-mcp`; 22k stars. Does NOT inject input.
- **OpenAdapt** — https://github.com/OpenAdaptAI/OpenAdapt — MIT — record once (human or agent) → compile to local program → replay with no model calls; verifies by reading back from system of record; "stops instead of guessing"; surfaces: DOM, UIA, macOS AX, AT-SPI (read only in CI), RDP/Citrix pixel-only. Reported RDP: 40 false stops in 105 runs.

### Vision/hybrid agent frameworks
- **UFO2 / Windows AgentOS** — https://github.com/microsoft/UFO — MIT — HostAgent + per-app AppAgents (FSMs), UIA + OmniParser grounding, "Native APIs are preferred" (xlwings, win32com, python-pptx), GUI fallback; commands "dispatched through MCP servers"; knowledge substrate (help docs, demos, traces); speculative multi-action (51% fewer LLM calls). Extension: per-app help docs, demonstrations, native-API wrappers, `AppUIExecutor` MCP server. PiP mode is **roadmap only** on the docs index despite the arXiv paper describing it. Safety/human-in-loop: not described on pages read.
- **Agent S** (simular-ai) — https://github.com/simular-ai/Agent-S — Apache-2.0 — `pip install gui-agents` (S3 runtime), S4 announced 2026-10-20; mac/Win/Linux; screenshot in, click/type/scroll out (SECONDARY: pyautogui); recommended grounding model UI-TARS-1.5-7B; local Python/Bash code execution; single-monitor. Simular API (2026-10-01) is the hosted "always-on computer agents" product.
- **UI-TARS Desktop / Agent TARS** (ByteDance) — https://github.com/bytedance/UI-TARS-desktop — Apache-2.0 — VLM-driven local/remote computer and browser operators; Agent TARS kernel "built on MCP and can mount MCP servers"; ~39.5k stars. Model weights licensed separately (UNKNOWN).
- **c/ua** (trycua) — https://github.com/trycua/cua — MIT core; FSL-1.1-MIT for Cua Spaces/Bench (converts to MIT after 2 years); AGPL-3.0 for `cua-som`/`cua-perception` — Lume macOS/Linux VMs on Apple Silicon, gVisor sandboxes, Cua Driver (CLI/MCP/SDK) for native apps with background delivery "where the platform supports it"; ~31.7k stars; several packages deprecated; README warns `cuabot` npm package has known security issues.
- **nut.js** — https://github.com/nut-tree/nut.js — core reported Apache-2.0 (nutjs.dev llms.txt, SECONDARY); README does not name license — mouse/keyboard/screen/window; public npm packages removed, prebuilt binaries subscription-only; X11 only on Linux.

### Model-vendor computer use
- **Anthropic computer toolset** — https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool — reference impl https://github.com/anthropics/anthropic-quickstarts/tree/main/computer-use-demo (Xvfb/Mutter container).
- **OpenAI computer tool** — https://developers.openai.com/api/docs/guides/tools-computer-use — sample https://github.com/openai/openai-cua-sample-app (Playwright + PyAutoGUI).
- **Gemini computer_use** — https://aistudio.google.com/docs/computer-use — browser/mobile/desktop; powers Project Mariner, Firebase Testing Agent, AI Mode (Google blog 2025-10-07).
- **Claude Desktop computer use (Cowork/Claude Code)** — https://support.claude.com/en/articles/14128542 — per-app permission prompts, blocked categories (SECONDARY).

### OS vendors
- **Windows ODR / MCP on Windows** — https://learn.microsoft.com/en-us/windows/ai/mcp/overview — prerelease.
- **Windows App Actions** — https://learn.microsoft.com/en-us/windows/ai/app-actions/ .
- **Apple App Intents** — https://developer.apple.com/documentation/appintents/apple-intelligence-and-siri-ai (body not retrievable); WWDC26 session 343.
- **Android AppFunctions** — https://developer.android.com/reference/android/app/appfunctions/package-summary — API 36, system-privileged agents only.

### RPA vendors
- **Microsoft Copilot Studio computer use** — https://learn.microsoft.com/en-us/microsoft-copilot-studio/computer-use (page dated 2026-10-20) — OBSERVED: CUA model (OpenAI CUA, GPT-6 Sol, Claude Sonnet 4.5/4.6, Opus 4.6) with virtual mouse/keyboard on a Power Automate-registered Windows machine; access-control allowlist of URLs/process names ("only prevents the model from taking actions on websites or applications that aren't in the allow list. It doesn't stop the model from opening them"); stored credentials (Key Vault); human-supervision email escalation on suspected harmful instructions; 5 or 15 Copilot Credits per step; credential-fill unsupported for "Electron, Java, Unity, games, command-line interfaces, Citrix."
- **UiPath** — docs.uipath.com/agents — SECONDARY only: "Computer Use is a UI agent class that operates software visually ... does not depend on brittle selectors"; Autopilot/ScreenPlay/Maestro. Primary doc page for screen agents: UNKNOWN (404 on guessed URL; search budget exhausted).
- **Automation Anywhere** — https://www.automationanywhere.com/products — SECONDARY: Process Reasoning Engine, AI Agent Studio, "EnterpriseClaw" local-agent governance; computer-use capability: UNKNOWN.

---

## 5. Strengths of the existing ecosystem

- Model-side action vocabularies have converged (click/type/key/scroll/screenshot/wait/drag) across Anthropic, OpenAI, Gemini; an executor can be model-agnostic with thin adapters (c/ua, Copilot Studio already do this).
- Accessibility-tree servers prove that deterministic, token-cheap control is feasible on every desktop OS and in browsers; Windows-MCP, Peekaboo and Playwright MCP each return element IDs the model can act on without vision.
- The OS vendors are converging on **opt-in typed actions + contained execution + per-agent identity** (ODR containment, agent workspace accounts, AppFunctions system-agents). This is the direction policy will push.
- Vendor safety layers (Gemini `safety_decision` categories, Anthropic screenshot classifiers, OpenAI pending safety checks, Copilot Studio human-supervision) provide reusable taxonomies for "consequential action" gating.
- Sandboxing substrates exist (c/ua Lume/gVisor, Anthropic reference container, Windows agent workspace) so a control plane does not need to invent isolation.
- Recording-to-deterministic-program (OpenAdapt, terminator recorder) shows a path from one-off agent exploration to repeatable, model-free automation with read-back verification.

## 6. Failure modes (observed across the prior art)

- **Permission granularity collapses to the host process**: macOS Automation/Accessibility grants attach to the launching app (Terminal, Claude Desktop), so every MCP server launched by that host inherits them; Windows ODR grants file access per *host*, not per server (OBSERVED). A control plane cannot express "this capability may only touch app X."
- **Containment excludes the live desktop**: Windows contained servers cannot see "apps and windows the user is using"; the only sanctioned path to the user's session is reduced protections or the separate agent workspace. Any a11y/vision driver of the user's own windows is, by Microsoft's model, "unprotected."
- **"Not a security boundary" everywhere**: Windows-MCP takeover, Playwright origin lists, Desktop Commander directory limits, Copilot Studio allowlist (model can still *open* disallowed apps). Nobody enforces at the OS layer; policies are advisory.
- **Prompt injection via screen content** is acknowledged by all three model vendors; mitigations are classifiers (Anthropic), opt-in scanners (Gemini), human callbacks (OpenAI), none deterministic.
- **Vision cost and drift**: 1,000-1,800 tokens/screenshot; >20 images tightens limits (Anthropic). Benchmarks are not comparable across OSWorld variants and "a large share of OSWorld tasks can be solved without touching the GUI" (SECONDARY).
- **a11y tree gaps**: custom-drawn controls (games, Electron canvases, Java/Unity) are invisible to UIA/AX; Windows-MCP can't select intra-paragraph text; Copilot Studio can't fill credentials into Electron/Java/Unity/CLI/Citrix. UFO2 needs OmniParser to cover them.
- **Wayland/Linux fragmentation**: compositor-specific window focus APIs, ydotool for input, manual validation on one distro only.
- **Telemetry and supply chain**: Windows-MCP and Desktop Commander ship telemetry on by default; most servers are `npx -y`/`uvx` installs with no signature; c/ua itself warns about a compromised npm package in its own ecosystem.
- **License drift**: screenpipe moved from MIT to a commercial source-available license; nut.js removed public npm binaries; c/ua mixes MIT/FSL/AGPL. A control plane that bridges these must track license per adapter.
- **No shared evidence model**: each server returns ad-hoc text/JSON; none retains a before/after state record with provenance that can be compared, exported, or used to confirm a write (OpenAdapt's read-back is the exception, and it is per-workflow, not per-tool).

## 7. Trade-offs vs other technique families

- vs **pure accessibility-tree automation**: prior art shows a11y is the cheapest reliable layer but fails on custom-drawn UI; the universal layer must declare per-target which layer produced an observation (UFO2's hybrid is the closest).
- vs **pure computer-use/vision**: vendor tools are model-first and executor-agnostic; they solve "what to click" not "how to know it worked." Vision is the fallback, not the base.
- vs **OS scripting (AppleScript/COM/PowerShell)**: highest fidelity where available, zero where not; UFO2's "native API preferred, GUI fallback" ordering is the evidence-backed default.
- vs **dynamic instrumentation (Frida)**: absent from every surveyed control plane. No prior-art agent framework instruments the target process; REA's `frida-instrumentation.md` lane is unique in this comparison set. This is both an opportunity and the boundary area (see Section 9).
- vs **app-declared actions (App Actions/App Intents/AppFunctions)**: only path with OS-grade permissions and identity, but zero coverage for the "binaries with no public API" case that motivates agentic-anything. Should be a *first-choice adapter when present*, never the floor.
- vs **recorded/replay automation (OpenAdapt)**: deterministic and model-free at runtime, with read-back verification, but requires a system-of-record to confirm writes and fails on tasks that vary.

## 8. Integration notes for an MCP control plane (what REA-style discipline would add)

Observed in REA (`rea-src`):
- `src/contracts/toolEffects.ts:1-54` declares per-tool `ToolEffects` (`mutatesTarget, mutatesSession, writesFilesystem, launchesProcess, accessesNetwork, changesUiState, mayDiscardData, executesForeignCode, mutatesDevice, idempotent`) and derives MCP annotations (`readOnlyHint, destructiveHint, idempotentHint, openWorldHint`). No surveyed desktop MCP server declares anything comparable; Windows-MCP, Peekaboo, terminator expose tools with free-text descriptions only.
- `docs/mcp-contracts.md:14-38`: every evidence-producing tool returns a canonical Evidence record with `evidence_id`, `normalized_result`, `raw_result`, `analysis_profile`; retained records can be passed by reference (`{"kind":"retained-evidence"}`) to comparison tools; unresolvable references fail with `evidence_integrity_mismatch`.
- `docs/capability-composition.md:1-60`: capability declarations (`src/composition/capabilityDeclarations.ts`) separate static provider identity/provenance from runtime availability probes; "host availability is an observation."
- `docs/browser-scenario-contract.md:1-40,116-171`: declarative action vocabulary, explicit origin, deterministic environment, final DOM/accessibility/URL capture; service-worker policy violations reject the capture but retain artifacts without "policy-compliant completion."
- `docs/agent-environments.md:184`: write effects carry `unconfirmed` / `incomplete-entities` states until a later read confirms them.
- Gap in REA itself (from salvaged notes and this read): no per-step interactive desktop driving with state carried across calls; UI work is batch scenarios (`capture_browser_scenario`, `trace_native_ui_action` at `docs/mcp-contracts.md:894`).

Integration shape suggested by the prior art plus REA:
1. **Process model**: one local MCP server (stdio for local clients; optional HTTP with auth as Windows-MCP does) that supervises per-platform *driver* subprocesses (UIA/AX/AT-SPI, screenshot, scripting bridge, optional Frida). On Windows, ship two packagings: an MSIX-identity connector for ODR (read/typed actions only, contained) and an unpackaged "session driver" that is explicitly labelled as running in the user session.
2. **Capability registry**: per target app, a declaration listing which layers are available (declared actions > scripting/IPC > a11y > instrumentation > vision), each with provenance and `ToolEffects`-style flags; runtime probes decide availability (REA's static-vs-observed split). UFO2's AppAgent/knowledge substrate and Windows App Actions JSON are the nearest prior art for the per-app descriptor.
3. **Evidence per action**: every drive call returns before/after observation (a11y snapshot diff like `mcp-server-macos-use`'s `showDiff`, plus optional screenshot digest) with an `evidence_id`; writes are `unconfirmed` until a read-back confirms (OpenAdapt/REA agent-environments pattern). Model-vendor safety taxonomies (Gemini categories) can be mapped onto `mayDiscardData`/`accessesNetwork`-style effects to decide when to require confirmation.
4. **Permission prompts**: because OS grants attach to the host, the control plane must implement its own per-target policy (allowlist of process names/bundle IDs/origins, like Copilot Studio and Playwright MCP) and must tell the agent that it is advisory. Confirmation stays client-side (MCP elicitation) unless the server is the host.
5. **Latency/cost**: a11y snapshots are cheap; screenshots cost ~1-2k tokens and should be opt-in per call (Windows-MCP, Playwright MCP precedent); speculative multi-action batches (UFO2, Anthropic ordered blocks) require "stop at first failure" semantics.
6. **State**: a session object holding focused app, retained evidence bundle, and takeover state (user moved mouse → pause) as Windows-MCP does; export bundle for audit (REA `export_evidence_bundle`).

## 9. Authorization & legal notes

- Scope boundary: every surveyed tool is positioned for software the user runs; none is marketed for DRM/licensing defeat. The one technique with elevated boundary risk is **dynamic instrumentation** (Frida), present only in REA; adding it to a control plane requires making `executesForeignCode`/`mutatesTarget` explicit and defaulting it off, and documenting that attaching to third-party processes may violate those products' terms even when technically local.
- Anthropic requires that you "Inform end users of relevant risks and obtain their consent prior to enabling computer use in your own products" (OBSERVED). Gemini and OpenAI docs carry similar supervision language.
- Windows "Reduce protections for agent connectors" is a user-visible security downgrade; a product that instructs users to enable it inherits that risk and should say so.
- macOS Accessibility/Screen Recording permissions are granted to the host app; Peekaboo's explicit foreground-consent policy is a model for not stealing focus.
- Licenses to track: screenpipe (commercial, non-OSI), nut.js binaries (subscription), c/ua components under FSL-1.1-MIT and AGPL-3.0, UI-TARS model weights (UNKNOWN), several registry servers with no stated license. Bridging commercial RPA products (UiPath/AA/Copilot Studio) means per-seat/credit licensing and their platform terms.
- Telemetry: Windows-MCP and Desktop Commander default to on; a control plane wrapping them should disable or disclose.
- Android AppFunctions are legally/technically restricted to system-privileged agents; third-party use requires accessibility-service or ADB paths, each with their own Play policy constraints (UNKNOWN details).

## 10. Open questions

- Does Windows ODR expose any sanctioned way for a contained connector to observe or drive windows in the *user* session, or is the agent workspace the only route? (Docs read say no; prerelease.)
- Has Apple shipped public MCP access to App Intents for third-party agents (reported in macOS 26.1 betas, Sept 2025)? Not confirmed on any Apple page read.
- Current status of Windows agent workspace / Copilot Actions beyond Insider preview (last primary date Nov 2025).
- UiPath's primary documentation for Computer Use / Screen Agents and Automation Anywhere's computer-use capability (search budget exhausted before locating primary pages).
- Does OpenAI still document the `pending_safety_checks` categories (`malicious_instructions`, `irrelevant_domain`, `sensitive_domain`) for the current `computer` tool? Not on the current page.
- Licenses of mcp-server-macos-use, computer-use-linux, linux-desktop-mcp, desktop-touch-mcp, DesktopManager (not stated on pages read).
- Whether UFO2's Picture-in-Picture mode has shipped (paper says yes; docs roadmap says planned).
- Whether UI-TARS Desktop itself (vs Agent TARS) exposes an MCP surface, and the license of UI-TARS model weights.
- Whether any surveyed server implements MCP elicitation for per-action confirmation (Agent360dk/computerMCP claims "writes need consent"; not verified).
- Whether REA intends in-server approval/policy hooks or per-step interactive UI driving (salvaged open questions; still open).

## 11. Sources

Primary (OBSERVED):
- https://learn.microsoft.com/en-us/windows/ai/mcp/overview (ms.date 2025-12-06, updated 2026-06-04)
- https://learn.microsoft.com/en-us/windows/ai/mcp/servers/mcp-server-overview (2025-08-12)
- https://learn.microsoft.com/en-us/windows/ai/mcp/servers/mcp-containment (2025-11-11)
- https://learn.microsoft.com/en-us/windows/ai/app-actions/ (2026-10-14)
- https://learn.microsoft.com/en-us/microsoft-copilot-studio/computer-use (2026-10-20)
- https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool
- https://developers.openai.com/api/docs/guides/tools-computer-use
- https://aistudio.google.com/docs/computer-use
- https://blog.google/technology/google-deepmind/gemini-computer-use-model/ (2025-10-07)
- https://developer.android.com/reference/android/app/appfunctions/package-summary
- https://microsoft.github.io/UFO/ufo2/overview and https://microsoft.github.io/UFO/
- https://github.com/CursorTouch/Windows-MCP
- https://github.com/simular-ai/Agent-S
- https://github.com/bytedance/UI-TARS-desktop
- https://github.com/microsoft/playwright-mcp
- https://github.com/browser-use/browser-use
- https://github.com/browserbase/stagehand
- https://github.com/screenpipe/screenpipe
- https://github.com/mediar-ai/mcp-server-macos-use
- https://github.com/mediar-ai/terminator
- https://github.com/steipete/Peekaboo
- https://github.com/openclaw/AXorcist
- https://github.com/OpenAdaptAI/OpenAdapt
- https://github.com/trycua/cua
- https://github.com/nut-tree/nut.js
- https://github.com/wonderwhy-er/DesktopCommanderMCP
- https://registry.modelcontextprotocol.io/v0/servers?search=desktop and ?search=computer
- https://wwdcnotes.com/documentation/wwdc26-343-explore-advanced-app-intents-features-for-siri-and-apple-intelligence/
- REA checkout: `docs/mcp-contracts.md`, `docs/capability-composition.md`, `docs/browser-scenario-contract.md`, `docs/agent-environments.md`, `docs/desktop-applications.md`, `src/contracts/toolEffects.ts`

Secondary (search summaries / third-party):
- https://www.windowscentral.com/software-apps/windows-11/windows-11-model-context-protocol-mcp-ai-announcement-build-2025
- https://support.microsoft.com/en-us/windows/ai/ai-features/experimental-agentic-features
- https://www.privacyguides.org/news/2025/11/19/windows-announces-experimental-agentic-features-admits-potential-for-prompt-injection-and-malware/
- https://support.claude.com/en/articles/14128542-let-claude-use-your-computer
- https://the-decoder.com/claude-code-and-cowork-now-let-anthropics-ai-take-control-of-your-mac-or-windows-desktop/
- https://9to5mac.com/2025/09/22/macos-tahoe-26-1-beta-1-mcp-integration/
- https://appleinsider.com/articles/25/09/22/ios-26-could-get-a-major-ai-boost-with-the-model-context-protocol
- https://engadget.com/2170770/gemini-intelligence-brings-app-automation-to-android
- https://github.com/agent-sh/computer-use-linux ; https://pypi.org/project/linux-desktop-mcp/
- https://github.com/cioinside/da-mcp ; https://pypi.org/project/computer-mcp/
- https://github.com/steipete/macos-automator-mcp
- https://nutjs.dev/llms.txt
- https://arxiv.org/abs/2504.14603 (UFO2 paper)
- https://adoption.microsoft.com/files/agents/AgenticAutomationAdoptionGuide.pdf
- https://www.automationanywhere.com/products ; https://docs.automationanywhere.com/bundle/enterprise-v2019/page/pre-and-genai.html
- https://benchlm.ai/md/benchmarks/osworld-verified.md ; https://www.buildmvpfast.com/blog/osworld-benchmark-ai-agents-passed-human-baseline-2026
- https://venturebeat.com/ai/openai-unveils-chatgpt-agent-that-gives-chatgpt-its-own-computer-to-autonomously-use-your-email-and-web-apps-download-and-create-files-for-you
