# Survey: Screenshot / vision-driven "computer use" agents as a universal fallback control interface

Technique family: `computer-use-vision-agents`
Research date: 2026-11-02..04 (web facts labeled with the date observed on the source page; many leaderboards change weekly).
Legend: **OBSERVED** = read on a page / in source. **INFER** = my inference. **UNKNOWN** = could not verify.

Note on model names: vendor doc pages fetched during this survey reference model generations (e.g. "Claude Fable 5.1", "Claude Opus 4.8", "GPT-6.1 Sol", "Gemini 4 Argon") that I could not cross-verify against a second primary source. They are reported exactly as the pages showed them; treat vendor-version specifics as time-sensitive.

---

## 1. Summary

- A vision computer-use agent (CUA) loop is: capture screenshot -> model emits a primitive action (click at x,y / type / key / scroll / drag / zoom) -> harness executes via OS input injection -> repeat. It needs **no cooperation from the target app**, which is why it is the universal fallback for apps with no API, no accessibility tree, no scripting surface (games, Electron apps with broken AX, Citrix/RDP sessions, custom-drawn canvases).
- All three big vendors ship a first-party tool: Anthropic (`computer_toolset_20260801`, GA, no beta header; earlier `computer_2025xxxx` betas), OpenAI (Responses API `computer` tool; `computer-use-preview` snapshot retired 2026-07-23), Google (Gemini `computer_use` tool, browser/mobile/desktop environments). Open-source model/agent stacks (Agent S3/S4, UI-TARS-2, OpenCUA, Holo3, UI-Mate...) and parsers (OmniParser) are mature enough to run locally.
- Maturity (OSWorld-Verified, 369 Ubuntu tasks, human baseline 72.36%): best general models are reported at ~83-86% (aggregator snapshot 2026-10-29) and an agentic framework at 90.19% (2026-07-25). On the harder OSWorld 2.0 (108 long-horizon tasks, 06/2026) author-run binary completion is still ~13-21% and vendor self-reports top out ~42% binary / ~78% partial (page updated 2026-09-30). So: short tasks in a VM are solved-ish; multi-hour workflows are not.
- Cost/latency are the dominant engineering constraints: ~1,000-1,800 input tokens per screenshot, 2-5+ s per step including screenshot settle delay, hundreds of steps for long tasks (OSWorld 2.0: ~318 tool calls/task; author-run cost $1.5k-3.9k per 108-task benchmark run for frontier models).
- Safety: on-screen prompt injection is the characteristic attack (pop-up attack 86% ASR on undefended agents, 2024/25; VPI-Bench up to 51% CUA / 100% BUA, ICLR 2026; Anthropic reports 1.4-2.4% with improved safeguards under best-of-N attacker, 2025-11-24). Every vendor requires: sandbox/VM, least privilege, domain allowlists, human confirmation for consequential actions.
- For the agentic-anything control plane: vision belongs at the **bottom** of a capability ladder (API/CLI -> a11y/UIA/DOM -> vision), invoked only when structured interfaces fail, with the structured layer still used to *verify* what vision did. Evidence: UFO2 (Windows) shows hybrid UIA+vision detection beats either alone and GUI+API actions cut steps up to 58%.

---

## 2. What it gives an agent

- **Observation**: raw pixels of a display/window (optionally with zoom-in crops). Works on anything that renders: native apps, remote desktops, video, canvas, games, legacy toolkits with no AX tree. (OBSERVED: all three vendor docs; OpenAdapt supports "RDP and Citrix sessions".)
- **Actuation**: OS-level synthetic input (mouse move/click/drag/scroll, keyboard, hold key). Anthropic reference impl uses `xdotool` on Xvfb (OBSERVED in `computer_use_demo/tools/computer.py`: `mousemove --sync`, `click N`, `type --delay 12`, `key --`, scroll via buttons 4-7).
- **Grounding**: the model maps a semantic target ("the Save button") to coordinates. Three strategies:
  1. *Native coordinate prediction* by the VLM (Anthropic: pixel coords in the returned screenshot frame; Gemini: 0-999 normalized grid rescaled by client; OpenAI: pixel coords).
  2. *Parser-assisted*: OmniParser (YOLO icon detector + Florence-2 captions) turns the screenshot into an element list with boxes; model picks an element id (Set-of-Mark).
  3. *Tree-assisted*: a11y/UIA/DOM tree supplies ids+bounds; SoM overlay aligns the tree to the image (OSWorld, WindowsAgentArena, Spider2-V, ScienceBoard all do this).
