# Survey: CLI / IPC / Protocol Wrapping as an Agent Control Interface

Technique family: `cli-ipc-protocol-wrapping`
Date of research: 2026-11-04. Web facts marked with dates where time-sensitive.
Legend: **OBSERVED** = read on a primary page or in source; **INFER** = my inference; **UNKNOWN** = not verified.

---

## Summary

- The family covers every way an agent can drive existing software *without* touching its pixels or widget tree: wrapping its CLI/TUI in a pseudo-terminal, speaking a protocol it already exposes (LSP, DAP, CDP, WebDriver BiDi, MCP, Jupyter, mpv JSON IPC, obs-websocket, GDB/MI, OSC, D-Bus, COM, Apple events), hitting a local HTTP/socket admin API, dropping files into a watch folder, or launching the program in a documented headless mode.
- Where such an interface exists it is almost always the most reliable, cheapest, and most observable channel: typed request/response, explicit events, stable identifiers, no OCR, no coordinate guessing. The cost is that coverage is app-by-app: each program exposes a different surface (or none), with its own discovery, lifecycle, and auth story.
- Maturity varies widely by sub-technique. PTY wrapping (expect 1990 → pexpect → node-pty + @xterm/headless) and the "big" protocols (LSP, DAP, CDP, BiDi, Jupyter, D-Bus, COM, Apple events) are mature and well documented. App-specific IPC (mpv, OBS, VLC, REAPER OSC, Blender/Unity/Unreal) is mature but idiosyncratic. MCP-shaped adapters over these channels (terminal-mcp, chrome-devtools-mcp, playwright-mcp, lsp-mcp, blender-mcp, unity-mcp) are a 2025–2026 wave, mostly MIT/Apache, mostly single-maintainer except the vendor ones (Google, Microsoft).
- Platform spread: PTY and Unix sockets work on Linux/macOS natively and on Windows via ConPTY / named pipes; D-Bus is Linux-centric; Apple events/XPC are macOS-only; COM is Windows-only. The protocols over TCP/WebSocket/stdio are cross-platform.
- The biggest open problem for a "control plane" is not transport but *discovery + normalization*: finding what a given installed app exposes, and projecting dozens of unrelated schemas into one agent-facing vocabulary of sessions, observations, and actions.

---

## What it gives an agent

- **Structured observation.** Protocols hand back typed JSON (LSP diagnostics, DAP variables, CDP DOM/accessibility tree, mpv `property-change`, OBS events, D-Bus properties). A PTY wrapper plus a headless terminal emulator gives a cell-accurate screen buffer and cursor rather than a raw byte stream (OBSERVED: terminal-mcp `getContent`/`takeScreenshot` text format "returns JSON with plain text, cursor position, and dimensions"; REA `TerminalRenderer` uses `@xterm/headless` + `@xterm/addon-serialize` to produce "deterministic frames" — `rea-src/src/process/capture/TerminalRenderer.ts:11-16,33-37`).
- **Deterministic actuation.** Commands name objects by stable ID (tmux `%pane`, `@window`, `$session`; CDP `targetId`/`sessionId`; D-Bus object paths; Unreal object paths `/path/PackageName.ObjectName`), not by screen position.
- **Event streams / subscriptions.** tmux `%output`, mpv `observe_property`, OBS `EventSubscription` bitmask, BiDi `session.subscribe`, Jupyter IOPub, D-Bus `PropertiesChanged`, LSP `$/progress`. An agent can wait on a condition instead of polling screenshots.
- **Headless execution** where the GUI is irrelevant: Blender `--background --python`, Unity `-batchmode -nographics -executeMethod`, Chrome `--headless`, mpv/VLC without a window.
- **Introspection of capability**: LSP/DAP `initialize` capability exchange, D-Bus `Introspect` XML, COM type libraries, Apple `sdef`, Unreal `remote/object/describe`, CDP `/json/protocol` (UNKNOWN whether still served at tip-of-tree; not re-verified), REAPER `.ReaperOSC` pattern files.

---

## Platforms & permission models

