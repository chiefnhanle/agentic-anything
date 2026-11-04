# Survey: Accessibility / UI-Automation Trees as an Agent Control Interface

Technique family: **accessibility-trees** (Windows UIA + legacy MSAA/IAccessible2, macOS AXUIElement/NSAccessibility, Linux AT-SPI2, and cross-platform wrappers).
Date of survey: 2026-11-04. Time-sensitive facts carry their observed date. "OBSERVED" = read on a primary page or in source; "INFER" = my inference; "UNKNOWN" = not verified.

Comparison baseline: REA checkout at `/tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src` (read-only; not built or run).

---

## 1. Summary

- Every major desktop OS ships an out-of-process **semantic UI tree** originally built for screen readers. Each node carries a role/control type, a name/label, states (enabled, focused, selected, offscreen...), screen geometry, and a set of *typed actions* (UIA control patterns such as Invoke/Value/Toggle/Scroll; AX actions such as AXPress/AXIncrement; AT-SPI Action/EditableText/Value interfaces). An agent can read this tree without pixels and drive controls without synthesizing mouse input.
- **Platform maturity differs sharply.** Windows UIA is the richest and most stable (COM API, 30+ control patterns, caching, event model, UIAccess security model). macOS AX is complete but needs per-process TCC Accessibility consent and has no stable element identifiers. Linux AT-SPI2 works on X11 and (for reading) on Wayland, but input synthesis on Wayland is unsolved in the a11y stack itself and the successor "Newton" protocol is still experimental (last status post June 2024; AT-SPI2 2.56.x released Aug 2025).
- **Coverage, not API quality, is the limiting factor.** Custom-drawn UIs (games, canvas editors, DirectUI/owner-draw controls, Flutter without UIA build flag), and Chromium/Electron apps that have accessibility *off until an AT is detected* produce empty or shallow trees. Every serious 2025-2026 agent project therefore runs a **hybrid**: tree first, screenshot/vision fallback (Microsoft UFO2, Windows-MCP, Peekaboo, Ghost OS, Terminator, Agent S); Anthropic's own computer-use tool is screenshot-only and does not touch a11y APIs at all (OBSERVED in platform docs).
- For an MCP control plane the tree is the cheapest, most deterministic observation and actuation channel where it exists, and it must be wrapped with: capability probing per app (is the tree populated?), ID minting (RuntimeId/AX path/AT-SPI object path are all unstable across restarts), permission preflight (TCC / UIAccess / a11y bus), and treatment of all tree text as **untrusted** (prompt-injection channel; RedTeamCUA and the Android "Not an A11y" paper quantify it).

---

## 2. What the tree gives an agent

### 2.1 Common data model (all three platforms)
| Concept | Windows UIA | macOS AX | Linux AT-SPI2 |
|---|---|---|---|
| Node type | `ControlType` (Button, Edit, List, Custom...) + `LocalizedControlType`, ARIA role passthrough (`AriaRole`) | `AXRole` + `AXSubrole` + `AXRoleDescription` | `Role` enum (`atspi_accessible_get_role`) |
| Label | `Name`, `HelpText`, `FullDescription`, `LabeledBy` | `AXTitle`, `AXDescription`, `AXHelp`, `AXTitleUIElement` | name, description, `LabelledBy` relation |
| Value | Value / RangeValue / Toggle patterns | `AXValue`, `AXMinValue`/`AXMaxValue` | Value, Text, EditableText interfaces |
| State | `IsEnabled`, `HasKeyboardFocus`, `IsOffscreen`, `IsPassword`, `IsPeripheral`, `IsDialog`... | `AXEnabled`, `AXFocused`, `AXSelected`, `AXHidden`... | `StateSet` bitmap (`StateType`: enabled, focused, showing, visible, sensitive, selected...) |
| Geometry | `BoundingRectangle` (physical screen px), `ClickablePoint`, `CenterPoint` | `AXPosition`, `AXSize`, `AXFrame` | Component interface (extents in screen/window coords) |
| Identity | `RuntimeId` (opaque, session-unique, reusable), `AutomationId` (dev-assigned, sibling-unique, not stable across builds), `NativeWindowHandle`, `ProcessId` | `AXIdentifier` (dev-assigned, often absent); AXUIElementRef equality via CFEqual; no durable ID | D-Bus (bus name, object path) tuple; paths can be reused |
| Actions | Control patterns: Invoke, Toggle, Value.SetValue, RangeValue, ExpandCollapse, SelectionItem.Select, Scroll/ScrollItem, Text/TextRange, Window (close/minimize), Transform (move/resize), Drag/DropTarget, LegacyIAccessible (MSAA passthrough) | `AXUIElementPerformAction` with `AXPress`, `AXIncrement`, `AXDecrement`, `AXConfirm`, `AXCancel`, `AXShowMenu`, `AXPick`, `AXRaise`; `AXUIElementSetAttributeValue` on settable attrs (e.g. `AXValue`, `AXFocused`) | Action interface (`do_action(i)`, named actions), EditableText (`set_text_contents`, insert/delete), Value (`set_current_value`), Selection, plus `atspi_generate_keyboard_event` / `atspi_generate_mouse_event` (X11-era) |
| Events | `IUIAutomationEventHandler` family: focus changed, property changed, structure changed, text changed, notification | `AXObserver` + notifications (`kAXFocusedUIElementChangedNotification`, value/window/layout changes) | `EventListener` on D-Bus signals (`object:state-changed`, `window:activate`...) |
| Hit-test | `ElementFromPoint`, `GetFocusedElement` | `AXUIElementCopyElementAtPosition` on system-wide element | `atspi_component_get_accessible_at_point` |
| Bulk read | `IUIAutomationCacheRequest` + `FindAllBuildCache` (one cross-process call for N properties) | none native; per-attribute IPC (batch via `AXUIElementCopyMultipleAttributeValues`) | `Collection` interface; otherwise per-call D-Bus round trips |