- **Verification**: screenshot-after-action is the only feedback channel in pure-vision mode (Anthropic prompt tip: "take a screenshot after each step"). This is also the weakest link (see hallucinated completion).
- **Teach/replay**: record a human demonstration once, then replay (OpenAdapt "compiles" demos into a local program; Anthropic best-practices blog "Teach Mode recordings store both CSS selectors ... and coordinates as a visual fallback").

---

## 3. Platforms & permission models

| Platform | How the harness captures / injects | Permission gates | Notes |
|---|---|---|---|
| Linux X11 (reference impls) | `gnome-screenshot`/`scrot` + `xdotool` inside Xvfb; VNC/noVNC for human view | none inside a container; the container *is* the sandbox | Anthropic quickstart Docker image (`ghcr.io/anthropics/anthropic-quickstarts:computer-use-demo-latest`), OSWorld uses Ubuntu VMs with `pyatspi` for a11y |
| Linux Wayland | UNKNOWN in surveyed sources; INFER: portal-based screenshot (xdg-desktop-portal) and `ydotool`/libei needed; most stacks pin X11 | portal consent dialogs | Not covered by any surveyed vendor doc |
| Windows | UIA tree + screenshot (UFO2, WindowsAgentArena, OmniTool on Win11 VM); input via pywinauto / SendInput | UAC for elevated windows; no OS prompt for screen capture | UFO2 "picture-in-picture" uses RDP loopback virtual session so agent and user share the box without fighting over input (OBSERVED, arXiv 2504.14603) |
| macOS | Screen Recording + Accessibility TCC permissions required for capture and synthetic input; Retina 2x DPR must be handled | TCC prompts, per-app | Anthropic docs: "macOS Retina displays capture at a 2x device pixel ratio, so downscale by 2x or halve the coordinates" (OBSERVED). trycua/cua provides local macOS VMs (secondary source, 2026-09) |
| Browser-only | Playwright/CDP screenshot + input; OpenAI examples use 1024x768 viewport; Gemini "browser" environment adds navigate/go_back | browser profile isolation | Anthropic has a separate browser-use tool; Google Mariner; hosted sandboxes Browserbase/Browser Use/E2B/Daytona (OBSERVED OpenAI docs) |
| Android | Gemini "mobile" environment (open_app, long_press...); AndroidWorld benchmark | ADB | Gemini eval PDF shows Claude's tool adapted to Android via custom functions (OBSERVED) |

Permission model of the *model APIs* themselves (OBSERVED):
- Anthropic: toolset is client-side; all screenshots stored in your environment; ZDR/HIPAA eligible. Server-side prompt-injection classifiers scan tool outputs incl. screenshots and "steer the model to verify instructions with the user"; opt-out via support.
- OpenAI: `pending_safety_checks` (e.g. code `malicious_instructions`) halt the sample; client must echo `acknowledged_safety_checks` to continue; frameworks (MS Agent Framework, TanStack) never auto-acknowledge. Docs: "Page content can influence the model, so you should use isolated profiles and limit network access."
- Google: `safety_decision` with `decision: require_confirmation` + `explanation`; client sets `safety_acknowledgement` in function result after user approval; "Overrides are preferences only, so the model may still return require_confirmation." Policy categories: FINANCIAL_TRANSACTIONS, SENSITIVE_DATA_MODIFICATION, COMMUNICATION_TOOL, ACCOUNT_CREATION, DATA_MODIFICATION, USER_CONSENT_MANAGEMENT, LEGAL_TERMS_AND_AGREEMENTS. Prompt-injection detection is opt-in (default false).

---

## 4. Representative tools