### Linux
- **PTY**: `forkpty(3)`; node-pty, pexpect, expect. No special permission beyond running as the user.
- **Unix domain sockets**: file permissions on the socket path are the auth. Discovery: `ss -xlp` (listening side only; the client side does not map cleanly to a process — OBSERVED in fraggod.net 2024 note), `lsof -U`, `/proc/net/unix` + `/proc/<pid>/fd`. `lsof` "may hang on unresponsive sockets" (OBSERVED, same source).
- **D-Bus**: session bus address from `DBUS_SESSION_BUS_ADDRESS` (fallback `autolaunch:` / X root-window property); auth is SASL `EXTERNAL` using Unix credentials over the socket, `DBUS_COOKIE_SHA1` fallback, `ANONYMOUS` must not be accepted by a bus (OBSERVED, dbus-specification). Discovery: `org.freedesktop.DBus.ListNames`, `ListActivatableNames`, `GetNameOwner`, `GetConnectionUnixProcessID`, `GetConnectionCredentials`; per-object `org.freedesktop.DBus.Introspectable.Introspect` returns XML (node/interface/method/signal/property/arg with DTD) (OBSERVED). Effective permission = same UID on the session bus; system bus gated by policy XML (INFER from spec's bus-policy section, not fully re-read).
- **procfs**: REA reads `/proc/<pid>/stat`/`status` for sampling (`rea-src/src/process/LinuxProcessStat.ts`, `LinuxProcessStatus.ts`) — illustrates a sampler that an IPC discovery layer could reuse.

### macOS
- **PTY**: same as Linux (node-pty, pexpect).
- **Apple events / scripting dictionaries**: scriptable apps set `NSAppleScriptEnabled` and `OSAScriptingDefinition` in `Info.plist`; `sdef /Applications/X.app` dumps the dictionary (OBSERVED: sdef(1) man page says it extracts an app's sdef, converting legacy `aete` and Cocoa scriptSuite/scriptTerminology if needed). Since 10.14 the *sender* needs TCC "Automation" consent per target app; a hardened-runtime sender needs `com.apple.security.automation.apple-events` and `NSAppleEventsUsageDescription`. (Apple pages fetched returned title only — the above is general knowledge echoed by the fetch tool, so mark as **PLAUSIBLE, not re-verified on apple.com this session**.)
- **XPC / Mach services**: bundled `.xpc` services are private to the embedding app; launchd Mach services are in a shared bootstrap namespace and *should* verify peers by code-signing requirement / audit token (OBSERVED only as tool-summarized general knowledge; apple.com page body not retrievable). Discovery: `launchctl print gui/<uid>` lists services/endpoints; `launchctl bslist [PID]` lists Mach bootstrap services visible to a PID; `launchctl print` output is "only intended for debugging ... should not be relied upon in production" (OBSERVED, launchctl man page via search). `lsmp -p <pid>` (needs sudo) for Mach ports (third-party guide). **Bearing on scope boundary**: connecting to an app's XPC service that enforces a code-signing requirement is by design closed to third parties; attempting to bypass that is "evading security controls" and out of scope.
- **Unix sockets**: `lsof -U`, `netstat -f unix`.

### Windows
- **PTY**: node-pty uses ConPTY, requires Windows 10 1809+; winpty removed (OBSERVED, node-pty README). pexpect `spawn` is unavailable on Windows; `PopenSpawn` "is not a direct replacement" because programs behave differently without a TTY; wexpect/winpexpect are described as unmaintained (OBSERVED, pexpect docs). ConPTY caveats: soft-wrap vs logical newline are indistinguishable in the VT stream (OBSERVED, PuTTY dev comment via search); some keys lack unique VT encodings, addressed by `win32-input-mode` (OBSERVED, microsoft/terminal spec #4999). Passthrough mode: proposed, not confirmed shipped (UNKNOWN).
- **Named pipes**: enumerate with Sysinternals `PipeList`, `Get-ChildItem \\.\pipe\`, map to process with Process Explorer/`handle.exe` (search `\Device\NamedPipe\`), DACL with `accesschk` (OBSERVED via search summaries of MS docs). Many pipes use random GUID names so enumeration alone may not identify the app's endpoint.
- **COM automation**: type library (.tlb) is "a binary file that stores information about a COM or DCOM object's properties and methods in a form that is accessible to other applications at runtime"; lets a client "determine which interfaces an object supports, and invoke an object's interface methods" (OBSERVED, learn.microsoft.com MIDL page). Registration under `HKCR\CLSID`, `HKCR\TypeLib`, ProgIDs; running instances via the Running Object Table / `GetActiveObject` (INFER; the ROT page URL 404'd this session). OleView for inspection (general knowledge, not fetched).
- Permission model is the user's token; UAC integrity levels can block COM/pipes across elevation boundaries (INFER).

### Cross-platform protocols
- **stdio JSON-RPC** (LSP, DAP single-session mode, MCP stdio): the client spawns the server; permission = process ownership. LSP/DAP frame with `Content-Length` headers; MCP stdio uses newline-delimited JSON and forbids embedded newlines and non-protocol stdout (OBSERVED, MCP 2025-06-18 transports).
- **Local TCP/WebSocket** (CDP 9222, obs-websocket 4455, Unreal RC 30010/30020, blender-mcp 9876, Unity MCP bridges 8080/58888, VLC `--rc-host`): auth ranges from none (mpv, CDP, VLC rc) to password challenge (OBS SHA256 salt+challenge) to origin checks (CDP `--remote-allow-origins` from Chrome 111; MCP Streamable HTTP MUST validate `Origin`, SHOULD bind 127.0.0.1).
- **UDP** (OSC): no auth, no transport reliability.
- **ZeroMQ + HMAC** (Jupyter): key from the connection file; empty key disables signing (OBSERVED).

---

## Representative tools

| Tool | URL | License | One line |
|---|---|---|---|
| Expect (Don Libes) | https://core.tcl-lang.org/expect/index | not stated on page; NIST-hosted (public-domain status UNKNOWN from page) | Tcl-based original PTY automation; 5.45.4 (2018), "no longer maintained", successor under tcltk-depot (OBSERVED) |
| pexpect | https://pexpect.readthedocs.io/ ; https://pypi.org/project/pexpect/ | ISC | Python PTY `spawn`/`expect`; 4.9.0 (2023-11-25); no `spawn` on Windows |
| node-pty | https://github.com/microsoft/node-pty | MIT | `forkpty` bindings for Node; ConPTY on Windows; not thread-safe |
| @xterm/headless | https://github.com/xtermjs/xterm.js (addon-headless) | MIT (xterm.js; README fetch 404'd — license from general knowledge, PLAUSIBLE) | DOM-less terminal emulator to turn PTY bytes into a screen buffer |
| tmux control mode | https://github.com/tmux/tmux/wiki/Control-Mode | ISC (tmux) | `-CC` gives `%begin/%end/%error` framed command output and `%output` pane notifications |
| terminal-mcp (elleryfamilia) | https://github.com/elleryfamilia/terminal-mcp | MIT | node-pty + @xterm/headless + MCP SDK; `type`, `sendKey`, `getContent`, `takeScreenshot` (text/ansi/png), asciicast recording, sessions |
| terminal-control-mcp (wehnsdaefflae) | https://pypi.org/project/terminal-control-mcp/ | UNKNOWN | tmux-backed persistent sessions exposed over MCP, xterm.js browser view |
| mcp-tuikit | https://mcpservers.org/servers/dragoscirjan/mcp-tuikit | UNKNOWN | tmux 3.3a+ required; textual screen state + PNG screenshot of TUIs as MCP resource |
| Language Server Protocol 3.17 | https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/ | spec: CC-BY (INFER, same family as DAP) | JSON-RPC over stdio/socket with `Content-Length` framing; `initialize` capability exchange |
| Debug Adapter Protocol | https://microsoft.github.io/debug-adapter-protocol/overview | docs CC-BY 3.0 US (OBSERVED License.txt; file also contains CC-BY 4.0 text) | Adapter normalizes any debugger into one request/response/event schema; stdio or port |
| lsp-mcp (jonrad) / mcp-language-server (isaacphi) | https://github.com/jonrad/lsp-mcp ; https://github.com/isaacphi/mcp-language-server | UNKNOWN / UNKNOWN | MCP servers wrapping an LSP server (definitions, references, diagnostics, rename); created 2025 |
| Chrome DevTools Protocol | https://chromedevtools.github.io/devtools-protocol/ | BSD-3 (Chromium) | `--remote-debugging-port`/`--remote-debugging-pipe`; `/json/version`, `/json/list` discovery; WebSocket per target |
| chrome-devtools-mcp (Google) | https://github.com/ChromeDevTools/chrome-devtools-mcp | Apache-2.0 | Puppeteer-backed MCP server; `--headless`, `--isolated`, `--slim`; "exposes content of the browser instance to the MCP clients" |
| playwright-mcp (Microsoft) | https://github.com/microsoft/playwright-mcp | Apache-2.0 | Accessibility-tree snapshots "not pixel-based"; `--cdp-endpoint` to attach; "not a security boundary" |
| WebDriver BiDi | https://w3c.github.io/webdriver-bidi/ | W3C Document License | WebSocket, event subscriptions via `session.subscribe`; Editor's Draft dated 2026-11-03 (OBSERVED) |
| MCP spec 2025-06-18 | https://modelcontextprotocol.io/specification/2025-06-18/basic/transports | MIT (spec repo) | stdio + Streamable HTTP; `Mcp-Session-Id`; Origin validation MUST |
| Jupyter messaging | https://jupyter-client.readthedocs.io/en/stable/messaging.html | BSD-3 | 5 ZeroMQ channels; HMAC-SHA256 signed messages; connection file holds ports + key |
| mpv JSON IPC | https://github.com/mpv-player/mpv/blob/master/DOCS/man/ipc.rst | GPLv2+/LGPLv2.1+ | `--input-ipc-server=/tmp/mpvsocket` (`\\.\pipe\mpvsocket` on Windows); `{"command":[...],"request_id":N}`; "explicitly insecure" |
| obs-websocket v5 | https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md | GPLv2 | Bundled with OBS 28+; port 4455; Hello/Identify SHA256 salt+challenge auth; RequestBatch; event bitmask |
| VLC rc / http interfaces | https://wiki.videolan.org/Documentation:Modules/rc/ | GPLv2+ | `-I rc`, `--rc-host host:port`, `--rc-unix path`; plain-text command set (`help`/`longhelp`); http iface not covered on that page |
| GDB/MI | https://sourceware.org/gdb/current/onlinedocs/gdb.html/GDB_002fMI.html | GPLv3 | "line based machine oriented text interface", `--interpreter=mi`; REA negotiates mi3 then mi2 (`rea-src/src/gdb/GdbSessionManager.ts:773-780`) |
| Blender `--background --python` | https://docs.blender.org/manual/en/latest/advanced/command_line/arguments.html | GPL | Page fetch returned nav only this session; flags from general knowledge (PLAUSIBLE) |
| blender-mcp (ahujasid) | https://github.com/ahujasid/blender-mcp | UNKNOWN (MIT per general knowledge, not verified) | Addon starts JSON-over-TCP server on localhost:9876; MCP server bridges |
| Unity Editor CLI | https://docs.unity.com/en-us/engine/current/manual/unity-editor/command-line-arguments/editor | proprietary docs | `-batchmode -nographics -executeMethod Class.Method -quit -logFile -`; no documented remote control of a running editor |
| MCP for Unity (CoplayDev) / mcp-unity (CoderGamester) | https://github.com/CoplayDev/unity-mcp ; https://github.com/CoderGamester/mcp-unity | UNKNOWN | Editor-side HTTP (localhost:8080) or WebSocket server + MCP bridge; instance selection `Name@hash` |
| Unreal Remote Control API | https://dev.epicgames.com/documentation/en-us/unreal-engine/remote-control-api-http-reference-for-unreal-engine | Unreal EULA | Beta; `PUT remote/object/call`, `remote/object/property`, `remote/object/describe`, `remote/batch`; property access gated by UPROPERTY flags |
| REAPER OSC | https://www.reaper.fm/sdk/osc/osc.php | proprietary | Add OSC control surface in Preferences; `Default.ReaperOSC` pattern file; bind messages to actions |
| AbletonOSC | https://github.com/ideoforms/AbletonOSC | UNKNOWN | MIDI Remote Script exposing Live Object Model over OSC, listens 11000 / replies 11001; Live 11+ |
| OSC 1.0 spec | https://opensoundcontrol.stanford.edu/spec-1_0.html | n/a | Datagram-oriented message format; "1.1" never formally published |
| Sysinternals PipeList / Handle / AccessChk | https://learn.microsoft.com/sysinternals/downloads/pipelist | freeware (Sysinternals EULA) | Enumerate named pipes, find owning process, read DACLs |
| D-Bus spec + `busctl`/`gdbus`/`d-feet` | https://dbus.freedesktop.org/doc/dbus-specification.html | AFL/GPL (reference impl) | Introspection XML; ListNames; EXTERNAL auth |

---

## Strengths

- **Reliability.** No layout drift, no font/DPI/theme sensitivity, no race between paint and screenshot. Request/response correlation is explicit (`request_id` in mpv; `seq` in DAP; JSON-RPC `id`; tmux command numbers in `%begin`/`%end`; OBS `requestId`).
- **Observability without vision.** Text buffers and JSON are cheap in tokens versus screenshots; Playwright MCP's README explicitly argues accessibility snapshots are "lightweight and deterministic" and that you "can't act on a screenshot" (OBSERVED).
- **Capability negotiation exists** in the mature protocols (LSP/DAP `initialize`, OBS `rpcVersion` → `negotiatedRpcVersion`, MCP version negotiation, BiDi capabilities) so an adapter can degrade gracefully.
- **Batching and async**: OBS `RequestBatch` with `haltOnFailure`/`executionType`; Unreal `remote/batch`; mpv `"async": true`; tmux flow control (`pause-after`, `%pause`/`%continue`) — the primitives needed for an agent to issue multi-step actions atomically or stream large outputs safely.
- **Ownership is clear.** The agent usually *spawns* the target (stdio servers, PTY children, headless launches), so lifecycle, cleanup, and permission are straightforward. REA formalizes this as "owned" processes with a socket-backed lease (`rea-src/src/process/OwnedLease.ts:27-34`) and ownership checks via process tree (`ProcessOwnership*.ts`).
- **Reusable ecosystems.** Hundreds of LSP servers, dozens of DAP adapters, every Chromium-based app (including Electron apps via `--remote-debugging-port`/`--inspect`; REA's `PlaywrightElectronActiveFuses.ts:35` notes Electron fuses can disable these) speak the same protocol, so one adapter covers a class rather than one app.

---

## Failure modes

- **PTY/TUI parsing pitfalls** (OBSERVED, pexpect docs): `$` end-of-line matching "is useless", must match `\r\n`; minimal-match regex surprises; default 30 s `TIMEOUT`; `EOF` semantics vary by platform. Screen-scraping a TUI requires a real emulator (xterm headless) because cursor-addressed redraws make the raw stream meaningless; even then, soft wrap vs newline is ambiguous on ConPTY.
- **Non-UTF-8 and escaping**: tmux `%output` octal-escapes control bytes and "may not be valid UTF-8"; paused panes drop output so the client must `capture-pane` to resynchronize (OBSERVED).
- **Platform gaps**: no pexpect `spawn` on Windows; node-pty needs native build toolchains and is tied to Node/Electron ABI versions; node-pty not thread-safe (OBSERVED).
- **Interfaces that are off by default or vendor-gated**: Chrome 136+ ignores `--remote-debugging-port`/`--remote-debugging-pipe` for the default user-data-dir (post dated 2025-03-17) — automation must use a separate `--user-data-dir` or Chrome for Testing (OBSERVED). Chrome 111+ rejects DevTools WebSocket connections carrying an `Origin` header unless `--remote-allow-origins` matches (OBSERVED via Chromium commit/Selenium issue). Electron apps may have fuses that disable `--inspect`/`--remote-debugging-port`.
- **Undocumented or unstable surfaces**: Unreal RC is "Beta ... use caution when shipping"; GDB/MI "still under construction"; `launchctl print` output not to be relied on; CDP tip-of-tree domains change; REAPER OSC "minimal documentation". mpv text (non-JSON) commands return no reply at all.
- **Session/state leaks**: mpv observations die with the connection; LSP document sync requires the client to mirror every edit; DAP variable references are valid only while suspended; Unity allows one editor instance per project; Playwright persistent profile can be used by only one browser at a time.
- **Auth absent → anything on the host can drive the app**: mpv ("no authentication, no encryption"), VLC rc, OSC/UDP, CDP (profile-wide control, used for cookie theft per Google), MCP servers bound to 0.0.0.0. Shared-machine localhost exposure reported for ChromeDriver (2021 thread).
- **Discovery is lossy**: client-side Unix sockets do not map to processes via `ss -xp`; named pipes often have GUID names; `strings` on a binary to find endpoints is heuristic (no primary source found; treat as folk technique).
- **Headless ≠ same behaviour**: Unity `-nographics` cannot bake GI and "simulated input requires a focused window"; programs detect lack of a TTY and change output (pexpect docs).
- **Exploding tool surface**: every protocol has hundreds of methods (CDP has dozens of domains; LSP 3.17 is a 400k-character spec). Exposing them raw to an LLM blows the context budget; Google and Microsoft both ship reduced tool sets (`--slim`, curated Playwright tools).

---

## Trade-offs vs other technique families

- **vs GUI/accessibility-tree automation (AT-SPI, UIA, AX, computer-use)**: protocol wrapping is far more reliable and cheaper *when the interface exists*; GUI automation is universal but brittle. Many real apps expose *nothing*, so a control plane needs GUI automation as the fallback and protocol wrapping as the preferred path. Playwright MCP shows the hybrid: structured snapshot (accessibility tree via CDP) for reading, same channel for acting, screenshots only as a diagnostic.
- **vs binary instrumentation / hooking (Frida, DLL injection, REA's approach)**: hooking can create an interface where none exists, but it is invasive, version-fragile, anti-cheat/EDR-hostile, and sits close to the "evading security controls" boundary. Protocol wrapping only uses doors the vendor left open. REA itself prefers vendor protocols where available (CDP `/json/version` discovery in `rea-src/src/browser/CdpEndpoint.ts:121-126` and `src/inspector/V8InspectorEndpoint.ts:43-44`; GDB/MI; PTY capture) and treats foreign-code execution as a flagged capability (`docs/mcp-contracts.md:1254` "executesForeignCode").
- **vs vendor SDK / plugin development**: writing an in-app plugin (Blender addon, Unity editor package, Ableton Remote Script, OBS plugin) gives the richest access but requires per-app code running inside the app and the user installing it. Most "X-MCP" projects are exactly this: plugin inside + bridge outside. Protocol wrapping without a plugin only reaches what the vendor already shipped.
- **vs file-drop / watch-folder / CLI batch**: lowest coupling, trivially sandboxable, but one-shot and blind to app state; good for converters and renderers, useless for interactive sessions.
- **vs web/remote APIs**: not applicable to local-only desktop apps; local IPC has no network auth story, which is both simpler and riskier.

---

## Integration notes for an MCP control plane

Process model
- Three ownership shapes to model explicitly: (a) **spawned-and-owned** (stdio LSP/DAP/MCP servers, PTY children, headless Blender/Unity runs) — control plane owns lifecycle, can kill, must clean up; (b) **attached** (CDP to a running Chrome, mpv socket, OBS WebSocket, D-Bus name, COM running object) — control plane must not kill, must tolerate the app quitting, and must handle "someone else is also connected"; (c) **multiplexed** (tmux server, Jupyter kernel with multiple frontends) where the app itself arbitrates. REA's `OwnedLease` (socket-based lease with `LeaseOwner {runId, processId}`) and `ProcessOwnership*` modules are a concrete pattern for (a).
- Keep each adapter as its own process (crash isolation; node-pty is not thread-safe; LSP servers are heavy). The control plane holds a registry `{adapterId, target, transport, sessionIds[]}` and forwards.

Discovery pipeline (per app launch or on demand)
1. Static: inspect the install — `Info.plist` (`NSAppleScriptEnabled`, `OSAScriptingDefinition`) → `sdef`; Windows registry `HKCR\CLSID`/`TypeLib`/ProgID for the exe → type library; `.desktop` + D-Bus service files (`/usr/share/dbus-1/services`); known flags table (`--remote-debugging-port`, `--input-ipc-server`, `--interpreter=mi`, `-batchmode`). `strings`/`rg` on the binary for `localhost:`, `\\.\pipe\`, `.sock`, `ws://` is a heuristic last resort; label results unverified.
2. Dynamic: after launch, diff `ss -xlp`/`lsof -p <pid>`/`/proc/<pid>/fd`, `PipeList`+`handle`, `launchctl print`/`bslist`, `ListNames` on the session bus, `GET /json/version` on candidate ports. Record *evidence* (which probe found what) so the agent can judge confidence.
3. Negotiate: run the protocol's own capability handshake (LSP/DAP `initialize`, OBS `Hello`, MCP `initialize`, D-Bus `Introspect`, Unreal `remote/info`) and cache the result as the adapter's tool manifest.

Normalized schema (thin adapter per app)
- Common nouns: `Target` (app instance), `Session` (one protocol connection or PTY), `Observation` (text screen / JSON state / event), `Action` (typed command), `Capability` (what the session can do), `Evidence` (how we know). Each adapter maps native → common and exposes native escape hatches (`raw_call(method, params)`) for the long tail.
- For PTYs: expose `screen` (rows, cursor, dimensions, optional ANSI), `scrollback`, `send_text`, `send_key`, `wait_for(regex|idle|cursor)`, `resize`. terminal-mcp's tool set is a reasonable minimum; add explicit `wait_for` because agents otherwise spin.
- For event protocols: a `subscribe(filter)` that buffers into a bounded, cursor-addressable log the agent polls (`events_since(cursor)`), mirroring MCP Streamable HTTP's `Last-Event-ID` resumability.
- Keep tool count small per adapter (Google `--slim`, Playwright curated set); push the protocol's full method list into a `describe_capabilities` resource rather than N tools.

Auth / permission prompts
- The control plane should prompt once per *(app, capability class)*, not per call: e.g., "allow driving OBS via WebSocket (can start/stop streams)". Store the OBS password / Jupyter key / CDP user-data-dir in the control plane's secret store, never in tool arguments the model sees.
- Respect OS prompts: macOS TCC Automation prompts fire when the control plane first sends an Apple event to a given app; the plane process (not the agent) is the TCC subject, so it needs the entitlement/usage string and should surface the denial clearly.
- Bind any HTTP/WS surface the plane itself exposes to 127.0.0.1 and validate `Origin` (MCP spec MUST), since local apps like mpv/CDP have no auth and a DNS-rebinding web page could otherwise reach them through the plane.

Latency and state
- stdio/socket round trips are sub-millisecond to tens of ms; PTY observation needs a settle heuristic (REA uses "settlement" observation in `ProcessCaptureLifecycle.ts`; tmux offers `pause-after`). Expose `idle_ms` as a parameter.
- Persist per-session state the protocol requires: LSP open-document mirror, DAP suspended/resumed flag, mpv observer ids, OBS negotiated RPC version, CDP `sessionId` per target, tmux IDs (use `$`/`@`/`%` IDs not names — tmux wiki).
- Everything is single-host; for Windows/macOS/Linux parity, implement PTY via node-pty (one codebase) and treat pexpect as the Python alternative.

---

## Authorization & legal notes

- **In scope**: all channels here are vendor-provided interfaces (documented flags, sockets, protocols) or OS-level IPC on software the user runs. Using them is ordinary automation; no DRM or protection is touched.
- **Edge cases to flag in the project's boundary**:
  - CDP attach to a *default* Chrome profile is now blocked by Google specifically because it was used for cookie theft; a control plane should follow the vendor path (separate `--user-data-dir` / Chrome for Testing) rather than work around the restriction.
  - XPC/Mach services with code-signing requirements and Electron apps whose fuses disable `--inspect` are *deliberately closed*; the plane should report "not exposed" rather than inject or patch.
  - Enumerating named pipes / sockets / Mach services is read-only reconnaissance of the user's own machine; enumerating and connecting to endpoints of *other users'* sessions on a shared host (the ChromeDriver localhost case) is not authorized use.
  - Game-engine remote control (Unreal RC, Unity bridges) is editor tooling; using similar channels against a shipped game client with anti-cheat is a different activity and out of scope.
- **Licensing**: GPL components (mpv, OBS/obs-websocket, VLC, GDB, Blender) are driven over IPC, not linked, so no copyleft propagation; MIT/Apache/ISC for the wrapper libraries (node-pty MIT, pexpect ISC, chrome-devtools-mcp and playwright-mcp Apache-2.0, terminal-mcp MIT). Spec documents under CC-BY (DAP) / W3C license (BiDi). Sysinternals tools are freeware under their own EULA (redistribution limits — check before bundling).
- **Telemetry**: chrome-devtools-mcp sends usage statistics by default and may send trace URLs to CrUX; disable with `--no-usage-statistics` / `--no-performance-crux` in a privacy-conscious control plane (OBSERVED).
- **Exposure**: both Google and Microsoft MCP servers warn they expose browser content to the MCP client and are "not a security boundary"; the control plane inherits that and should say so in its own disclosure.

---

## Open questions

- Is there any cross-platform *discovery* library that unifies `ss`/`lsof`/`PipeList`/`launchctl`/D-Bus `ListNames` into one inventory of "IPC endpoints owned by PID X"? None found; likely needs to be written.
- How far can a *generic* PTY adapter go for TUIs (vim, htop, lazygit) before per-app semantic parsing is needed? terminal-mcp/mcp-tuikit give screen text; nothing surveyed gives widget-level semantics.
- Has ConPTY passthrough shipped, and does it change what a headless emulator sees on Windows? (UNKNOWN as of this session.)
- Which Apple docs confirm the exact current TCC/entitlement requirements for an agent process sending Apple events (fetches returned only titles)?
- Does CDP still serve `/json/protocol` and is there an official "stable 1.3" subset an adapter should pin to?
- How should the plane handle *multiple* agents/clients on single-connection protocols (mpv observers per connection, LSP one-client-per-server)? Proxying vs spawning per client.
- For file-drop/watch-folder interfaces no primary source was surveyed; worth a pass over common "hot folder" apps (Handbrake queue, Lightroom auto-import, Davinci watch folders).
- Blender command-line page could not be fetched (nav-only); verify `--background`, `--python`, `--python-expr`, `--python-exit-code` against the live manual.

---

## Sources

Primary docs / specs
- pexpect overview: https://pexpect.readthedocs.io/en/stable/overview.html ; PyPI: https://pypi.org/project/pexpect/
- Expect home: https://core.tcl-lang.org/expect/index
- node-pty: https://github.com/microsoft/node-pty
- tmux control mode wiki: https://github.com/tmux/tmux/wiki/Control-Mode
- MCP transports 2025-06-18: https://modelcontextprotocol.io/specification/2025-06-18/basic/transports
- LSP 3.17: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/
- DAP overview: https://microsoft.github.io/debug-adapter-protocol/overview ; license: https://github.com/microsoft/debug-adapter-protocol/blob/main/License.txt
- WebDriver BiDi ED (2026-11-03): https://w3c.github.io/webdriver-bidi/
- Chrome remote debugging (local server): https://developer.chrome.com/docs/devtools/remote-debugging/local-server
- Chrome 136 remote-debugging change (2025-03-17): https://developer.chrome.com/blog/remote-debugging-port
- Chromium `--remote-allow-origins` commit: https://github.com/chromium/chromium/commit/0154caeefc74530d5cb57ce71608beb1b77bca39 ; Selenium issue: https://github.com/SeleniumHQ/selenium/issues/11750
- CDP viewer / repo README: https://chromedevtools.github.io/devtools-protocol/ ; https://github.com/ChromeDevTools/devtools-protocol
- Jupyter messaging: https://jupyter-client.readthedocs.io/en/stable/messaging.html
- mpv JSON IPC: https://github.com/mpv-player/mpv/blob/master/DOCS/man/ipc.rst
- obs-websocket protocol v5: https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md
- VLC rc module: https://wiki.videolan.org/Documentation:Modules/rc/
- GDB/MI: https://sourceware.org/gdb/current/onlinedocs/gdb.html/GDB_002fMI.html
- Unity editor CLI: https://docs.unity.com/en-us/engine/current/manual/unity-editor/command-line-arguments/editor
- Unreal Remote Control HTTP reference: https://dev.epicgames.com/documentation/en-us/unreal-engine/remote-control-api-http-reference-for-unreal-engine
- Blender CLI args (fetch returned nav only): https://docs.blender.org/manual/en/latest/advanced/command_line/arguments.html
- REAPER OSC: https://www.reaper.fm/sdk/osc/osc.php
- OSC 1.0: https://opensoundcontrol.stanford.edu/spec-1_0.html
- D-Bus specification: https://dbus.freedesktop.org/doc/dbus-specification.html
- sdef(1): https://keith.github.io/xcode-man-pages/sdef.1.html
- Apple events entitlement (title only retrieved): https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.automation.apple-events
- XPC (title only retrieved): https://developer.apple.com/documentation/xpc
- COM type libraries: https://learn.microsoft.com/en-us/windows/win32/midl/com-dcom-and-type-libraries
- Sysinternals PipeList: https://learn.microsoft.com/sysinternals/downloads/pipelist
- launchctl man page (macOS): https://man.freebsd.org/cgi/man.cgi?manpath=macOS+26.4&query=launchctl&sektion=1
- Unix-socket discovery caveats: https://blog.fraggod.net/2024/08/06/list-connected-processes-for-unix-sockets-on-linux.html
- ConPTY issues/specs: https://github.com/microsoft/terminal/issues/1173 ; https://github.com/microsoft/terminal/issues/1985 ; https://github.com/microsoft/terminal/blob/main/doc/specs/%234999%20-%20Improved%20keyboard%20handling%20in%20Conpty.md

MCP-shaped adapters
- terminal-mcp: https://github.com/elleryfamilia/terminal-mcp
- node-terminal-mcp: https://glama.ai/mcp/servers/@hazzel-cn/node-terminal-mcp
- terminal-control-mcp: https://pypi.org/project/terminal-control-mcp/
- mcp-tuikit: https://mcpservers.org/servers/dragoscirjan/mcp-tuikit
- chrome-devtools-mcp: https://github.com/ChromeDevTools/chrome-devtools-mcp
- playwright-mcp: https://github.com/microsoft/playwright-mcp
- lsp-mcp: https://github.com/jonrad/lsp-mcp ; mcp-language-server: https://github.com/isaacphi/mcp-language-server
- blender-mcp: https://github.com/ahujasid/blender-mcp
- MCP for Unity: https://github.com/CoplayDev/unity-mcp ; mcp-unity: https://github.com/CoderGamester/mcp-unity ; Unity_MCP (ChiR24): https://github.com/ChiR24/Unity_MCP
- AbletonOSC: https://github.com/ideoforms/AbletonOSC ; NIME 2023 paper: https://nime.org/proc/nime2023_60/

REA checkout (read-only comparison), root `/tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src`
- `src/process/capture/ProcessHarness.ts:2,293` (`@lydell/node-pty` spawn)
- `src/process/capture/TerminalRenderer.ts:11-16,33-37,63` (`@xterm/headless` + `@xterm/addon-serialize` deterministic frames)
- `src/browser/CdpConnection.ts:1-50` (correlated CDP JSON-RPC over `ws`)
- `src/browser/CdpEndpoint.ts:121-126`, `src/inspector/V8InspectorEndpoint.ts:43-44` (`/json/version`, `/json/list` discovery)
- `src/browser/PlaywrightElectronActiveFuses.ts:35` (Electron fuses disabling `--inspect`/`--remote-debugging-port`)
- `src/gdb/GdbSessionManager.ts:497,773-780` (GDB/MI mi3→mi2 negotiation)
- `src/process/OwnedLease.ts:20-34` (socket-backed process lease / ownership)
- `package.json:357,362-363` (pinned `@lydell/node-pty` 1.1.0, `@xterm/headless` 6.0.0, `@xterm/addon-serialize` 0.14.0)
- `docs/mcp-contracts.md:1254` ("executesForeignCode" flagged capabilities)