Sources: UIA control patterns overview (ms.date 2025-07-14), UIA property IDs (2025-07-14), UIA tree overview (2025-07-14), libatspi reference, Apple AXUIElement.h (page body not fetchable; details from REA source usage and general API knowledge, marked INFER where not in REA code).

### 2.2 Tree views and scoping (Windows)
- OBSERVED: UIA exposes three filtered views: **raw** (full provider structure), **control** (`IsControlElement=TRUE`: interactive items plus structural containers; "most interesting to automated testing"), **content** (`IsControlElement && IsContentElement`; what a screen reader reads). Caching defaults to the control view. The tree "is seldom seen in its totality because it might contain thousands of elements" and is "built as a client needs them". (learn.microsoft.com uiauto-treeoverview)
- INFER: an agent snapshot tool should default to the control view scoped to one top-level window, which is what Windows-MCP's `Snapshot` and UFO2's `get_ui_tree` effectively do.

### 2.3 Dynamic patterns
- OBSERVED: pattern support is dynamic; a multiline edit exposes `ScrollPattern` only while content overflows (uiauto-controlpatternsoverview). Agents must re-query supported patterns before acting rather than caching "this element is scrollable".

### 2.4 What REA already does with the tree (comparison point)
- OBSERVED `bridge/native/ReaNativeUI.swift` (179 lines): checks `AXIsProcessTrusted()` and fails with code `accessibility-denied` plus remediation text (line 84); creates `AXUIElementCreateApplication(pid)` and sets `AXUIElementSetMessagingTimeout(application, 2)` seconds (lines 85-86); matches the CG window to exactly one AX window by geometry and fails closed on ambiguity (`ambiguous-window`, line 91); addresses elements by **child-index path** re-validated at action time (`element-missing`, line 104) and verifies `AXUIElementGetPid` matches the selected PID (`element-owner-mismatch`, line 108); supports only three actions: `click` -> `kAXPressAction`, `scroll` -> `kAXIncrementAction`/`kAXDecrementAction`, `key-entry` -> `AXUIElementSetAttributeValue(kAXValueAttribute)` (lines 111-113); BFS capture with `max_nodes`, depth cap 32, explicit `truncated` and `gaps` arrays (lines 118-139).
- OBSERVED `docs/native-investigation.md:995-1035`: "No global event injection is used. Unsupported AX actions fail explicitly." Screenshots use a selected-window ScreenCaptureKit filter on macOS 14+; "Missing Screen Recording/Accessibility permissions produce actionable errors without broad capture or automatic permission prompts."
- OBSERVED `docs/mcp-contracts.md:1264-1266`: "macOS may deny Accessibility, Screen Recording... These failures are reported at the operation that needs them."
- OBSERVED REA's browser side (`src/domain/browserObservationSchemas.ts`, `tests/boundary/cli/accessibilityCaptureComparison.test.ts:16-45`) captures the Chromium a11y tree via CDP with fields `node_id, parent_id, role, name, description, ignored, states`, and gates name/description text behind `include_accessibility_text` because it "may contain application secrets" (`docs/browser-observation.md:356,470`).
- INFER: REA is an *observation-first* design (single selected window, index paths, no event injection, truncation made explicit). A general drive layer needs broader actions (Toggle/Select/Scroll patterns, focus, keyboard) and a search/locator model, but should keep REA's fail-closed identity checks and explicit gap reporting.

---

## 3. Platforms and permission models

### 3.1 Windows: UI Automation (UIA) and MSAA
- API: COM `IUIAutomation` (UIA3, `UIAutomationCore.dll`), managed `System.Windows.Automation` (UIA2, older/less complete; FlaUI notes UIA2 "works poorly with WPF and Store apps", UIA3 "can have bugs with WinForms"). Legacy MSAA (`IAccessible`, oleacc) is bridged into UIA via proxies and exposed through `LegacyIAccessiblePattern`.
- Chromium/Edge/Electron on Windows: OBSERVED (Chrome for Developers blog, 2025-08-14) native UIA is **on by default from Chrome 138**; the MSAA->UIA emulation layer is removed; MSAA/IAccessible2 clients still work; enterprise policy `UiAutomationProviderEnabled` can revert through Chrome 146. Earlier rollout began Chrome 126 (2024). INFER: Electron apps inherit this per bundled Chromium version; older Electron still needs `--force-renderer-accessibility` or AT detection (Python-UIAutomation README says Chrome/Electron "require the --force-renderer-accessibility flag").
- Chromium AT detection on Windows: OBSERVED Chromium fires `NotifyWinEvent(EVENT_SYSTEM_ALERT, objectId=1)` and treats a `WM_GETOBJECT` for that ID as "AT present" (chromium.org accessibility design doc). INFER: any UIA client walking a Chrome window normally triggers this, so trees populate after the first query (with a delay).
- **Permission / integrity model** (OBSERVED, uiauto-securityoverview, ms.date 2026-02-18):
  - No user consent prompt exists for UIA. Any medium-IL process can read and drive any other medium-IL window.
  - Elevated (high-IL) windows, UAC consent dialog and logon screen are inaccessible unless the client has `uiAccess="true"` in its manifest **and** is Authenticode-signed **and** installed in a secure location (e.g. Program Files) **and** launched by an admin user. UIAccess without admin yields "medium+" IL which still cannot touch high-IL UI. Nothing reaches system-IL UI on the secure desktop. UWP apps cannot request UIAccess.
  - Python-UIAutomation README: "Running Python as administrator is recommended, since enumeration can fail otherwise" (consistent with IL rule).