Vendor APIs
- **Anthropic computer use tool** - https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool - `computer_toolset_20260801` (GA, 17 member tools, zoom on by default), `computer_20251124` beta (adds `zoom`, `enable_zoom`), `computer_20250124` beta (adds scroll/hold_key/wait/triple_click/mouse down-up), `computer_20241022` (original). Proprietary API; reference impl is open source.
- **anthropic-quickstarts/computer-use-demo** - https://github.com/anthropics/anthropic-quickstarts/tree/main/computer-use-demo - Docker Ubuntu + Xvfb + Mutter + Tint2 + noVNC + Streamlit; `computer.py`, `bash.py`, `edit.py`, `loop.py`. License: repo has LICENSE file; type not shown on fetched page (UNKNOWN, INFER MIT).
- **OpenAI computer tool (Responses API)** - https://developers.openai.com/api/docs/guides/tools-computer-use - actions click/double_click/drag/move/scroll/keypress/type/wait/screenshot; `computer_call_output` with `computer_screenshot` + `detail: "original"`. `computer-use-preview` (2025-03-11 snapshot) retired 2026-07-23, replacement listed as `gpt-5.6-terra` (OBSERVED on deprecations page). Standalone Operator product folded into ChatGPT Agent mode ~Aug 2025 (secondary sources).
- **Gemini computer use** - https://aistudio.google.com/docs/computer-use (redirect from ai.google.dev) - environments browser/mobile/desktop; coords 0-999 normalized; `safety_decision`, `excluded_predefined_functions`, custom functions (`yield_to_user` example). Launched as Gemini 2.5 Computer Use 2025-10-07 ("not yet optimized for desktop OS-level control" at launch); powers Project Mariner, Firebase Testing Agent, AI Mode.

Open-source agents / models
- **Agent S / S2 / S3 / S4 (Simular)** - https://github.com/simular-ai/Agent-S - Apache-2.0; macOS/Windows/Linux; S3 = simplified agent + code actions + Behavior Best-of-N (OSWorld 69.9% 2025-10-02 -> 72.6% 2025-12-15, "above human"); S4 released 2026-10-20 (OSWorld 2.0 21.3% with Qwen3.8-27B). Hosted "Sai" agent 73% partial on OSWorld 2.0 at $15.70/task (self-reported, co-author).
- **UI-TARS / UI-TARS-2 (ByteDance)** - https://arxiv.org/abs/2509.02544 - native end-to-end GUI model trained with multi-turn RL; OSWorld 47.5, WAA 50.6, AndroidWorld 73.3, Online-Mind2Web 88.2 (paper, 2025-09). Weights release status for -2 not stated on arXiv page (UI-TARS-1.5 weights were public; license for 1.x Apache-2.0 per earlier releases - not re-verified here).
- **OpenCUA (xlang)** - https://opencua.xlang.ai/ - NeurIPS 2025; open data+tools+models 7B/32B/72B; 45.0% OSWorld-Verified (2025); vLLM support 2026-01. License: UNKNOWN from fetched pages.
- **OmniParser V2 (Microsoft)** - https://github.com/microsoft/OmniParser - screenshot -> structured element list; ~0.6 s/frame A100, 0.8 s RTX 4090 (model card); ScreenSpot-Pro 39.5-39.6; OmniTool drives a Windows 11 VM with any VLM. Licenses: code MIT (README badge; sidebar shows CC-BY-4.0 - unreconciled), caption models MIT, older Ultralytics-based detectors **AGPL**, new `icon_detect_v3` on MIT YOLOv9-E (2026-07).
- **UFO2 (Microsoft)** - https://github.com/microsoft/UFO - Windows "AgentOS": HostAgent + per-app AppAgents, hybrid UIA + OmniParser control detection, unified GUI+API action layer, speculative multi-action, picture-in-picture desktop. License MIT (repo; not re-verified in this pass).
- **OpenAdapt** - https://github.com/OpenAdaptAI/OpenAdapt - MIT; record-once/replay; current README describes compiling demos into a local program using layers "page structure, then a direct API call where one is bound, then pixel matching ... only where nothing else exists"; Windows UIA / macOS AX / Linux AT-SPI; separate `openadapt-privacy` PII/PHI scrubber; v1.17.0, benchmark runs dated Oct 2026.
- **ScreenAgent** - https://github.com/niuzaisheng/ScreenAgent - IJCAI 2024; VNC-driven plan/act/reflect loop; CogAgent-based model; code MIT, data Apache-2.0, model CogVLM license. Last visible activity May 2024 (historical interest).
- **Open Interpreter `--os` mode** - https://github.com/OpenInterpreter/open-interpreter - "Computer API" for GUI control; 2026 releases (0.0.54-0.0.59, Sep-Nov 2026) do not mention OS mode; status of OS mode: UNKNOWN / likely dormant (INFER).
- **trycua/cua** - https://github.com/trycua/cua - sandboxed local macOS/Linux VMs + agent SDK + benchmarks (secondary sources; license not verified).
- **Microsoft Agent Framework / TanStack AI computer-use** - wrap OpenAI/Anthropic loops with explicit human-approval callbacks for safety checks.

