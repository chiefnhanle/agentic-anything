# Survey: OS-level automation APIs (technique family: `os-automation-apis`)

Research date: 2026-11-04. Scope: first-party OS mechanisms an agent can use to observe and drive software the user owns, without reverse engineering the target. Inputs: primary vendor docs where reachable (Apple, Microsoft Learn, freedesktop portal specs), WWDC25/26 session transcripts, project READMEs, and press for status-of-shipping questions. Each claim is tagged **OBS** (read on a page / in source) or **INF** (inference). Unknowns are marked unknown. Web-search budget was exhausted near the end; a few items are flagged as not re-verified for that reason.

Comparison baseline: the REA checkout at `/tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src` (read-only).

---

## 1. Summary

- **What the family is.** Vendor-sanctioned, in-process or IPC-level control surfaces: macOS Apple Events (AppleScript/JXA/Scripting Bridge), App Intents + Shortcuts; Windows COM/OLE Automation, PowerShell, WinRT, and the 2025-26 "App Actions" / "MCP on Windows" stack; Linux D-Bus plus xdg-desktop-portal (RemoteDesktop/ScreenCast/GlobalShortcuts/InputCapture) and libei; cross-platform URL schemes / intents; and the synthetic-input fallbacks (SendInput, AutoHotkey, xdotool/ydotool/wtype).
- **Maturity is bimodal.** The classic layers (Apple Events, COM, D-Bus, SendInput) are 20-30 years old, stable, and documented; the agent-oriented layers (App Intents for Siri/Spotlight, App Actions on Windows, Windows ODR/MCP, portals v2 with `persist_mode`) are 2024-2026 and in several cases still labelled prerelease (**OBS**: MS Learn MCP/App Actions pages carry a "prereleased product" banner as of their Oct 2026 / Jun 2026 updates).
- **Coverage is the limiting factor, not reliability.** Where an app exposes a dictionary/COM object model/App Intent/D-Bus interface, the surface is structured, fast and verifiable. Most apps do not: on one author's Mac, 8 of 50 third-party apps shipped App Intents (**OBS**, intents-mcp README); AppleScript dictionary coverage across all apps is **unknown** (no survey found), and Electron/cross-platform apps generally ship none (**INF**).
- **Permission models differ sharply.** macOS gates every (sender app, target app) Apple-Event pair through TCC "Automation" consent and requires `NSAppleEventsUsageDescription` + `com.apple.security.automation.apple-events` for hardened/sandboxed senders. Windows has no consent prompt for COM or SendInput at the same integrity level but blocks SendInput across UIPI boundaries silently; the new MCP-on-Windows stack adds a separate agent user account and per-host folder consent. Linux portals prompt per session with optional persistence tokens; raw `uinput` (ydotool) needs root/udev and bypasses all of that.
- **Platform vendors are converging on "app-declared actions consumed by an OS registry"**: App Intents (Apple, exposed to Shortcuts/Spotlight/Siri; MCP strings found in iOS/macOS 26.1 betas but no shipped third-party MCP bridge confirmed), App Actions + ODR (Microsoft, public preview since Insider build 26220.7344, Nov 2025). Neither is GA as of this writing (**OBS** for Windows prerelease banners; Apple status **unknown** beyond WWDC26 session content, which never mentions MCP).
- **Bearing on the project boundary.** Everything here is designed for automating software you run; none of it defeats DRM/licensing. The boundary issues are (a) service ToS that forbid bots/macros even on your own machine (Steam SSA 4.C, Microsoft Services Agreement 3.a) and (b) OS consent systems (TCC, UIPI, portals) that an automation layer must respect rather than bypass.

---

## 2. What it gives an agent

| Capability | macOS | Windows | Linux (Wayland) | Cross-platform |
|---|---|---|---|---|
| Structured, typed read of app state | Apple Events object model (`get name of every window`), App Intents entities (via Shortcuts) | COM object model (Excel.Application, Shell.Application), WinRT, PowerShell | D-Bus properties/methods exposed by app; KWin scripting objects | URL scheme replies (x-callback-url) |
| Structured command | AppleScript/JXA `tell app`, `shortcuts run`, Scripting Bridge | COM `IDispatch`, App Actions `ActionInstance`, MCP connectors via ODR | D-Bus method calls, portal sessions, KWin `loadScriptFromText` | Deep links, Android `am start` |
| Discovery of what is scriptable | `.sdef` in bundle, `sdef`/`osascript` introspection; `extract.actionsdata` for App Intents | Type libraries / `Get-Member`; `ActionCatalog.GetAllActions`; `odr.exe mcp list` | `busctl --user list`, `Introspect` | `Info.plist` `CFBundleURLTypes`; registry `HKCR\<scheme>` |
| Synthetic input (fallback) | CGEvent (accessibility family) | `SendInput`, AutoHotkey Send modes | portal RemoteDesktop + libei; wlroots virtual-keyboard/pointer; `uinput` | - |
| Global hotkeys / triggers | Shortcuts Automations (Tahoe adds Mac automations) | - (AutoHotkey) | GlobalShortcuts portal | - |
| Screen observation | ScreenCaptureKit (other family) | - | ScreenCast portal (paired with RemoteDesktop session) | - |