- 2025-2026 OS direction (OBSERVED, press coverage of Build 2025 and Oct 2025 announcements; Microsoft primary docs not fetched): native MCP Registry for Windows + "App Actions" API, private developer preview May 2025, with a proxy mediating MCP traffic, code-signing and declared-privilege requirements; Oct 2025 "Copilot Actions" / "agent workspace" runs agents in a separate Windows session under an agent account, opt-in via Settings > System > AI components > Agent tools > Experimental agentic features. UNKNOWN whether UIA is the mechanism used inside the agent workspace. UNKNOWN current (Nov 2026) preview status.
- Inspectors: Accessibility Insights for Windows (MS-recommended; Live Inspect, Event mode), legacy `Inspect.exe` (Windows SDK), `UISpy` (very old), FlaUInspect (FlaUI project), `automation.py -t 3` (Python-UIAutomation).

### 3.2 macOS: AXUIElement (HIServices / ApplicationServices) and NSAccessibility
- API: C API `AXUIElementCreateApplication(pid)`, `AXUIElementCreateSystemWide()`, `AXUIElementCopyAttributeValue(s)`, `AXUIElementSetAttributeValue`, `AXUIElementIsAttributeSettable`, `AXUIElementCopyActionNames` / `PerformAction`, `AXUIElementCopyElementAtPosition`, `AXObserverCreate` + run-loop source, `AXUIElementSetMessagingTimeout`. App side: `NSAccessibility` protocol / `NSAccessibilityElement`. (OBSERVED usage in REA Swift bridge; Apple page body not fetchable in this session.)
- **Permission model (TCC)**:
  - OBSERVED (forum/docs via search): `AXIsProcessTrusted()` checks; `AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: true})` prompts once and returns immediately, so callers poll. Without trust: `kAXErrorAPIDisabled`. Grant lives in System Settings > Privacy & Security > Accessibility and is **per executable/bundle identity**, so an MCP server launched via `npx`/`uvx` is trusted as the *node/python/uv binary path*, not as "your agent" (INFER from TCC behaviour; matches REA remediation text "Grant Accessibility to the REA host").
  - Sandboxed apps: prompt never appears / API unusable for controlling other apps (Apple forum thread 810677, OBSERVED via search).
  - Separate TCC services are involved in a full drive loop: **Accessibility** (AX reads/actions and `CGEventPost`; developer forum notes post-event permission `kTCCServicePostEvent` shows under the Accessibility pane), **Input Monitoring** (listening via event taps), **Screen Recording** (screenshots, ScreenCaptureKit; REA uses it), **Automation** (Apple Events to a specific app, separate prompt per target app). Peekaboo README lists Screen Recording + Accessibility + "a further permission for synthetic input".
  - OBSERVED (Apple forums, late 2025 / 2026): macOS 26.1 (Tahoe) had a bug where plain (non-bundle) executables could not be added to Accessibility / Screen Recording lists; reported fixed in 26.3 beta. Developers reported the old "first keypress opens the permission window" flow no longer works. INFER: ship the control plane host as a signed `.app` bundle with a stable bundle ID to survive TCC identity rules.
  - Also OBSERVED (Apple forum 794253): `AXIsProcessTrusted` can be true while `AXUIElementCopyAttributeValue` returns `.cannotComplete` (sporadic empty data) — flakiness exists even when permission is granted.
- Electron/Chromium on macOS: OBSERVED (Electron docs) the tree is **off** until VoiceOver sets `AXEnhancedUserInterface` or a third party sets the `AXManualAccessibility` attribute on the app element (Electron ships ObjC/Swift samples), or the app calls `app.setAccessibilitySupportEnabled(true)`. Electron API docs warn rendering the tree "can significantly affect the performance of your app".
- Inspector: Xcode Accessibility Inspector; CLI `axorc tree` (AXorcist), `peekaboo see --json`.

### 3.3 Linux: AT-SPI2 (and Wayland)
- Architecture (OBSERVED, libatspi docs + GTK blog): apps and ATs talk over a **dedicated accessibility D-Bus bus** (separate from session bus; address discovered via `org.a11y.Bus` on the session bus); `at-spi2-registryd` maintains the registry; libatspi default method-call timeout **800 ms** (`atspi_set_timeout`), registryd uses 3 s for keyboard notifications. Toolkits: GTK4 has its own AT-SPI backend (GTK blog 2020-10, still default per GTK blog 2025-05-12 even with the new AccessKit backend available); Qt via `QT_ACCESSIBILITY`/AT-SPI plugin; Chromium/Electron via ATK bridge; Java via `java-atk-wrapper`.
- **Enablement gates** (OBSERVED): Qt only becomes accessible when `org.a11y.Status.IsEnabled` and `ScreenReaderEnabled` are true, or `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` is set (Debian bug 874054 from 2017 notes a "huge negative performance impact" when always-on); GTK4 backend selectable with `GTK_A11Y=atspi|none`; Chromium/Electron need AT detection or `--force-renderer-accessibility`. INFER: a Linux control plane should set `org.a11y.Status.IsEnabled=true` (what Orca does) and/or set the toolkit env vars for apps it launches, and must expect pre-running apps to have empty trees.
- **Permission model**: no consent prompt; any process with the session's a11y bus address can read/drive everything on it. Sandboxing: Flatpak exposes a filtered a11y bus proxy; `--no-a11y-bus` / `--a11y-bus` flags; abstract-socket issue (at-spi2-core #40) was about sandbox weakness. INFER: agent host running outside Flatpak has full access; agent host *inside* Flatpak needs `--talk-name=org.a11y.Bus`-style grants.
- **Wayland**: reading works (AT-SPI is D-Bus, not X11). Input synthesis is the problem: `atspi_generate_mouse_event` is XTest-based (libatspi exposes `DeviceX11`); on Wayland use the **RemoteDesktop portal + libei** (GNOME 46+, KDE Plasma 6; first use triggers a consent dialog with restore token; wlroots portal still lacks libei per 2026 roadmap note), compositor-specific virtual-keyboard/pointer protocols (wlroots), or `ydotool` (`/dev/uinput`, needs root/udev, no focus awareness). dogtail 1.0.x (latest 1.0.7, July 2025) uses `gnome-ponytail-daemon` for Wayland input and dropped Qt/KDE support. Newton (Wayland-native push protocol by Matt Campbell): last public status June 2024, GNOME Shell still on AT-SPI, mouse synthesis "not yet supported", AT protocol only discovers surfaces on focus/pointer enter; author flags that "automated testing tools which currently use AT-SPI" may need a separate solution. UNKNOWN current 2025-26 Newton merge status (a March 2026 BRLTTY list post still describes it as D-Bus-based and in progress).
- Inspector: Accerciser (GNOME; maintenance status UNKNOWN), `busctl --user tree`/`gdbus introspect` on the a11y bus, `pyatspi` (dogtail notes a transition from pyatspi to GI-introspected `Atspi`).