Benchmarks
- **OSWorld / OSWorld-Verified** - https://os-world.github.io/ (-> osworld-v1.xlang.ai) - 369 tasks (361 excl. Google Drive); Verified release 2025-07-28 fixed tasks, AWS parallel eval "within 1 hour"; human 72.36%; observation tabs: Screenshot / A11y tree / Screenshot+A11y / Set-of-Mark.
- **OSWorld 2.0** - https://arxiv.org/abs/2606.29537, osworld-v2.xlang.ai - 108 tasks, median human 1.6 h, ~318 tool calls, 31 self-hosted sites, ~27 weighted checkpoints/task; binary + partial metrics; "not comparable to OSWorld 1.0/Verified".
- **WindowsAgentArena** - https://microsoft.github.io/WindowsAgentArena - 154 Windows tasks; human 74.5%; Navi 19.5% (2024); UIA tree beats pixel-only OCR/icon detection; Azure parallel run ~20 min.
- **ScreenSpot-Pro** - https://arxiv.org/abs/2504.07981 - 1,581 high-res professional-app grounding instructions; official leaderboard gui-agent.github.io/grounding-leaderboard; aggregators disagree on leader (72.7% vs 86.3% vs vendor claim 82.7% 2026-09) - UNRESOLVED.
- **VPI-Bench** (security) - https://arxiv.org/abs/2506.02456 - 306 visual-prompt-injection cases, 5 platforms, ICLR 2026.

---

## 5. Strengths

- Universal coverage: works where a11y/DOM/API do not exist or are broken (custom-drawn UI, remote desktop, legacy Win32 without UIA, games). No instrumentation of the target, no injection into its process, no reverse engineering -> fits the "software you own" boundary cleanly (INFER).
- Zero per-app integration cost; one harness covers every app. Vendor tools are now GA (Anthropic toolset, OpenAI GA per secondary source 2026-09, Gemini preview).
- Rapid capability growth: OSWorld-Verified SOTA moved from ~38% (2024) -> 61-66% (Sonnet/Opus 4.5, late 2025) -> 72.7% (Opus 4.6) -> 83-86% general models (Oct 2026 aggregators). Above the 72.36% human baseline on short tasks.
- Robust to layout drift compared with brittle selectors/coordinates: the model re-grounds every step.
- Natural human-in-the-loop story: the human watches the same pixels (noVNC / PiP) and can take over.
- Mature safety plumbing exists in all three vendor APIs (confirmation signals, injection classifiers) and in open frameworks (never-auto-acknowledge patterns).
- Record/replay hybrids (OpenAdapt, Teach Mode) can amortize model cost to ~zero on healthy runs.

---

## 6. Failure modes