**OBS** anchors: App Intents in Spotlight on Mac (WWDC25 session 260 transcript: "you can now run actions from your app directly from Spotlight on Mac"); `ActionCatalog` "Provides APIs for querying for app actions that are registered with the system" (MS Learn `Windows.AI.Actions.Hosting`); RemoteDesktop portal `SelectDevices` types bitmask 1 KEYBOARD / 2 POINTER / 4 TOUCHSCREEN (portal spec v2).

---

## 3. Platforms and permission models

### 3.1 macOS

**Apple Events / AppleScript / JXA / Scripting Bridge**
- Mechanism: inter-process Apple Events addressed to an app's scripting dictionary (`.sdef`). AppleScript and JavaScript for Automation are both OSA languages over the same transport; Scripting Bridge is the Objective-C/Swift binding. **OBS**: no official deprecation of JXA, Scripting Bridge or AppleScript found as of 2025-26 (MacScripter "Scripting Changes (or lack thereof) in macOS Tahoe"; DEVONthink dev: "We have not deprecated or ended support for JXA"). Status of Scripting Bridge beyond its 10.5 introduction page: **unknown** (no current Apple page located).
- Permission: TCC service `AppleEvents` ("Automation" pane). **OBS** (Eclectic Light, 2018, still the operative model): "When consent is obtained for Automation, it must be specific for that pair of apps"; benign events (e.g. reveal in Finder) pass without a dialog; events to System Events for login items etc. prompt. User scripts are not exempt: TCC has "no robust way of telling whether any given script or app is owned by the same person as the data". Pre-authorisation by the end user is not possible; signed script applets stop re-prompting; MDM PPPC payload (`AppleEvents` service with `AEReceiverIdentifier`) can pre-approve - **INF** from general knowledge, Apple page body not retrievable in this session.
- Hardened runtime / sandbox: senders need `com.apple.security.automation.apple-events` entitlement plus `NSAppleEventsUsageDescription`; sandboxed apps need `com.apple.security.scripting-targets` (per-target access groups) or `com.apple.security.temporary-exception.apple-events` (**OBS**, Apple App Sandbox Temporary Exception Entitlements page). Community reports say App Review rejects the temporary-exception form (**OBS** forum; not official policy).
- Who is "the sender" for `osascript`? The responsible process (e.g. Terminal, or the MCP host's parent) is what TCC attributes consent to. **INF** from the TCC model; the practical consequence is that an MCP server launched by an agent host inherits/produces prompts in that host's name.
- 2025-26 regressions: **OBS** reports of Apple Event timeouts (-1712) and slow error retrieval on macOS 26 Tahoe, Apple bug FB20174869 unfixed through GM (mjtsai.com 2025-09-17; MacScripter "AppleScript and macOS Tahoe 26"); a DEVONthink-forum hypothesis blames the rewritten TCC for apps built with the 26 SDK (not confirmed by Apple). Apple Community (Oct 2025): TV/Music dictionaries broken on 26; Apple's response that it "reserves the right to alter the Scripting Dictionary support for its applications at any time". System Events screen-saver query returning -10000 on 26.3 (Apple Community JP).

**App Intents / Shortcuts / Spotlight**
- **OBS** WWDC25-260: App Intents surface in Spotlight on Mac if the parameter summary covers required params and the intent is not `isDiscoverable: false` / `assistantOnly`. Personal Automations come to Mac (folder, external-drive, time-of-day). "Use Model" action routes shortcut data through on-device, Private Cloud Compute, or ChatGPT models.
- **OBS** `shortcuts(1)`: subcommands `run`, `list`, `view`, `sign`; `run --input-path` (`-` = stdin), `--output-path` (`-` = stdout), `--output-type <UTI>`. Creating/editing shortcuts requires the Shortcuts app.
- Agent bridge in the wild: **OBS** intents-mcp (MIT) reads `extract.actionsdata` under `/Applications` and `/System`, wraps each chosen action in a signed Shortcut, runs via `shortcuts run`. Limits reported: entity actions unsupported (657 of 945 discoverable actions on the author's Mac), only 8/50 third-party apps ship App Intents, each wrapper must be added by clicking "Add Shortcut" (no public API), first run may prompt and "Always Allow" is needed, `shortcuts sign` uses the user's iCloud account.
- WWDC26 (June 2026): **OBS** sessions 240 and 343 - App Schemas / schema domains make intents "executable by Siri through natural language"; SiriKit deprecation reported by third parties (not verified against Apple text). Neither transcript mentions MCP, agents or third-party AI. Third-party blog (petegoldsmith.com) asserts "no MCP anywhere in Siri's path" and that the Xcode 27 MCP is a dev-tools bridge; **OBS** that Xcode 26.3+ exposes an MCP server via `xcrun mcpbridge` (Cursor docs). Sept 2025 betas (iOS/macOS 26.1) contained MCP code in the App Intents framework (9to5Mac via AppleInsider/AlternativeTo). Whether Apple has shipped any MCP surface for third-party agents over App Intents: **unknown / not confirmed**.

### 3.2 Windows

**COM / OLE Automation, PowerShell, WinRT**
- COM `IDispatch` automation (Office, `Shell.Application`, `WScript.Shell`, `InternetExplorer.Application`) via `New-Object -ComObject` (**OBS**, MS Learn PowerShell sample; "This sample only runs on Windows platforms"; ActiveX EXE lifetime is not standardised, test `Quit()` behaviour per app).
- **OBS** Microsoft KB 257757: Office automation "from any unattended, non-interactive client application or component ... is not recommended and not supported"; Office "assume[s] an interactive desktop and user profile". Interactive-session agents are inside the supported envelope; services/agent-session accounts are not (**INF**: relevant to Windows "agent workspace" accounts).
- WinRT from PowerShell: works in Windows PowerShell 5.1 via `[Type,Namespace,ContentType=WindowsRuntime]`; **OBS** PowerShell 7 lost built-in WinRT projection (.NET 5+); workaround loads C#/WinRT DLLs from `Microsoft.Windows.SDK.NET.Ref`.
- Permission model: none beyond process integrity/ACLs. No per-app consent prompt. UAC elevation boundary applies.

**App Actions on Windows (Build 2025 -> docs updated 2026-10-14)**
- **OBS** MS Learn: "individual units of behavior that a Windows app can implement and register so that they can be accessed from other apps and experiences". Provider must have package identity (MSIX); actions declared in `registration.json` with typed entities (Document, Photo, Text...); invoked via COM (`IActionProvider`) or URI launch. Hosts enumerate with `Windows.AI.Actions.Hosting.ActionCatalog` (`GetAllActions`, `GetActionsForInputs`). Target OS 10.0.26100+, `WindowsSdkPackageVersion` 10.0.26100.75. Edge 137 lets Store-published PWAs expose actions.
- Access control: **OBS** `allowedAppInvokers` (AppUserModelIDs; `"*"` recommended; omitted/empty = no app can discover). So third-party hosts (including an MCP control plane with package identity) can invoke actions only if the provider opted in.
- Status: Hosting namespace page carries a prerelease warning; MS Learn overview still prerelease as of 2026-10-14 update (**OBS**). GA not confirmed.

**MCP on Windows / On-Device Agent Registry (ODR)**
- **OBS** MS Learn (ms.date 2025-12-06, updated 2026-06-04): ODR is "a secure, manageable interface to discover and use agent connectors from local apps and remote servers using MCP"; `odr.exe mcp list|add|remove|run|configure`; default connectors include File Explorer and Windows Settings; hosts listed: VS / VS Code Copilot agent mode. Host quickstart: Windows build 26220.7262+, host "with package identity" (not enforced in preview, will be in stable); host shells out to `odr.exe list`, parses JSON `manifest.server.mcp_config.command/args`, and connects with a standard stdio MCP client.
- Containment (**OBS**): registered MSIX servers "run in a separate Windows session using a separate agent user account"; no access to user files, settings, registry, credentials, or "apps and windows the user is using"; user-file access granted per *host* via known-folder capabilities and a prompt; MCPB / unpackaged servers cannot be contained; Settings toggle `System > Advanced > AI components > Reduce protections for agent connectors`. Insider public preview via build 26220.7344 (Dev/Beta, Nov 2025), initially Copilot+ devices (press). GA: not found as of 2026-11 (**unknown**).
- Agent Launchers (**OBS**): registry of interactive agents built on App Actions (`agentName`, `prompt` inputs); explicitly "not for background services or silent automation".
- Copilot Actions / agent workspace (**OBS** Windows Experience blog 2025-10-16; Insider rollout 2025-11-17): agent gets "a separate standard account", its own desktop, limited to Documents/Downloads/Desktop/Pictures unless granted more; drives apps by vision ("clicks, types and scrolls like a human would"), not MCP; disabled by default under `Settings > System > AI components > Agent tools > Experimental agentic features`; not in EEA; third-party agents "in private preview soon". Reports put broad rollout at 26H2 (borncity, plan not confirmation).

**Synthetic input**
- **OBS** `SendInput` docs: "subject to UIPI. Applications are permitted to inject input only into applications that are at an equal or lesser integrity level"; "neither GetLastError nor the return value will indicate the failure was caused by UIPI blocking"; does not reset keyboard state. UIAccess (signed, installed in secure location) lifts the restriction (**OBS** UAC policy page).
- **OBS** AutoHotkey v2 `Send`: SendInput "faster and generally more reliable than SendEvent and SendPlay", buffers physical input during send; "Send may have no effect if the active window is running with administrative privileges and the script is not" (UIPI); Ctrl+Alt+Del cannot be simulated. `*uiAccess` launch flag requires the UIA build (community gist).

### 3.3 Linux (Wayland-first)

- **D-Bus session bus**: the general IPC. Apps expose whatever they choose (media players via MPRIS, GNOME apps' `org.gtk.Actions`, KDE apps via KDBusAddons). No central consent model; any same-user process can call. Coverage is per-app and **unknown** in aggregate.
- **GNOME Shell**: **OBS** `org.gnome.Shell.Eval` restricted since GNOME 41 to `unsafe-mode` ("unarguably the most sensitive D-Bus method we expose"); downstream (AlmaLinux/RHEL) patch restricts D-Bus callers further; community extensions (unsafe-mode-menu, eval-gjs) re-enable it with explicit security warnings. Practical consequence: window management from outside the shell needs an extension exposing its own D-Bus API.
- **KDE KWin**: **OBS** `org.kde.KWin /Scripting` `loadScript` / `loadScriptFromText` + `start()` (Plasma 6 requires `start()`); scripts live until KWin restart; overloaded methods confuse pydbus (KDE bug 494716). wdotool's KDE backend generates JS and loads it this way.
- **xdg-desktop-portal** (**OBS** spec pages):
  - `RemoteDesktop` v2: `CreateSession` -> `SelectDevices(types)` -> `Start` (dialog "letting the user select what to share") -> `NotifyPointerMotion`/`NotifyKeyboardKeycode` or `ConnectToEIS` (recommended; once used, "input events must be sent exclusively via the EIS connection"). `persist_mode` 0/1/2; `restore_token` single-use, re-issued per Start; if restore fails "the user will be prompted normally". Can be combined with ScreenCast in the same session; persistence then managed only by RemoteDesktop.
  - `InputCapture` v2: receive input at pointer barriers (for input-leap style sharing); "The compositor is always in control of input capturing and may filter events or stop capturing at any time". It is capture, not emulation.
  - `GlobalShortcuts` v2: `CreateSession`, `BindShortcuts` (once per session; portal "typically" shows a config dialog), `ListShortcuts`, `ConfigureShortcuts` (v2), signals `Activated`/`Deactivated`.
  - Backend support: GNOME 46+ and Plasma 6 implement RemoteDesktop with libei (**OBS** wdotool README); Plasma 6.3 added permanent Remote Desktop permission (KDE bug 499869); xdg-desktop-portal-gnome 46.0 InputCapture barrier bug (ashpd source comment); Hyprland portal 1.3.11 lacked RemoteDesktop at time of wdotool README. GNOME 48/49 persist behaviour: **unknown** (not verified).
- **libei** (EI/EIS): the emulated-input protocol behind `ConnectToEIS`; project page blocked by bot protection in this session, details taken from wdotool/python-libei READMEs (**OBS**: "not a way around user consent", "input only").
- **Tools**: xdotool (X11 only; "does not work on Wayland at all" per wdotool README); ydotool (AGPL-3.0; `uinput` via `ydotoold`, needs `/dev/uinput` access "usually root"; no focus/window awareness; layout mismatches); wtype / wlrctl (wlroots `virtual-keyboard` / `virtual-pointer` protocols; not on GNOME/KDE); wdotool (per-compositor backend: wlroots protocols, libei via portal on KDE/GNOME, KWin script for window ops). Which one works is a function of compositor, not distro.

### 3.4 URL schemes / deep links / intents

- macOS/iOS: `CFBundleURLTypes`; x-callback-url convention adds `x-success`/`x-error`/`x-cancel` round-trips (**OBS** site tagline only; spec page 404 in this session). Fire-and-forget, no return value without callback, no auth beyond "any process may open".
- Windows: protocol handlers in registry; also the App Actions URI-launch provider path (**OBS**: "some more advanced scenarios, like streaming text responses from an action, aren't supported" via URI).
- Android: `adb shell am start -a android.intent.action.VIEW ...` (**OBS**); Google's 2026 "Android CLI" ships as a plugin for Claude Code/Codex with `install`, `run`, `screen capture`, `layout`, natural-language "Journeys" (**OBS**, page updated 2026-10-28) - an OS vendor explicitly packaging device control for coding agents.

---

## 4. Representative tools

| Name | URL | License | One line |
|---|---|---|---|
| `osascript` / AppleScript / JXA | https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/ | Apple OS component | Run OSA scripts against any app with an `.sdef`. |
| `shortcuts` CLI | man page mirror: https://keith.github.io/xcode-man-pages/shortcuts.1.html | Apple OS component | Run/list/view/sign Shortcuts, stdin/stdout piping; the only CLI into App Intents. |
| intents-mcp | https://github.com/VladUZH/intents-mcp | MIT | Exposes Mac App Intents as MCP tools by wrapping them in signed Shortcuts. |
| Scripting Bridge | https://www.macosxautomation.com/applescript/features/scriptingbridge.html | Apple framework | ObjC/Swift proxies over Apple Events (status 2025: unknown). |
| PowerShell `New-Object -ComObject` | https://learn.microsoft.com/en-us/powershell/scripting/samples/creating-.net-and-com-objects--new-object- | MIT (PowerShell 7) | COM automation of Office/Shell/WSH from scripts. |
| App Actions on Windows | https://learn.microsoft.com/en-us/windows/ai/app-actions/ | Windows SDK (prerelease) | Apps register typed actions; hosts call `ActionCatalog`. |
| MCP on Windows / `odr.exe` | https://learn.microsoft.com/en-us/windows/ai/mcp/overview | Windows component (preview) | On-device registry of MCP "agent connectors", contained in an agent session. |
| mcp-on-windows-samples | https://github.com/microsoft/mcp-on-windows-samples | MIT (**INF**, not verified) | Reference host that shells to `odr.exe list` and connects over stdio. |
| AutoHotkey v2 | https://www.autohotkey.com/docs/v2/lib/Send.htm | GPL-2.0 (**INF**, not re-verified) | Scripting with Send modes, hotkeys, window ops; UIPI-limited. |
| `SendInput` | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput | Win32 API | Serial synthetic keyboard/mouse injection at same-or-lower integrity. |
| xdg-desktop-portal (RemoteDesktop/ScreenCast/GlobalShortcuts/InputCapture) | https://flatpak.github.io/xdg-desktop-portal/docs/ | LGPL-2.1 | Consent-gated input emulation and capture on Wayland. |
| libei | https://gitlab.freedesktop.org/libinput/libei | MIT (**INF**, project page blocked) | Emulated-input protocol used by portal `ConnectToEIS`. |
| ydotool | https://github.com/ReimuNotMoe/ydotool | AGPL-3.0 | `uinput`-based typing/clicking that works on any display server; needs root/udev. |
| wtype / wlrctl | https://github.com/atx/wtype ; https://git.sr.ht/~brocellous/wlrctl | MIT | wlroots virtual-keyboard/pointer injection (Sway/Hyprland/river). |
| wdotool | https://github.com/cushycush/wdotool | **unknown** | Multi-backend Wayland xdotool replacement (wlroots, libei, KWin scripting). |
| xdotool | https://github.com/jordansissel/xdotool | BSD-3 | X11 XTEST input + window ops; dead on pure Wayland. |
| KWin Scripting D-Bus | https://develop.kde.org/docs/plasma/kwin/api/ | GPL (KWin) | Load JS into the compositor for window queries/moves. |
| Android CLI (agents) | https://developer.android.com/tools/agents | **unknown** | Google's agent plugin for device install/run/screen capture/layout. |

---

## 5. Strengths

- **Structured, semantic access.** Object models and intents return typed data (window titles, document properties, entities) instead of pixels; actions are idempotent and verifiable by re-reading state.
- **No reverse engineering, no injection.** Everything runs through interfaces the app vendor shipped and supports; it sits cleanly inside "software you own and are authorized to automate".
- **Audit and consent are first-class on the newer stacks.** TCC per-pair consent; Windows ODR containment with "separate agent user account" and per-host folder grants; portal `restore_token`s that the user can revoke.
- **Low latency and headless-capable** where it exists: Apple Events and COM calls are sub-ms to tens of ms; `shortcuts run` and D-Bus calls are process-local.
- **Vendor momentum (2025-26).** Apple (App Intents in Spotlight, schema domains for Siri), Microsoft (App Actions, ODR/MCP, Agent Launchers) and Google (Android CLI for agents) are all shipping agent-oriented action registries; building on them means improving coverage over time without changes to the control plane.
- **Composability.** Shortcuts and App Actions let the agent chain multiple apps' actions in a single declarative unit; PowerShell can combine COM, WinRT and .NET.

## 6. Failure modes

- **Coverage gaps.** The majority of apps expose nothing: Electron/Chromium-based and cross-platform toolkits generally have no `.sdef`, no COM object model, no D-Bus interface (**INF**; no quantitative survey found). Only 8/50 third-party Mac apps shipped App Intents on one machine (**OBS**). App Actions require MSIX + developer opt-in (`allowedAppInvokers`).
- **Permission friction and prompt storms.** macOS prompts once per (sender, target) pair and again when the sender's signature changes; `shortcuts` wrappers must be installed by hand ("Add Shortcut") and may re-prompt on upgrade (**OBS**). Portal sessions prompt per Start unless `persist_mode` 2 is honoured by the backend. Headless `shortcuts run` "just waits" on an unanswered prompt (third-party guide).
- **Silent failures.** `SendInput` blocked by UIPI returns no error (**OBS**). Apple Events on Tahoe time out (-1712) or return misleading -600 (**OBS**, community). `uinput` injection lands wherever focus is, with no acknowledgement.
- **Vendor churn.** Apple: "reserves the right to alter the Scripting Dictionary support for its applications at any time" (**OBS**). Microsoft: App Actions/ODR APIs are prerelease and "might change substantially"; package-identity requirement for hosts will be enforced at stable (**OBS**).
- **Session/identity mismatch.** Windows contained MCP servers run in a different session and cannot see "apps and windows the user is using" (**OBS**); Office automation from a non-interactive account is unsupported (**OBS** KB 257757). Linux portals need a running compositor session; D-Bus session bus address must be discovered for a detached agent.
- **Compositor fragmentation on Wayland.** Tool choice depends on compositor protocol support (wlroots vs libei vs KWin scripting); GNOME blocks shell `Eval` and lacks wlroots protocols; Hyprland's portal lacked RemoteDesktop (**OBS**, dated).
- **Scripting-language ergonomics.** AppleScript error reporting and dictionary quality vary widely; JXA documentation is thin; COM late binding fails at runtime with HRESULTs.
- **Shortcut/OSA sandboxing.** Shortcuts' "Run AppleScript"/"Run Shell Script" actions are gated by an "Allow Running Scripts" setting (**INF**, not re-verified this session because search budget ran out).

## 7. Trade-offs vs other technique families

- **vs accessibility trees (AX / UIA / AT-SPI).** Accessibility works on nearly every app (including Electron) but returns UI structure, not domain objects, and depends on layout. OS automation APIs are narrower but semantically richer and faster. REA's `observe_native_ui` / `capture_native_ui_scenario` (**OBS** `docs/native-investigation.md` "Native desktop observation": AX child-index paths, "No global event injection is used", fails closed on ambiguous geometry) is the accessibility-side counterpart; REA has no AppleScript/COM/D-Bus driver at all (grep for `osascript|AppleScript|UIAutomation|dbus` in `src/` and `native/` returned no automation code, only test fixtures and web accessibility capture).
- **vs vision + synthetic input (computer-use).** Universal but slow, costly, brittle to theme/scale, and least auditable. Microsoft's own Copilot Actions chose this route for breadth (**OBS**: "clicks, types and scrolls like a human would") while simultaneously building ODR/App Actions for depth - evidence that the two are complementary, not alternatives.
- **vs in-process instrumentation (Frida, DLL injection, REA's dynamic tools).** Maximum reach but crosses the "no RE / no security-control evasion" boundary for most third-party targets, trips anti-tamper, and breaks per build. OS automation APIs never require code signing waivers, SIP changes, or disabling AMFI (intents-mcp explicitly: "does not use private frameworks, change SIP or AMFI").
- **vs browser automation (CDP/WebDriver).** For web apps, CDP gives far richer control than URL schemes or AX; for native apps it is irrelevant. REA's browser scenario stack is the web analogue.
- **vs file/db/protocol-level automation.** Editing an app's files or talking to its backend API bypasses UI permission systems entirely but risks corrupting state the running app owns; Apple Events/COM keep the app in the loop.

## 8. Integration notes for an MCP control plane

**Process model**
- macOS: run one long-lived helper per OS session, codesigned with a stable Developer ID and bundled `NSAppleEventsUsageDescription`, so TCC consent attaches to *one* identity instead of to whatever terminal spawned the server. Spawn `osascript -l JavaScript` / `shortcuts run` as children of that helper. Expect first-use prompts per target app; surface them to the agent as `permission_required` rather than retrying. Consider a `preflight(target_bundle_id)` tool that sends a harmless event (e.g. `name of application`) to trigger the prompt early.
- Windows: COM/PowerShell can live in-process (STA thread per COM app). For App Actions and ODR the host must eventually have **package identity** (MSIX) and run in the *user's* interactive session; a contained MCP server (agent session) cannot see user windows (**OBS**). Design two tiers: an MSIX-packaged host that owns identity and consent, and worker processes for COM/SendInput. For SendInput against elevated windows either skip (detect integrity level via `GetTokenInformation`) or run a UIAccess-signed helper from `Program Files`.
- Linux: one helper per graphical session with `DBUS_SESSION_BUS_ADDRESS`/`WAYLAND_DISPLAY`; negotiate RemoteDesktop+ScreenCast as a *single* portal session, store `restore_token` (rotate on every Start), fall back to wlroots protocols or KWin scripting by detected compositor. Avoid `uinput` unless the user explicitly installs a udev rule; it defeats portal consent.

**Auth / permission prompts**
- Model permissions as data: `{platform, mechanism, grantor, scope, persistence}` - e.g. `{macos, AppleEvents, TCC, (helper->Safari), until-revoked}`, `{linux, RemoteDesktop, portal, keyboard+pointer, restore_token}`, `{windows, ODR, user, host-folder-grant, session}`. Report current grants via a resource so the agent can plan around them.
- Never attempt to auto-click consent dialogs; both TCC and portals treat that as the attack they exist to stop.

**Latency and state**
- Apple Events/COM: tens of ms; D-Bus: ~ms; `shortcuts run`: hundreds of ms to seconds (process launch + Shortcuts runtime); portal session start: user-paced. Cache dictionaries (`sdef`), type libraries, `ActionCatalog` and `odr.exe mcp list` results with invalidation on app install/update.
- Keep a per-target "capability card": discovered interfaces (sdef classes, COM ProgIDs, App Intents, App Actions, D-Bus names, URL schemes) so the planner picks the richest available surface and degrades to accessibility/vision only when none exists.

**Tool surface suggestions**
- `discover_automation_surfaces(app)`, `run_osa(script, lang)`, `run_shortcut(name, input)`, `com_invoke(progid, path, args)`, `app_action_invoke(id, entities)`, `dbus_call(bus, dest, path, iface, method, args)`, `portal_input_session(start|key|pointer|stop)`, `open_url(scheme_url, wait_for_callback)`. Each returns structured results plus `permission_state`.

## 9. Authorization and legal notes

- **OS-level consent is the authorization boundary.** TCC Automation, Windows UIPI/UAC, and portal dialogs encode the OS vendor's view of what a user must approve. Working *through* them (including MDM PPPC pre-approval on managed Macs) is within the project's stated boundary; disabling SIP/AMFI, enabling GNOME `unsafe-mode` by default, or shipping `uinput` access to bypass portal prompts is not.
- **Automating software you own is normally fine**, and these APIs exist for that purpose. Exceptions are contractual, not technical:
  - **OBS** Steam Subscriber Agreement 4.C: "You may not use any form of scripts, bots, macros, or other non-human-controlled systems" (in the context of gameplay stats/rewards); 4.B bars cheats/"unauthorized third-party software"; 4.D allows automated detection and account action.
  - **OBS** Microsoft Services Agreement, Code of Conduct 3.a: don't engage in "automating inauthentic activity", "Don't circumvent any restrictions on access to, usage, or availability of the Services"; 14.s restricts scraping of and reverse engineering of AI services. These govern Microsoft's *online services*, not local Windows API use.
  - Many games and anti-cheat systems treat synthetic input (AutoHotkey, SendInput, uinput) as cheating regardless of intent (**OBS** ESO forum staff statement; **INF** generally). The control plane should let users exclude processes.
  - Office automation on a server to service other clients runs into Office licensing (**OBS** KB 257757: clients must themselves have licensed Office).
- **Microsoft's responsible-AI note** for App Actions providers places content-moderation responsibility on the action author, and `contentAgeRating` exists in the JSON schema (**OBS**).
- **Data-flow disclosure.** `shortcuts sign` sends wrapper shortcuts to Apple via the user's iCloud account (**OBS** intents-mcp); Shortcuts "Use Model" may route data to Private Cloud Compute or ChatGPT (**OBS** WWDC25-260). Surface these in the control plane's consent UI.
- General legal framing (**INF**, not legal advice): in the US, CFAA exposure after *Van Buren v. United States* (2021) turns on access without authorization rather than ToS breach alone, but ToS violations can still ground contract/account actions; EU/UK positions differ. Nothing in this family touches DRM/TPM circumvention.

## 10. Open questions

1. Has Apple shipped any MCP (or MCP-like) bridge from App Intents to third-party agents in macOS 26.x/27, or is MCP confined to Xcode's `mcpbridge`? WWDC26 transcripts are silent; blogs conflict. Needs Apple release notes.
2. Windows ODR / App Actions GA date and whether non-Microsoft hosts (e.g. an open-source MCP control plane) will be admitted when package identity becomes mandatory.
3. Quantitative scripting coverage: what fraction of top-N Mac apps ship `.sdef`/App Intents; of top-N Windows apps a COM type library or App Action; of Linux desktop apps a public D-Bus API. No survey found; could be measured with a scan script (bundle `OSAScriptingDefinition`, `extract.actionsdata`, registry `TypeLib`, `busctl --user list`).
4. Does xdg-desktop-portal-gnome 48/49 honour `persist_mode=2` for RemoteDesktop, and has mutter's libei input-capture bug been fixed?
5. Root cause and fix status of macOS 26 Apple Event timeouts (FB20174869); does it affect `shortcuts run` paths too?
6. Shortcuts' "Allow Running Scripts" and per-shortcut privacy prompts: exact scope and whether they can be pre-approved via MDM (not verified this session).
7. Exact Apple PPPC semantics for `AppleEvents` (Allow via MDM?) - Apple page body not retrievable via fetch; verify from the Device Management docs.
8. Android CLI "Journeys"/agents plugin licensing and whether it can target production (non-debug) apps the user owns.

## 11. Sources

Apple
- WWDC25 260 "Develop for Shortcuts and Spotlight with App Intents": https://developer.apple.com/videos/play/wwdc2025/260/
- WWDC26 240 "Build intelligent Siri experiences with App Schemas": https://developer.apple.com/videos/play/wwdc2026/240/
- WWDC26 343 "Explore advanced App Intents features for Siri and Apple Intelligence": https://developer.apple.com/videos/play/wwdc2026/343/
- App Sandbox Temporary Exception Entitlements (scripting-targets / apple-events): https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/AppSandboxTemporaryExceptionEntitlements.html
- `shortcuts(1)` man page mirror: https://keith.github.io/xcode-man-pages/shortcuts.1.html
- Eclectic Light, "Privacy + Scripting = Problems" (TCC Automation model): https://eclecticlight.co/2018/09/07/privacy-scripting-problems/
- mjtsai.com, "Tahoe AppleScript Timeouts" (2025-09-17): https://mjtsai.com/blog/2025/09/17/
- MacScripter, "AppleScript and macOS Tahoe 26": https://www.macscripter.net/t/applescript-and-macos-tahoe-26/77695
- MacScripter, "Scripting Changes (or lack thereof) in macOS Tahoe": https://www.macscripter.net/t/scripting-changes-or-lack-thereof-in-macos-tahoe/77173
- DEVONthink forum, AppleScript regression / TCC hypothesis: https://discourse.devontechnologies.com/t/regression-in-applescript-performance-with-dt-4-2/86246/9
- Apple Community, TV app AppleScript broken in macOS 26: https://discussions.apple.com/thread/256150063
- AppleInsider on MCP in iOS 26.1 betas: https://appleinsider.com/articles/25/09/22/ios-26-could-get-a-major-ai-boost-with-the-model-context-protocol
- intents-mcp (App Intents -> MCP via Shortcuts): https://github.com/VladUZH/intents-mcp
- petegoldsmith.com, "The new Siri is a tool-calling agent built on App Intents" (2026-06-12, third-party): https://petegoldsmith.com/2026/06/12/2026-06-12-siri-on-macos-golden-gate/
- Cursor docs on Xcode 26.3 `mcpbridge`: https://cursor.com/docs/integrations/xcode.md

Microsoft
- App Actions on Windows overview: https://learn.microsoft.com/en-us/windows/ai/app-actions/
- Get started with App Actions (allowedAppInvokers, MSIX, SDK 26100): https://learn.microsoft.com/en-us/windows/ai/app-actions/actions-get-started
- Windows.AI.Actions.Hosting namespace: https://learn.microsoft.com/en-us/uwp/api/windows.ai.actions.hosting
- MCP on Windows overview: https://learn.microsoft.com/en-us/windows/ai/mcp/overview
- Quickstart: MCP host on Windows: https://learn.microsoft.com/en-us/windows/ai/mcp/quickstart-mcp-host
- Securely containing MCP servers on Windows: https://learn.microsoft.com/en-us/windows/ai/mcp/servers/mcp-containment
- odr.exe tool: https://learn.microsoft.com/en-us/windows/ai/mcp/odr-tool
- Agent Launchers on Windows: https://learn.microsoft.com/en-us/windows/ai/agent-launchers/
- Securing AI agents on Windows (agent workspace, 2025-10-16): https://blogs.windows.com/windowsexperience/2025/10/16/securing-ai-agents-on-windows/
- Windows Central, Build 2025 App Actions: https://www.windowscentral.com/software-apps/windows-11/windows-11-app-actions-api-announcement-build-2025
- Windows Central, native MCP announcement: https://windowscentral.com/software-apps/windows-11/windows-11-model-context-protocol-mcp-ai-announcement-build-2025
- Winaero, build 26220.7344 MCP preview: https://winaero.com/windows-11-build-26220-7344-dev-beta-introduces-model-context-protocol
- The Register, Insiders agentic preview (2025-12-08): https://www.theregister.com/2025/12/08/microsoft_windows_insiders_agentic_future/
- Thurrott, PWAs support App Actions: https://www.thurrott.com/?p=321620
- SendInput function: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- UAC: Only elevate UIAccess applications installed in secure locations: https://technet.microsoft.com/de-de/itpro/windows/keep-secure/user-account-control-only-elevate-uiaccess-applications-that-are-installed-in-secure-locations
- Creating .NET and COM objects (PowerShell): https://learn.microsoft.com/en-us/powershell/scripting/samples/creating-.net-and-com-objects--new-object-
- KB 257757 Considerations for server-side Automation of Office: https://support.microsoft.com/en-us/help/257757
- Old New Thing, clipboard history / WinRT in PowerShell 7: https://devblogs.microsoft.com/oldnewthing/20230303-00/?p=107894
- WinRT on PowerShell 7 via C#/WinRT DLLs: https://masaru-iritani.github.io/powershell/2022/02/26/windows-runtime-api-on-powershell-7.html
- AutoHotkey v2 Send: https://www.autohotkey.com/docs/v2/lib/Send.htm
- Microsoft Services Agreement: https://www.microsoft.com/en-us/servicesagreement

Linux / freedesktop
- RemoteDesktop portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html
- InputCapture portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.InputCapture.html
- GlobalShortcuts portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- libei: https://gitlab.freedesktop.org/libinput/libei (fetch blocked by Anubis in this session)
- python-libei README: https://pypi.org/project/python-libei/0.5.1/
- wdotool README (backend matrix): https://docs.rs/crate/wdotool/0.2.0/source/README.md
- ydotool: https://github.com/ReimuNotMoe/ydotool
- wtype: https://github.com/atx/wtype ; wlrctl: https://packages.debian.org/de/sid/wlrctl
- semicomplete, "Exploring the Fragmentation of Wayland, an xdotool adventure": https://semicomplete.com/blog/xdotool-and-exploring-wayland-fragmentation/
- GNOME Shell `shellDBus.js` / Eval unsafe-mode commit: https://lists.gnome.org/archives/commits-list/2021-September/msg01051.html
- AlmaLinux restrict-dbus-callers patch: https://git.almalinux.org/rpms/gnome-shell/src/branch/c9s/restrict-dbus-callers.patch
- unsafe-mode-menu extension: https://github.com/linushdot/unsafe-mode-menu
- KWin scripting API: https://develop.kde.org/docs/plasma/kwin/api
- KDE bug 494716 (overloaded Scripting D-Bus methods): https://bugs.kde.org/show_bug.cgi?id=494716
- KDE bug 499869 (permanent Remote Desktop permission, Plasma 6.3): https://bugs.kde.org/show_bug.cgi?id=499869
- ashpd input_capture.rs (GNOME 46.0 barrier bug note): https://docs.rs/crate/ashpd/latest/source/src/desktop/input_capture.rs

Other
- x-callback-url: https://x-callback-url.com/ (spec page 404 at fetch time)
- Android adb (`am start`): https://developer.android.com/tools/adb
- Android CLI for agents: https://developer.android.com/tools/agents
- Steam Subscriber Agreement: https://store.steampowered.com/subscriber_agreement/
- ESO forum staff statement on macros: https://forums.elderscrollsonline.com/en/discussion/comment/3644879
- REA checkout: `docs/native-investigation.md` ("Native desktop observation" section), `docs/desktop-applications.md`, `src/domain/native/nativeUiObservation.ts` (local path above)