---

## 4. Representative tools

| Tool | URL | What it does | License |
|---|---|---|---|
| pywinauto | https://pywinauto.readthedocs.io / https://github.com/pywinauto/pywinauto | Python; `backend="win32"` (HWND/messages, MFC/VB6/legacy) or `backend="uia"`; 0.6.9 released 2025-01-06 (OBSERVED via directory listing) | BSD-3 (UNKNOWN - not verified this session) |
| FlaUI | https://github.com/FlaUI/FlaUI | .NET wrapper of UIA2/UIA3 with FlaUInspect; ~3.2k stars, 764 commits (OBSERVED) | MIT |
| uiautomation (Python-UIAutomation-for-Windows) | https://github.com/yinkaisheng/Python-UIAutomation-for-Windows | Python/comtypes UIA3 wrapper, `automation.py` tree dumper; used by Windows-MCP; ~3.6k stars; "spare time" project | Apache-2.0 |
| WinAppDriver / appium-windows-driver | https://github.com/microsoft/WinAppDriver | WebDriver server over UIA; **unmaintained** by Microsoft, non-W3C, WebdriverIO advises against (OBSERVED) | MIT |
| Windows-MCP (CursorTouch) | https://github.com/CursorTouch/Windows-MCP | MCP server: `Snapshot` (UIA tree with interactive element IDs, optional `use_vision`), `Screenshot`, Click/Type/Scroll/Shortcut, App/PowerShell/Registry/Clipboard...; claims 0.2-0.5 s per action; Python 3.14+; Win10/11 | MIT |
| Microsoft UFO / UFO2 "Desktop AgentOS" | https://github.com/microsoft/UFO , https://arxiv.org/abs/2504.14603 | HostAgent + AppAgents; `control_backend: uia|win32`; hybrid UIA + OmniParser visual grounding; prefers native APIs (xlwings, win32com) over GUI; speculative multi-action (51% fewer LLM calls) | MIT |
| Terminator (mediar-ai) | https://github.com/mediar-ai/terminator | Rust "Playwright for Windows"; selectors `name:`, `role:`, `window:` from UIA; MCP agent via `npx terminator-mcp-agent`; YAML workflows; README says Windows-only, macOS/Linux rows all "No" | MIT |
| atomacos (fork of ATOMac/pyatom) | https://github.com/daveenguyen/atomacos | Python 3 pyobjc wrapper over AX API; raises `AXErrorAPIDisabled` without trust; ATOMac last tested on 10.6-10.8 | MIT (UNKNOWN - not verified) |
| AXorcist (steipete) | https://github.com/steipete/AXorcist | Swift wrapper + `axorc` CLI (JSON protocol, `tree`, `find`, `permissions`); criteria role/title/value/identifier/DOM class with exact/contains/regex/prefix/suffix; actions press/increment/decrement/confirm/cancel/showMenu/pick/raise; observers; macOS 14+ | MIT |
| Peekaboo (steipete) | https://github.com/steipete/Peekaboo | macOS CLI + menu-bar app + MCP (`npx @steipete/peekaboo`); `see --json` returns UI map with opaque element IDs, then `click`/`type`/`press` by ID; background input without foregrounding; agent mode; macOS 15+ | MIT |
| Ghost OS (ghostwright) | https://github.com/ghostwright/ghost-os | Swift MCP server on AXorcist, ~29 tools, local vision fallback (ShowUI-2B); macOS 14+, Swift 6.2+; "no built-in auth or access control" | UNKNOWN |
| MacosUseSDK / mcp-server-macos-use (mediar-ai) | https://github.com/mediar-ai/MacosUseSDK | Swift lib + CLI: `TraversalTool` dumps AX tree as JSON; input simulation; MCP wrapper | UNKNOWN |
| macOS-use (browser-use, `mlx-use`) | https://github.com/browser-use/macOS-use | Python agent over AX tree; PyPI warns it "can and will use your private credentials"; MLX local inference is future work | MIT (UNKNOWN) |
| macos-mcp / MacOS-Use (CursorTouch) | https://pypi.org/project/macos-mcp/ | PyObjC + AX API MCP server; needs Accessibility + Screen Recording; "executes real system operations without sandboxing" | MIT |
| dogtail | https://gitlab.com/dogtail/dogtail , https://pypi.org/project/dogtail | Python AT-SPI test framework; Wayland via gnome-ponytail-daemon; GTK/GNOME only; 1.0.7 (2025-07) | GPL-2.0 (UNKNOWN - not verified) |
| LDTP / Cobra | http://ldtp.freedesktop.org | Legacy cross-platform AT-SPI/AX automation; maintenance status UNKNOWN (only historical hits) | LGPL (UNKNOWN) |
| pyatspi / libatspi | https://gitlab.gnome.org/GNOME/at-spi2-core | Reference AT-SPI2 client library; 2.56.4 (2025-08-02) | LGPL-2.1 |
| AccessKit | https://github.com/AccessKit/accesskit | Rust cross-platform *provider-side* abstraction (UIA/AX/AT-SPI); relevant when the user owns the app and can add a11y | MIT/Apache-2.0 (UNKNOWN) |
| Accessibility Insights for Windows | https://accessibilityinsights.io | MS inspector (Live Inspect, Event mode); replaces Inspect.exe | MIT |
| Xcode Accessibility Inspector / Accerciser | Xcode; https://gitlab.gnome.org/GNOME/accerciser | Platform inspectors for hand-discovery of roles/IDs | Apple EULA / GPL |
| Anthropic computer-use tool (reference) | https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool | Screenshot-only toolset (`computer_toolset_20260801`, `computer_20251124`, `computer_20250124`); 17 actions incl. `zoom`; coordinate-scaling guidance; Docker/Xvfb/Mutter reference; **no a11y API use** | reference demo MIT (quickstarts) |