Grounding / perception
- **Coordinate errors**: Anthropic docs list "Claude might make mistakes or hallucinate when outputting specific coordinates"; OpenAI docs: "Clicks can miss ... verify outcomes." Precision varies by model (Sonnet 4.6 "more mechanically precise" than Opus 4.6; Opus 4.7 ~ Sonnet 4.6).
- **Small targets after downscaling**: "A 16px checkbox at 3840x2160 becomes roughly 5px at 1280x720" (Anthropic blog 2026-05-13). Mitigation: `zoom` action (crop at physical res, resize into frame), or keyboard/Tab navigation.
- **Resolution budget**: 4.6-family limit 1568 px long edge / 1.15 MP (silently downscaled); Opus 4.7+ 2576 px / 4,784 visual tokens; toolset *rejects* oversize images instead of downscaling -> client must resize and rescale coordinates. Recommended 1024x768 / 1280x720 desktop, 1280x800 / 1366x768 web; avoid >1920x1080.
- **High-DPI / Retina**: screenshot in physical px vs. input in logical points mismatch (macOS 2x DPR; Windows per-monitor DPI). Must pick one frame and convert consistently (OBSERVED Anthropic docs; general DPI bug class in SDL/W3C refs).
- **Multi-monitor**: vendor tools model a single display (`display_number`/`DISPLAY_NUM`); no surveyed vendor doc covers multi-monitor. INFER: capture one monitor, or composite with offset mapping; window-scoped capture (as REA's `observe_native_ui` does by pid+window_id) sidesteps it.
- **Professional / dense UIs**: ScreenSpot-Pro grounding is far below OSWorld task scores (OmniParser V2 39.5%; best models 70-86% depending on source) - CAD, IDEs, DAWs remain hard.

Interaction mechanics
- **Scrolling**: "In applications where scrolling doesn't take effect, keyboard alternatives such as Page Down can help" (Anthropic limitations). Scroll amount semantics differ per app/toolkit.
- **Drag & drop / spreadsheets**: need `left_mouse_down`/`up` fine control; "Complex spreadsheet operations might still require multiple attempts."
- **Native dropdowns / system dialogs** not in the screenshot frame (OS-level menus, file pickers, UAC on secure desktop): blog recommends JS/keyboard/DOM alternatives; UAC secure desktop is uncapturable without elevation (INFER).
- **Timing**: harness sleeps (`_screenshot_delay = 2.0` s) are a heuristic; UI not settled -> stale screenshot -> wrong action. Batch/speculative multi-action increases this risk ("Avoid batching for exploratory navigation or error recovery").
- **Text entry**: xdotool typing in 50-char chunks at 12 ms; non-ASCII/IME needs clipboard path (ScreenAgent uses a clipboard service).

Cognition
- **Hallucinated completion / lost context**: OSWorld 2.0 analysis (Snorkel talk, 2026-09): agents "losing track of information gathered early in the task as trajectories stretch to hundreds of steps." Binary completion remains 13-42% while partial is 50-78% - agents get most checkpoints then stop or misreport.
- **Tool-selection errors**: "reliability might be lower when interacting with niche applications or multiple applications at once."
- **Latency**: Anthropic: "might be too slow compared to regular human-directed computer actions"; Gemini 2.5 launch chart ~225 s per Online-Mind2Web task at 70%+; OpenAI OSWorld 2.0 latency-sim rows ~40-75 min/task.
- **Cost**: ~1,000-1,800 tokens/screenshot; a 200k window fills in <100 screenshots; toolset definition adds ~4,500 tokens/request (410 of it for zoom); OSWorld 2.0 author runs: GPT-5.5 ~$2,750, Opus 4.7 ~$3,870, Sonnet 4.6 ~$2,410 per 108-task run (page updated 2026-09-30).

Security
- **Visual prompt injection**: pop-up attack 86% ASR / -47% task success on undefended OSWorld+VisualWebArena agents; "asking the agent to ignore pop-ups ... ineffective" (ACL 2025). VPI-Bench: up to 51% CUA / 100% BUA; system-prompt defenses "only limited improvements" (ICLR 2026). Anthropic Claude-for-Chrome: 23.6% -> 11.2% (2025-08), then model-only 8.0-13.8% -> improved safeguards 1.4-2.4% under best-of-N attacker (2025-11-24); "No browser agent is immune."
- **Credential exposure**: screenshots contain everything on screen (passwords in plaintext fields, 2FA codes, other windows). Anthropic warns against giving login info; `<robot_credentials>` tag pattern still has injection risk.
- **Over-reach**: an agent with OS input can act on *any* app on the desktop, not just the target -> allowlisting must be enforced by the harness (window focus checks), not by the model (INFER).

---

## 7. Trade-offs vs other technique families

| vs. | Vision CUA wins when | Loses because |
|---|---|---|
| Accessibility / UIA / AX trees | tree is missing, stale, or lies (custom controls, canvas, Electron with AX off, remote desktop) | tree gives exact bounds, roles, states, text at ~zero tokens and ms latency; WAA: "pixel-based OCR and icon detection perform worse than agents that also use the UIA tree"; UFO2 Table 3: OmniParser-only 14.3% vs UIA 22.4% on OSWorld-W, hybrid 28.6% |
| DOM / CDP (browsers) | iframes/canvas/shadow DOM/anti-automation UIs | DOM is semantic, scriptable, queryable; REA already does read-only CDP observation incl. a11y and screenshots |
| App APIs / CLI / IPC / plugin points | none exist | UFO2 Table 5: GUI+API +6-8 pts SR and up to 58.5% fewer steps; a CSV export = 1 API call vs 5 GUI steps |
| Dynamic instrumentation (Frida etc.) | you must not / cannot touch the process; want zero footprint | instrumentation gives ground-truth state and deterministic hooks, but raises authorization and anti-tamper concerns |
| Record/replay RPA (pixel templates) | layout changes often | deterministic replay is free and fast once recorded; OpenAdapt compiles to no-model-call programs |

INFER: the right mental model is a **capability ladder**: each rung is cheaper, faster, more verifiable than the one below; vision is the final rung and also the universal *verifier* (a screenshot can confirm what the a11y tree claims).

---

## 8. Integration notes for an MCP control plane

Process model
- Run the vision harness as a long-lived "display driver" per target display/window: owns screenshot capture, DPI/scale mapping, input injection, and a settle-wait. Expose primitives as MCP tools mirroring the vendor action vocabularies (screenshot, zoom(region), click variants with modifiers, drag, scroll(dir, amount, coord), type, key(repeat), hold_key, wait, cursor_position) so any vendor model loop can be plugged in. Anthropic's toolset shape (one member tool per action, `toolset_name`, batch with halt text `Not executed: an earlier computer action in this turn failed.`) is a good concrete target.
- Prefer **window-scoped** capture+input (pid + window id, like REA `observe_native_ui`, `src/contracts/native/nativeToolContracts.ts:87-88`) over whole-display: smaller images, cheaper tokens, no multi-monitor ambiguity, natural allowlist boundary. Fall back to display capture only for cross-app tasks.
- Keep one coordinate frame: capture at physical px, downscale to the model frame (1280x720 / 1024x768), store the scale factor, map model coords back (reference `scale_coordinates(API,...)` raises on out-of-bounds). Reject/resize oversize images client-side (toolset will 400).
- Isolation: default to a VM/container (Anthropic quickstart Docker, OmniTool Win11 VM, cua macOS VMs) or a separate virtual desktop/session (UFO2 PiP via RDP loopback) so the user's live desktop is not the agent's playground. On the real desktop, require explicit opt-in and show a persistent indicator.

Auth / permission prompts
- Map each vendor's safety signal to a single control-plane event: Anthropic (classifier-steered "verify with user" + your own policy), OpenAI `pending_safety_checks` -> must be surfaced; never auto-acknowledge; Gemini `require_confirmation` -> `safety_acknowledgement`. Add harness-side policy independent of the model: irreversible-action list (send, pay, delete, submit, install, agree-to-terms), app/window allowlist, domain allowlist, rate/step budgets, and a kill switch.
- macOS needs Screen Recording + Accessibility TCC grants to the harness binary; Windows needs elevation only for elevated targets; Linux Wayland needs portal consent (UNKNOWN specifics).

Latency / cost controls
- Settle-wait + screenshot + model round trip ~3-8 s/step (INFER from reference delays and vendor latency caveats). Reduce steps with: batched/speculative actions for self-contained sequences (Anthropic blog; UFO2 up to 51.5% fewer steps), API/keyboard shortcuts over mouse, zoom instead of full-res screenshots.
- Context: rolling screenshot buffer (keep_n=3, prune every 25), stable prefix for prompt caching (cache hit = 10% price), compaction near 150k tokens (Anthropic blog 2026-05-13).
- Cache a parsed element map (OmniParser / a11y) per screenshot so the agent can query "find element with text X" without re-sending pixels (INFER; this is how SoM harnesses work).

State
- Record every step as evidence: action, scaled coords, pre/post screenshot hashes, model rationale (Anthropic: `display: "summarized"` thinking). REA's self-verifying PNG artifact schema (`src/domain/webScreenshot.ts:15-33`, sha256+bytes) and pixel-diff tool (`compareWebScreenshotsInputSchema`) are directly reusable as the evidence substrate.
- Success verification must not rely on the model's claim: re-observe via structured layer (a11y/DOM/file system) or screenshot diff against expected state.
- Distill successful vision trajectories into deterministic replays (OpenAdapt pattern: selectors/API first, pixels last) so repeated tasks leave the model loop.

Composition rule (vision as last resort)
1. Try documented API / CLI / IPC.
2. Try a11y/UIA/AX/DOM: enumerate, act by element reference, verify state.
3. If an element is missing or action has no effect: screenshot the *window*, optionally run OmniParser/SoM, let the VLM ground and act; immediately re-verify with step 2 data.
4. Escalate to full-display vision only for cross-app choreography; require confirmation gates.
5. After success, persist a replay recipe keyed on the structured anchors.

REA comparison (OBSERVED in checkout): REA is observation-only and evidence-first. Browser tools "passively inspect ... without evaluating JavaScript, navigating, clicking" (`src/contracts/browserToolContracts.ts:191`); `observe_native_ui` captures "a bounded accessibility tree and selected-window screenshot" (`nativeToolContracts.ts:88`); `capture_native_ui_scenario` allows explicit click/scroll/key-entry/wait steps addressed by **accessibility path**, not coordinates (`src/domain/native/nativeUiObservation.ts:31-60`). REA has no VLM grounding or coordinate actions; a vision rung would be a new capability layered beneath REA's a11y-path actions, reusing its artifact/evidence schemas.

---

## 9. Authorization & legal notes

- All vendor ToS bind the operator: Anthropic limitations #8 "you must not employ computer use to violate any laws or the Acceptable Use Policy"; "Inform end users of relevant risks and obtain their consent prior to enabling computer use in your own products." Anthropic also notes the model's ability to create accounts / impersonate humans on social platforms is deliberately limited.
- Automating software you own via synthetic input is the same legal posture as RPA/accessibility tooling; it does not circumvent technical protection measures. Flag for the project boundary: vision agents **can** be pointed at DRM-protected video, licensed-seat apps, or CAPTCHAs/bot protections. REA's DISCLAIMER already excludes "bypassing authentication, bot protection or access controls" and "unlawful circumvention of access controls" (`DISCLAIMER.md:15,33`); the control plane should carry the same exclusions and enforce them with domain/app allowlists and refusal on CAPTCHA/anti-bot screens (INFER).
- Website/app terms may prohibit automated access even when screen-scraping is technically trivial; benchmark authors self-host sites for this reason (OSWorld 2.0 hosts 31 sites).
- Privacy: screenshots are full-fidelity captures of the user's desktop, sent to a third-party API. Window scoping, redaction (OpenAdapt `openadapt-privacy`), ZDR agreements (Anthropic computer use is ZDR eligible), and local models (OpenCUA, UI-TARS, Holo3 via vLLM) are the levers. OpenAI docs: "Data may be retained despite `store: false`."
- Licensing of components: OmniParser older detectors are AGPL (copyleft) - relevant if bundled; new YOLOv9-E detector MIT; Agent-S Apache-2.0; OpenAdapt MIT; ScreenAgent model under CogVLM license (non-commercial restrictions historically; not re-verified).
- Consent for consequential actions is a shared expectation across vendors (financial transactions, ToS acceptance, cookie consent, messaging) and should be a hard harness gate, not a prompt instruction.

---

## 10. Open questions

- Wayland capture/injection story for Linux desktops (portal-only screenshots, libei) - no surveyed stack documents it.
- Multi-monitor and mixed-DPI handling: no vendor guidance; needs a harness design decision (per-monitor virtual displays vs. composite).
- Which local VLM gives acceptable grounding on a consumer GPU for a last-resort fallback (OpenCUA-7B/32B, UI-TARS-1.5-7B, Holo3-35B-A3B, UI-Mate-9B) - OSWorld numbers exist, ScreenSpot-Pro leaderboard inconsistent; needs a self-run eval.
- Does a11y-tree-plus-screenshot still beat screenshot-only for frontier models in 2026? Benchmarks shifted to screenshot-only reporting; the UFO2 hybrid data is GPT-4o/o1 era (2025-04). A current ablation is missing.
- OpenAI's full safety-check code list (`irrelevant_domain`, `sensitive_domain`) could not be confirmed on current docs; only `malicious_instructions` observed.
- Gemini coordinate frame: docs show both a 0-999 normalized grid and "pixel coordinates scaled to the screen" - must be tested.
- Reliability of OSWorld-Verified aggregator numbers (Qwen3.8 Max 86.1% vs Intelligence-Indeed 90.19% framework) - official leaderboard is JS-rendered and was not readable; verify before quoting.
- Whether Open Interpreter OS mode and ScreenAgent are maintained (both look dormant).
- How to detect "agent is on the wrong window" cheaply (focus/ownership check before each injected event) - needed for the allowlist guarantee.

---

## 11. Sources

Vendor primary
- Anthropic computer use tool docs (fetched 2026-11-04): https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool
- Anthropic best practices for computer and browser use (2026-05-13): https://claude.com/blog/best-practices-for-computer-and-browser-use-with-claude
- Anthropic prompt injection defenses (2025-11-24): https://www.anthropic.com/research/prompt-injection-defenses
- Anthropic Piloting Claude for Chrome (2025-08-26): https://www.anthropic.com/news/claude-for-chrome
- anthropic-quickstarts computer-use-demo: https://github.com/anthropics/anthropic-quickstarts/tree/main/computer-use-demo ; computer.py: https://raw.githubusercontent.com/anthropics/anthropic-quickstarts/main/computer-use-demo/computer_use_demo/tools/computer.py
- OpenAI computer use guide: https://developers.openai.com/api/docs/guides/tools-computer-use
- OpenAI deprecations (computer-use-preview shutdown 2026-07-23): https://developers.openai.com/docs/deprecations
- OpenAI Operator system card (2025-01): https://openai.com/index/operator-system-card ; o3 CUA addendum: https://cdn.openai.com/pdf/4375e605-f9a6-438d-bcc8-190599c183a6/o3_cua_system_card.pdf
- Azure OpenAI computer use (pending_safety_checks): https://learn.microsoft.com/de-de/azure/ai-services/openai/how-to/computer-use ; MS Agent Framework: https://learn.microsoft.com/en-us/agent-framework/agents/tools/computer-use
- Gemini computer use docs: https://aistudio.google.com/docs/computer-use
- Gemini 2.5 Computer Use launch (2025-10-07): https://blog.google/technology/google-deepmind/gemini-computer-use-model/
- Gemini computer use eval additional info PDF (2025-10-07): https://storage.googleapis.com/deepmind-media/gemini/computer_use_eval_additional_info.pdf

Benchmarks
- OSWorld / OSWorld-Verified: https://os-world.github.io/ (-> http://osworld-v1.xlang.ai/) ; paper https://arxiv.org/pdf/2404.07972
- OSWorld 2.0 paper (2026-06-28, rev 2026-07-13): https://arxiv.org/abs/2606.29537 ; leaderboard snapshot (updated 2026-09-30): https://leaderboard.steel.dev/leaderboards/osworld-2.md ; Snorkel analysis: https://snorkel.ai/blog/osworld-2-0-why-computer-use-agents-fail-most-tasks/
- OSWorld-Verified aggregator snapshots (2026-10): https://benchlm.ai/benchmarks/osworld-verified ; https://presenc.ai/research/osworld-computer-use-leaderboard-2026 ; https://llm-stats.com/benchmarks/osworld-verified
- Windows Agent Arena: https://microsoft.github.io/WindowsAgentArena ; https://arxiv.org/abs/2409.08264
- ScreenSpot-Pro: https://arxiv.org/abs/2504.07981 ; https://github.com/likaixin2000/ScreenSpot-Pro-GUI-Grounding
- Pointer SOTA post (2026-05, superseded): https://www.pointer.ai/blog/sota

Open-source stacks / papers
- Agent S repo: https://github.com/simular-ai/Agent-S ; Agent S3 / Behavior Best-of-N paper: https://arxiv.org/abs/2510.02250
- UI-TARS-2: https://arxiv.org/abs/2509.02544 ; UI-TARS: https://arxiv.org/abs/2501.12326
- OpenCUA: https://arxiv.org/abs/2508.09123 ; https://opencua.xlang.ai/
- OmniParser: https://github.com/microsoft/OmniParser ; https://huggingface.co/microsoft/OmniParser-v2.0
- UFO2 Desktop AgentOS: https://arxiv.org/html/2504.14603v2 ; https://microsoft.github.io/UFO/ufo2/overview
- OpenAdapt: https://github.com/OpenAdaptAI/OpenAdapt ; https://pypi.org/project/openadapt/
- ScreenAgent: https://github.com/niuzaisheng/ScreenAgent
- Open Interpreter releases: https://github.com/OpenInterpreter/open-interpreter/releases
- trycua/cua: https://github.com/trycua/cua
- Spider2-V (SoM + a11y alignment): https://arxiv.org/pdf/2407.10956 ; ScienceBoard: https://arxiv.org/pdf/2505.19897 ; WindowsWorld (raw vs SoM vs hybrid): https://arxiv.org/pdf/2604.27776

Security
- Attacking VLM computer agents via pop-ups (ACL 2025): https://arxiv.org/abs/2411.02391
- VPI-Bench (ICLR 2026): https://arxiv.org/abs/2506.02456
- Simon Willison on Claude for Chrome: https://simonwillison.net/2025/Aug/26/piloting-claude-for-chrome/

Local code read (REA checkout)
- /tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src/src/contracts/native/nativeToolContracts.ts (lines 87-88)
- .../rea-src/src/domain/native/nativeUiObservation.ts (lines 13-60)
- .../rea-src/src/domain/webScreenshot.ts (lines 15-75)
- .../rea-src/src/contracts/browserToolContracts.ts (line 191)
- .../rea-src/DISCLAIMER.md (lines 15, 33)