---

## 5. Strengths

- **Semantic, text-native observation**: roles/names/values/states in a few KB instead of a 1-4 MP image; secondary estimates put a page snapshot at ~500-5,000 tokens vs 10k-50k for a screenshot (newsletter figure, not primary). OSWorld authors report the a11y tree "can boost some models' performance by providing structured information".
- **Deterministic actuation without pointer**: `Invoke`/`AXPress`/`do_action` work on offscreen, occluded, or background windows (Peekaboo advertises background input; UIA patterns do not need focus). No coordinate-scaling or Retina/DPI mistakes.
- **Reading hidden state**: values of unlabeled fields, `IsPassword`, `IsOffscreen`, `IsEnabled`, selection state, text ranges with attributes (UIA TextPattern), table/grid coordinates, list virtualization (`VirtualizedItemPattern.Realize`).
- **Events instead of polling**: focus/structure/property-changed events (UIA), AXObserver, AT-SPI signals let a plane wait for "dialog appeared" or "value changed" deterministically.
- **Security side-benefit**: RedTeamCUA (OBSERVED via search) found adding the a11y tree "substantially reduces" indirect-prompt-injection attack success across CUAs (while lowering raw task success for some).
- **Mature Windows ecosystem**: caching (`IUIAutomationCacheRequest`), tree walkers, FlaUI/pywinauto/uiautomation wrappers, Chromium native UIA since Chrome 138, Unity's accessibility module maps to UIA (`IRawElementProviderSimple`).

---

## 6. Failure modes

1. **Empty or shallow trees** (most common):
   - Chromium/Electron with AT not detected (Windows: wait for `WM_GETOBJECT` handshake; macOS: must set `AXManualAccessibility`/`AXEnhancedUserInterface`; Linux: `--force-renderer-accessibility`). First query returns a bare pane; tree appears seconds later. Electron warns of performance cost once enabled.
   - Qt apps on Linux without screen-reader flag or `QT_LINUX_ACCESSIBILITY_ALWAYS_ON`.
   - Flutter desktop: Windows UIA only if engine built with `FLUTTER_ENGINE_USE_UIA`; otherwise MSAA/IAccessibleEx (OBSERVED Flutter docs). macOS Flutter AX support UNKNOWN this session.
   - Games (Unity/Unreal), canvas/WebGL apps (Figma-style), owner-drawn/DirectUI controls (`FrameworkId="DirectUI"`), remote-desktop/VM windows (one opaque bitmap), terminal emulators (text often exposed as one blob or not at all).
   - Java Swing needs Java Access Bridge enabled (`jabswitch`), 64-bit only per Sema4 docs; Access Bridge Explorer archived 2022.
2. **Mislabeled trees**: `Name` empty or icon-only, ARIA misuse (WebAIM 2026 data cited by secondary source: pages with ARIA average more errors), duplicated names among siblings (UIA docs: "Name cannot be used as a unique identifier among siblings").
3. **Identity instability**: `RuntimeId` "can be reused over time", "format can change", opaque, comparison only; `AutomationId` "not guaranteed to be stable across different releases"; AX has no durable IDs; AT-SPI object paths are per-process-lifetime. Element references die on re-render (UIA `UIA_E_ELEMENTNOTAVAILABLE`, AX `.invalidUIElement`). REA mitigates with index paths re-validated at action time plus PID ownership check (ReaNativeUI.swift:100-108).
4. **Latency and hangs**: every property read is a cross-process call (UIA docs: "slow and inefficient because it requires a cross-process call for each property"); AX calls block until the target's main thread services them (`AXUIElementSetMessagingTimeout`, REA sets 2 s; default longer); AT-SPI 800 ms default timeout per call; busy or modal apps stall the whole walk. OSWorld-Human (arXiv 2506.16042) found adding the a11y tree "drastically increases per-task latency" in some apps. Windows-MCP self-reports 0.2-0.5 s per action.
5. **Tree size**: desktop trees hold thousands of nodes; unfiltered dumps blow the context window. Need control-view filter, depth/node caps (REA: `max_nodes`, depth 32, explicit `truncated`), interactive-only filters (Windows-MCP "interactive element IDs"), virtualization handling.
6. **Permission denials and TCC quirks**: macOS `AXIsProcessTrusted` true but reads fail `.cannotComplete`; Tahoe 26.1 bug hid plain executables from the Accessibility list (fixed 26.3 beta per forum); grant is tied to the binary path so `npx`/`uvx` launchers inherit or lose trust unpredictably. Windows: elevated apps invisible without UIAccess+signing+admin. Linux: a11y bus absent in some sessions (headless, minimal WMs), Flatpak proxy filtering.
7. **Action semantics mismatch**: `Invoke` fires the control's handler but skips hover/mousedown effects; `ValuePattern.SetValue` bypasses key events (no autocomplete, no `input` events in some web apps); `AXPress` on custom views may be a no-op while reporting success. Hybrid tools fall back to synthesized clicks at `ClickablePoint`.
8. **Wayland input gap**: tree readable, pointer/keyboard synthesis requires portal consent or root-level uinput.
9. **Prompt injection via tree text**: hidden nodes, `aria-label`/`aria-describedby` content, notification text, invisible `display:none`-adjacent constructs that still reach the tree. Secondary tests report agents processing "the full accessibility tree... significantly more susceptible" than visible-text extractors. REA marks target content as untrusted by default (`TARGET_CONTENT_FREE_TOOLS` allowlist, per salvaged capability-registry notes) and gates a11y text capture behind an opt-in flag.
10. **Screen reader side effects**: enabling AT detection in Chromium/Qt globally changes app performance; setting `org.a11y.Status.ScreenReaderEnabled` may start Orca-oriented behaviours in GNOME. INFER: scope enablement to processes you launch.

---

## 7. Trade-offs vs other technique families

| vs | Accessibility tree wins when | Loses when |
|---|---|---|
| **Screenshots + VLM grounding** (Anthropic computer use, OpenAI CUA, Agent S2 screenshot-only) | App exposes a populated tree; need exact text/values; background or offscreen operation; cost/latency per step; determinism | Custom-drawn/canvas/game UIs; tree disabled; visual layout matters (overlap, z-order, colour); cross-checking what the user actually sees. Evidence mixed: OSWorld says tree helps some models; RedTeamCUA shows tree lowering success for Claude 3.7 CUA (96.16 -> 84.43) while improving injection resistance |
| **Input synthesis only** (xdotool, SendInput, CGEvent, libei) | You need to read state, not just act; avoid focus stealing | Only choice for games/remote desktops; needed for hover/drag/gesture semantics |
| **In-process instrumentation** (Frida, CDP, DevTools, REA's deep providers) | No code injection, no anti-tamper triggers, OS-sanctioned, works on signed/hardened apps | Need internal state not surfaced in UI; need to call app functions directly; a11y tree too shallow. Injection raises the licensing/anti-cheat boundary the project excludes |
| **App-native APIs** (COM/AppleScript/App Actions/MCP servers shipped by vendors) | App has no API or API is partial (UFO2 explicitly prefers native API and falls back to GUI) | Vendor API exists: faster, stable contracts, intended use |
| **DOM/CDP for web content** | Hybrid Electron apps whose chrome is native but content is web; Windows-MCP `use_dom=True` and REA's CDP a11y capture show both coexisting | Pure web: CDP gives richer, more stable selectors than UIA passthrough of ARIA |

INFER: the pragmatic design shared by UFO2, Windows-MCP, Peekaboo, Ghost OS, Terminator and Agent S (v1) is **tree-first with screenshot/vision fallback and native API preference**; Anthropic's reference stays screenshot-only for uniformity across arbitrary apps.

---

## 8. Integration notes for an MCP control plane

Process model
- Host one **per-OS a11y driver process**, long-lived, owning COM apartment (Windows UIA needs STA/MTA consistency and the `IUIAutomation` object reused), the AX run loop (AXObserver needs a CFRunLoop), or the libatspi main loop (GLib). REA's pattern of a separate signed Swift helper (`bridge/native/ReaNativeUI.swift`) called per request is simpler but loses observers; keep a resident helper for events.
- Element handles: never hand raw `RuntimeId`/AXUIElementRef/object paths to the model. Mint short **session IDs** mapped to (process, window, locator = role+name+automationId/identifier+index path, last bbox), re-resolve on use, verify PID ownership (REA line 108), fail with a typed `element_stale` error.
- Snapshot tool shape (converge with Windows-MCP `Snapshot`, Peekaboo `see --json`, UFO2 `get_ui_tree`): `{window, nodes:[{id, role, name, value, states, bbox, actions[], children}], truncated, gaps[]}` with filters `interactive_only`, `max_nodes`, `max_depth`, `view: raw|control|content`, and optional annotated screenshot (Set-of-Mark style). Always include a `coverage` field: `full | truncated | tree_unavailable | at_not_enabled` so the agent knows when to switch to vision.
- Action tools map to *patterns*, not pixels: `invoke`, `toggle`, `set_value`, `select`, `expand`, `scroll_into_view`, `focus`, `close_window`, with `fallback: click_point` when the pattern is missing. Report which path executed.
- Wait primitives built on events: `wait_for(role/name appears | focus changed | window title matches | value equals)` with deadline; REA caps one wait step at 180 s.

Auth/permission prompts
- macOS: preflight `AXIsProcessTrusted()`; if false return `permission_required` with remediation (System Settings path) instead of prompting mid-task; ship as a signed `.app` with stable bundle ID so TCC identity survives updates (Tahoe 26.1 non-bundle bug); also preflight Screen Recording (for screenshots) and expect a separate portal/consent for synthetic input. Cache nothing: user can revoke at any time.
- Windows: no prompt. Document that elevated windows need the host to run elevated or carry UIAccess (signed + Program Files + admin launch). Detect `UIA_E_ELEMENTNOTENABLED`/access-denied on high-IL windows and report `integrity_level_blocked`.
- Linux: probe `org.a11y.Bus GetAddress`; if absent report `a11y_bus_unavailable`; for Wayland input use `org.freedesktop.portal.RemoteDesktop` (consent dialog on first use; persist the restore token in user config, never in logs).
- Enabling AT mode in targets you *launch*: pass `--force-renderer-accessibility` (Chromium/Electron), set `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`, set `AXManualAccessibility` (macOS Electron) only for the target PID. Record this as an effect (`changesTargetBehaviour: a11y_enabled`) in the tool's effect audit.

Latency
- Budget per snapshot: use `CacheRequest` with `FindAllBuildCache` on Windows (one IPC per subtree); on macOS batch with `AXUIElementCopyMultipleAttributeValues` and set `AXUIElementSetMessagingTimeout` (REA: 2 s); on Linux prefer `Collection` and set `atspi_set_timeout`. Expect 50-500 ms for a window-scoped control-view snapshot, seconds for unfiltered trees or busy apps (OBSERVED ranges only from Windows-MCP's 0.2-0.5 s self-report; others INFER).
- Run snapshot and screenshot concurrently; return both when `use_vision` requested.

State
- Keep per-session: target PID + executable identity + launch time (REA guards PID reuse), window handle, last snapshot hash, element ID map, observers/subscriptions, enablement side effects to undo on close (e.g. `AXManualAccessibility=false`).
- Mark all tree-derived strings as untrusted content in the MCP result (REA's `returnsTargetContent` flag); offer `include_text: false` mode for sensitive apps (REA gates a11y names/descriptions behind `include_accessibility_text`); honour `IsPassword`/`AXSecureTextField` by never returning values.

---

## 9. Authorization and legal notes

- The APIs are **public, documented, OS-sanctioned** interfaces for assistive technology and test automation; using them on software the user owns/runs is their intended use. No DRM, no code injection, no memory tampering. This is the least boundary-sensitive technique family in the survey.
- Platform terms still bite in places: Apple App Review rejects Mac App Store apps for `CGEvent.post`-based automation without AX APIs (Guideline 2.4.5 report, Apple forum 820594) and sandboxed apps cannot use AX on other apps, so a control plane host must be distributed outside the Mac App Store (Developer ID + notarization). Microsoft says UIAccess "should not be used by applications that are not assistive technologies" (uiauto-securityoverview); INFER: an agent host requesting UIAccess to reach UAC prompts is outside the documented intent and defeats a security control, which the project's scope excludes. Recommend: never request UIAccess; treat elevated UI as out of reach.
- Secure desktop / logon screen / UAC prompts are intentionally inaccessible; do not try to bridge them.
- Reading other apps' trees exposes **passwords, tokens, private messages** (`IsPassword` fields can still error or leak via `Value`; chat apps expose full message text). This is user data on the user's machine, but the plane should minimise retention (REA redacts/opt-ins) and log what was read.
- Chromium's "AT detected" handshake and `AXManualAccessibility` are documented, legitimate toggles; they are not security controls. `QT_LINUX_ACCESSIBILITY_ALWAYS_ON` likewise.
- Third-party license mix: MIT dominates (Windows-MCP, UFO2, Terminator, AXorcist, Peekaboo, FlaUI, pywinauto-BSD); Apache-2.0 (uiautomation); LGPL (libatspi); dogtail GPL (UNKNOWN exact). None impose field-of-use restrictions, but GPL components should stay out-of-process.
- "Not an A11y" (arXiv 2608.08939) and failurefirst ARIA tests show the tree is an **indirect prompt-injection channel**; treat as a safety obligation, not just robustness.

---

## 10. Open questions

1. Newton/Wayland: has any part merged into Mutter/GTK/Orca by late 2026, and will it keep an AT-SPI compatibility path for automation tools? (Last primary status: June 2024.)
2. Windows "agent workspace" (Oct 2025 announcement): does Microsoft expose UIA, App Actions, or an MCP registry inside the agent session, and can third-party MCP servers run there? Primary docs not fetched.
3. macOS 26.x TCC: confirmed state of the non-bundle Accessibility-list bug in shipping 26.3+; whether Apple tightened first-keypress prompting deliberately.
4. Quantified latency: no primary measurements found for AX/AT-SPI full-window snapshots; need our own benchmark across Electron, Qt, WinUI, SwiftUI apps.
5. Agent S2/S3 ablations on tree vs screenshot were not read from the primary PDF; OSWorld main paper observation-mode table not fetched (arXiv landing page only).
6. Flutter macOS AX support and Unreal Engine accessibility exposure: not verified.
7. Exact licenses for pywinauto, atomacos, dogtail, LDTP, Ghost OS, MacosUseSDK, browser-use/macOS-use: not verified this session.
8. Does Chrome still honour `UiAutomationProviderEnabled` (promised through Chrome 146) and what Electron versions ship Chromium >= 138 with native UIA?
9. How Windows-MCP's `Snapshot` derives "interactive element IDs" (filter rules) — README summary only; source not read.
10. Whether Microsoft's App Actions / Windows MCP Registry has left private preview (May 2025) as of Nov 2026.

---

## 11. Sources

Primary documentation
- UIA control patterns overview (ms.date 2025-07-14): https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-controlpatternsoverview
- UIA control pattern mapping: https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-controlpatternmapping
- UIA tree overview (raw/control/content): https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-treeoverview
- UIA automation element property IDs (RuntimeId, AutomationId, Name...): https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-automation-element-propids
- UIA security / UIAccess / integrity levels (ms.date 2026-02-18): https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview
- UIA caching for clients: https://learn.microsoft.com/en-us/windows/desktop/WinAuto/uiauto-cachingforclients
- IUIAutomationCacheRequest: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nn-uiautomationclient-iuiautomationcacherequest
- Accessibility testing tools (Accessibility Insights, Inspect): https://learn.microsoft.com/en-us/windows/uwp/accessibility/accessibility-testing
- Chrome: Native UI Automation for Windows in Chromium (2025-08-14): https://developer.chrome.com/blog/windows-uia-support-update
- Chrome: Introducing UIA support on Windows (2024): https://developer.chrome.com/blog/windows-uia-support
- Chromium accessibility design doc (AT detection, --force-renderer-accessibility): https://www.chromium.org/developers/design-documents/accessibility/
- Electron accessibility tutorial (AXManualAccessibility, setAccessibilitySupportEnabled): https://www.electronjs.org/docs/latest/tutorial/accessibility
- Apple AXUIElement.h: https://developer.apple.com/documentation/applicationservices/axuielement_h (body not fetchable in session)
- Apple forum: AXIsProcessTrusted true but reads fail: https://developer.apple.com/forums/thread/794253
- Apple forum: binary executable Accessibility permission in Tahoe: https://developer.apple.com/forums/thread/808897
- Apple forum: executables missing from Screen Recording list in 26.1: https://developer.apple.com/forums/thread/807898
- Apple forum: sandboxed app Accessibility prompt: https://developer.apple.com/forums/thread/810677
- Apple forum: CGEvent.post rejection / kTCCServicePostEvent: https://developer.apple.com/forums/thread/820594
- libatspi (AT-SPI2) reference: https://docs.gtk.org/atspi2/
- at-spi2-core (abstract socket issue #40): https://gitlab.gnome.org/GNOME/at-spi2-core/-/issues/40
- GTK: Accessibility in GTK 4 (2020-10-21): https://blog.gtk.org/2020/10/21/accessibility-in-gtk-4/
- GTK: An accessibility update (2025-05-12, AccessKit backend, AT-SPI default): https://blog.gtk.org/2025/05/12/an-accessibility-update/
- GTK running/debugging (GTK_A11Y): https://docs.gtk.org/gtk4/running.html
- GNOME a11y blog (Newton updates, 2023-2024): https://blogs.gnome.org/a11y/
- GNOME STF 2024 report (Newton): https://blogs.gnome.org/tbernard/?p=10486
- BRLTTY list, March 2026 (Newton still D-Bus): https://brltty.app/pipermail/brltty/2026-March/021759.html
- wayland-devel, May 2025 (libei via RemoteDesktop portal): https://lists.freedesktop.org/archives/wayland-devel/2025-May/044241.html
- Flatpak a11y bus design issue: https://github.com/flatpak/flatpak/issues/79
- flatpak run man page (--a11y-bus/--no-a11y-bus): https://dyn.manpages.debian.org/buster-backports/flatpak/flatpak-run.1
- Debian bug 874054 (QT_LINUX_ACCESSIBILITY_ALWAYS_ON perf): https://lists.alioth.debian.org/pipermail/pkg-a11y-devel/Week-of-Mon-20170828/000075.html
- Flutter Accessibility on Windows (UIA/MSAA, FLUTTER_ENGINE_USE_UIA): https://flutter.googlesource.com/mirrors/flutter/+/HEAD/docs/platforms/desktop/windows/Accessibility-on-Windows.md
- Oracle Java Access Bridge: https://www.oracle.com/java/technologies/javase/accessbridge.html
- Unity Accessibility module: https://docs.unity3d.com/Manual/accessibility/module-intro.html
- Anthropic computer use tool: https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool

Tools / projects
- Windows-MCP: https://github.com/CursorTouch/Windows-MCP ; PyPI https://pypi.org/project/windows-mcp/
- UFO2 docs: https://microsoft.github.io/UFO/ufo2/overview ; paper https://arxiv.org/abs/2504.14603
- Terminator: https://github.com/mediar-ai/terminator ; crate https://docs.rs/crate/terminator-rs
- FlaUI: https://github.com/FlaUI/FlaUI
- Python-UIAutomation-for-Windows: https://github.com/yinkaisheng/Python-UIAutomation-for-Windows
- pywinauto docs: https://pywinauto.readthedocs.io/en/latest/getting_started.html
- WinAppDriver status: https://github.com/microsoft/WinAppDriver/issues/1550 ; WebdriverIO note https://webdriver.io/docs/desktop-testing/windows
- AXorcist: https://github.com/steipete/AXorcist
- Peekaboo: https://github.com/steipete/Peekaboo
- Ghost OS: https://github.com/ghostwright/ghost-os
- MacosUseSDK: https://github.com/mediar-ai/MacosUseSDK ; MCP wrapper https://glama.ai/mcp/servers/mediar-ai/mcp-server-macos-use/tree
- browser-use/macOS-use: https://github.com/browser-use/macOS-use ; PyPI https://pypi.org/project/mlx-use/
- macos-mcp (CursorTouch): https://pypi.org/project/macos-mcp/
- atomacos: https://github.com/daveenguyen/atomacos ; ATOMac https://pypi.org/project/pyatomac/
- dogtail: https://pypi.org/project/dogtail ; README https://sources.debian.org/src/dogtail/1.0.0-6/README.rst
- Accessibility Insights for Windows blog: https://devblogs.microsoft.com/engineering-at-microsoft/accessibility-insights-for-windows/
- Desktop accessibility inspectors overview: https://quorumlanguage.com/tutorials/accessibility/desktopaccessibilityinspectors.html

Research / evidence
- OSWorld: https://arxiv.org/abs/2404.07972
- OSWorld-Human (a11y tree latency): https://arxiv.org/pdf/2506.16042
- Agent S: https://arxiv.org/pdf/2410.08164 ; Agent S2: https://arxiv.org/abs/2504.00906
- RedTeamCUA (a11y tree vs injection): https://arxiv.org/pdf/2505.21936
- Not an A11y (Android a11y tree injection): https://arxiv.org/pdf/2608.08939
- ARIA injection notes (secondary): https://8873116e.failurefirst-org.pages.dev/research/prompt-injection/07-aria-attributes/
- Windows agentic features coverage (Oct 2025): https://petri.com/windows-11-agentic-computing-workspaces/ ; https://www.tomshardware.com/software/windows/microsofts-vision-of-ai-native-windows-is-becoming-real-update-introduces-agents-that-pilfer-through-your-files-latest-windows-11-insider-build-includes-experimental-ai-agents-toggle-that-can-perform-tasks-for-you-in-the-background
- Windows MCP / App Actions at Build 2025 coverage: https://www.windowscentral.com/software-apps/windows-11/windows-11-model-context-protocol-mcp-ai-announcement-build-2025 ; https://devclass.com/2025/05/19/mcp-will-be-built-into-windows-to-make-an-agentic-os-but-security-will-be-a-key-concern/

REA checkout (read-only)
- `bridge/native/ReaNativeUI.swift` (AX trust check, messaging timeout, geometry-matched window, index-path actions, max_nodes/truncated/gaps)
- `docs/native-investigation.md:995-1035` (native desktop observation contract)
- `docs/mcp-contracts.md:1264-1266` (permission denial reporting)
- `docs/browser-observation.md:343-356,470` and `tests/boundary/cli/accessibilityCaptureComparison.test.ts:16-45` (CDP a11y capture shape, text opt-in)
