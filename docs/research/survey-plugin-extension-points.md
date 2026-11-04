# Survey: plugin / extension / scripting points as an agent control surface

Technique family: **plugin-extension-points**
Date of research: 2026-11-04. Inputs: web research (primary vendor docs where reachable) plus a read of the REA checkout at `/tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src` (read only; nothing executed).
Marking convention: **OBSERVED** = read directly in a primary doc or in source; **SECONDARY** = read in a third-party page or a search-engine summary; **INFER** = my reasoning; **UNKNOWN** = not established.

Note on coverage limits: the session's WebSearch budget ran out before I could verify (a) whether Microsoft ships an Office/Excel-desktop MCP server, (b) whether McNeel (Rhino) or Autodesk ship first-party MCP servers, (c) Unreal Python "Remote Execution" (UDP multicast) details, (d) the Emacs server manual (gnu.org returned HTTP 429 three times). Those are listed as UNKNOWN below rather than guessed.

---

## Summary

- Using an application's own extension surface (plugin API, embedded scripting language, remote-control socket, or first-party MCP server) is the highest-fidelity way to let an agent observe and drive software: the agent sees the app's object model (documents, layers, tracks, symbols) rather than pixels, and mutations go through the same code paths the GUI uses.
- Almost every category of professional desktop software has such a surface: IDEs (VS Code, JetBrains, Neovim, Emacs), creative tools (Blender, GIMP 3, Krita, Adobe UXP/ExtendScript, Figma), office suites (Office add-ins, VBA, Office Scripts), game engines (Unity, Unreal, Godot), DAWs (Reaper ReaScript; Ableton only via undocumented Remote Scripts; Audacity mod-script-pipe), CAD (FreeCAD, Fusion, Rhino), note apps (Obsidian), browsers (extensions + native messaging), and Electron apps generally (`--remote-debugging-port`).
- The dominant integration pattern is the one REA uses: a small **resident bridge** running inside the host process (loaded via the host's plugin/script mechanism) that opens a **local socket**, with an **out-of-process MCP server** that authenticates to it and translates MCP tool calls into host-API calls. Observed in REA for Hopper (`bridge/hopper_bridge.py`), Cutter (`bridge/cutter/rea_bridge.py`), Ghidra (`bridge/ghidra/ReaGhidraBridge.java`), and via upstream for IDA (ida-pro-mcp plugin on HTTP 13337).
- 2025–2026 trend: vendors increasingly ship this bridge themselves as a **first-party MCP server**: JetBrains (bundled since 2025.2), Unity (in-editor server, now deprecated in favour of a Unity CLI with an MCP mode), Epic Unreal (Experimental "Unreal MCP" plugin, loopback HTTP :8000), Blender Lab (add-on + `.mcpb` server, Blender 5.1+), Figma (remote `mcp.figma.com` plus desktop server), Adobe (hosted "Adobe for creativity" remote MCP, SECONDARY), Google Chrome DevTools MCP (public preview Sept 2025). Several of these are explicitly experimental/beta and have already changed shape within a year.
- Security models vary from "no sandbox, full user privileges" (VS Code, JetBrains, Blender, Obsidian, GIMP/Krita, Reaper, Neovim, Emacs) through manifest-declared permissions (Adobe UXP v5, Figma `networkAccess`, Office add-in permission levels, Chrome extension permissions, Blender extension manifest permissions) to store review + signing (VS Code Marketplace, JetBrains Marketplace, Chrome Web Store, Adobe Marketplace, Figma Community, Obsidian directory). For a user-installed private bridge most of these gates are bypassable by design (sideload / developer mode), which is the point for a control plane but also means the bridge is a full-privilege foothold.
- Main failure modes: API drift across host versions, threading constraints (host API must be called on the host's main/script thread), hosts that stop the bridge when idle or on project reload, plugin-store policies that forbid remote code or self-update, and EULA/ToS clauses that prohibit automation or that confine API use to internal/non-commercial purposes.

---

## What it gives an agent

- **Structured observation**: the host's document/object model (e.g. JetBrains `get_file_problems`, `get_symbol_info`; Figma frames/variables; Blender scene graph via `bpy`; Reaper tracks/items via `RPR_*`/`reaper.*`; IDA/Hopper/Ghidra functions, xrefs, pseudocode). No OCR, no coordinate guessing.
- **Semantic actuation**: operations named in domain terms (rename refactoring, spawn actor, apply material, add MIDI item, set bookmark) with the host's own validation and undo stack.
- **Events/subscriptions** where the API supports them (Office add-in events require `ReadWriteDocument`; Neovim `nvim_buf_attach`; VS Code `onDid*`; Krita/Qt signals).
- **Headless or background execution** for some hosts (Ghidra `analyzeHeadless -postScript`, Unreal `-run=pythonscript`, Godot `--headless --script`, FreeCAD `FreeCADCmd`, Blender `-b -P`), which lets a control plane do work without a visible UI.
- **Code execution inside the host** as a universal escape hatch: a bridge that accepts arbitrary Python/JS/Lua (Blender Lab's MCP server, community Ableton bridges, Emacs `--eval`, Neovim `nvim_exec_lua`) exposes the whole API surface with one tool. This is also the largest safety hazard (see Failure modes).

---

## Platforms & permission models

### IDEs / editors

| Host | Extension surface (OBSERVED unless noted) | Permission / isolation model | Agent-relevant notes |
|---|---|---|---|
| **VS Code** | Extension API (TypeScript, runs in extension host). MCP-specific: `vscode.lm.registerMcpServerDefinitionProvider` + `contributes.mcpServerDefinitionProviders` in `package.json`; definitions `McpStdioServerDefinition` / `McpHttpServerDefinition`; transports stdio, Streamable HTTP, legacy SSE. Alternative: Language Model Tools API for tools that need deep extension-API access or Marketplace distribution. ([MCP guide](https://code.visualstudio.com/api/extension-guides/mcp)) | "The extension host has the same permissions as VS Code itself" — extensions can read/write files, make network requests, run processes. Marketplace signs extensions at publish; VS Code verifies signature at install; malware scan + "clean room VM" dynamic analysis before publish; blocklist auto-uninstalls. Workspace Trust / Restricted Mode "can't prevent a malicious extension from executing code". ([runtime security](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security), [workspace trust](https://code.visualstudio.com/docs/editor/workspace-trust)) | MCP guide still says MCP support "is currently in preview" (date of that statement UNKNOWN). Proposed APIs cannot be published to Marketplace; VSIX sideload is the sanctioned route ([proposed API](https://code.visualstudio.com/api/advanced-topics/using-proposed-api)). A 2025 OX Security report found repackaged VSIX files could keep the "verified" badge; Cursor stated it does not verify extension signatures (SECONDARY). |
| **JetBrains IDEs** | Plugin SDK (Java/Kotlin). **Built-in MCP Server since 2025.2**, bundled plugin; Settings \| Tools \| MCP Server; transports SSE, stdio, HTTP stream; per-tool Enabled / Router-only checkboxes; tools span analysis, refactoring, run configs, debugger (`xdebug_*`), DB, VCS, terminal (`execute_terminal_command` requires confirmation unless "brave mode"). ([IDEA help](https://www.jetbrains.com/help/idea/mcp-server.html)) | "IDE does not restrict plugins using fine-grained permissions, nor does it isolate (sandbox) them." Marketplace: automated upload checks, Plugin Verifier (binary compatibility, not security), UI tests, manual review; "Verified Vendor" confirms identity only. ([plugin security](https://plugins.jetbrains.com/docs/marketplace/understanding-plugin-security.html)) | Enabling the server pops a consent dialog describing third-party access to open projects. Older `@jetbrains/mcp-proxy` reported deprecated (SECONDARY). `since-build`/`until-build` ranges + Plugin Verifier are the versioning contract ([build number ranges](https://plugins.jetbrains.com/docs/intellij/build-number-ranges.html)). |
| **Neovim** | MessagePack-RPC API over the default socket (`v:servername`) or `--listen 127.0.0.1:PORT`; `--server ADDR --remote-expr/--remote-send`; discover API via `nvim_get_api_info()` or `nvim --api-info`. ([:help api](https://neovim.io/doc/user/api.html), [:help remote](https://neovim.io/doc/user/remote.html)) | None beyond filesystem perms on the socket. Docs warn TCP sockets are "generally less secure than named pipes". | Ideal control-plane target: typed API introspection, stable since 0.x. |
| **Emacs** | `server-start` + `emacsclient --eval EXPR`; Unix socket or TCP with auth file. | UNKNOWN from primary doc this session (gnu.org returned 429). INFER from general knowledge: Unix socket in `server-socket-dir`; TCP mode writes an auth key file in `server-auth-dir`. Mark as unverified. | `--eval` is arbitrary Lisp = full user privileges. |

### Creative / 2D / 3D

| Host | Extension surface | Permission model | Notes |
|---|---|---|---|
| **Blender** | Python add-ons / extensions (`bpy`), Text editor, `-P script.py`, `-b` background. Extension manifest can declare `permissions` = files, network, clipboard, camera, microphone, each with a one-sentence reason; extensions should check `bpy.app.online_access`. ([extensions getting started](https://docs.blender.org/manual/en/latest/advanced/extensions/getting_started.html)) | "Python does not restrict what a script can do." Auto-run of scripts embedded in .blend files is off by default; "Trusted Source" per-file; `-y/--enable-autoexec`, `-Y/--disable-autoexec`. ([Scripting & Security](https://docs.blender.org/manual/en/4.2/advanced/scripting/security.html)) Extensions Platform: upload requires Blender ID; held for moderation review before publishing. | **First-party MCP**: Blender Lab "MCP Server for Blender", add-on v1.0.3, requires Blender 5.1+, MCP server shipped as `.mcpb` or source; page warns it "will execute LLM generated code in Blender without any guards" and recommends a VM. Transport, port and licence not stated on the page; source repo at projects.blender.org returned 403 to my fetch. ([blender.org/lab/mcp-server](https://www.blender.org/lab/mcp-server/)) |
| **GIMP 3.x** | Python plug-ins via GObject Introspection: subclass `Gimp.PlugIn`, implement `do_query_procedures`, register `Gimp.ImageProcedure`. Script-Fu also present. ([GIMP tutorial](https://docs.gimp.org/en/gimp-using-python-plug-in-tutorial.html)) | No sandbox described. No store. | Plug-ins run as separate processes talking to GIMP's PDB (INFER from GIMP architecture; not re-verified). |
| **Krita** | Python plugins in `pykrita/` with `.desktop` manifest (`ServiceTypes=Krita/PythonPlugin`, `X-KDE-Library`), `Extension` subclass registered via `Krita.instance().addExtension(...)`; PyQt available; "Scripter" console (Tools > Scripts > Scripter). Plugins must be enabled manually in the Python Plugin Manager. ([intro](https://docs.krita.org/en/user_manual/python_scripting/introduction_to_python_scripting.html), [howto](https://docs.krita.org/en/user_manual/python_scripting/krita_python_plugin_howto.html)) | No sandbox. No store. | PyQt in-process => a resident socket server is straightforward (INFER). |
| **Adobe (Photoshop, Premiere, InDesign, XD)** | UXP (JS, manifest v5), UXP Hybrid plugins (C++ `uxpaddon`, Photoshop 24.2+, Premiere 26.2+ per Adobe blog Apr 2026), legacy CEP + ExtendScript (no official EOL date found). ([UXP manifest v5](https://developer.adobe.com/photoshop/uxp/2022/guides/uxp-guide/uxp-misc/manifest-v5/), [hybrid plugins](https://developer.adobe.com/uxp/guides/how-to/hybrid-plugins/)) | Manifest v5 is default-deny: `network.domains`, `localFileSystem` (`request`/`plugin`/full), `launchProcess`, `webview.domains` ("all" discouraged; may be blocked by enterprises). Developer Mode must be enabled to load unpackaged plugins via UXP Developer Tool (older docs). Marketplace review: plugin "must not launch external applications (only browser links)", no debug consoles in production, hybrid binaries for 3 architectures, signed+notarised on macOS; ~10 business days. ([review guidelines](https://developer.adobe.com/uxp/guides/how-to/distribution/review-guidelines/)) | Forum reports `fetch` to `http://<ip>:<port>` rejected despite manifest listing — localhost bridging from UXP is fragile (SECONDARY). Hybrid C++ addon is the realistic path for a socket bridge (INFER). **First-party MCP**: "Adobe for creativity" hosted remote server at `adobe-creativity.adobe.io/mcp`, GA Apr 2026, ~50–67 tools across Photoshop/Lightroom/Illustrator/Firefly/Premiere/Express/Stock; reviewers note it works at Express/Firefly level, not full desktop apps (all SECONDARY; no Adobe primary page found). |
| **Figma** | Plugin API (sandboxed JS + iframe UI); `manifest.json` `networkAccess.allowedDomains` (`["none"]`, `["*"]`, or list; `devAllowedDomains`), enforced via CSP. ([manifest](https://developers.figma.com/docs/plugins/manifest), [network](https://developers.figma.com/docs/plugins/making-network-requests)) | Community review; security disclosure form (open beta); rejection for plugins that "read or modify a Figma file without a user's explicit awareness and consent"; "Any attempts to exploit the Figma Plugin API will result in immediate removal" and a publishing ban. ([review guidelines](https://help.figma.com/hc/en-us/articles/360039958914)) | **First-party MCP**: remote `https://mcp.figma.com/mcp` (recommended) and a desktop-app local server (org/enterprise/government needs; seat requirements). Read ("Dev Mode" context, Code Connect) is established; write-to-canvas is beta, remote-only, "free during the beta period and will eventually become a usage-based paid feature". ([Figma MCP docs](https://developers.figma.com/docs/figma-mcp-server/)) |

### Office / productivity

| Host | Surface | Permission model | Notes |
|---|---|---|---|
| **Microsoft Office** | (1) Office Add-ins (Office.js, web runtime), manifest `Permissions`: Restricted → ReadDocument → ReadAllDocument → WriteDocument → ReadWriteDocument; application-specific APIs need ReadWriteDocument even for read ([permissions](https://learn.microsoft.com/en-us/office/dev/Add-ins/develop/requesting-permissions-for-api-use-in-content-and-task-pane-add-ins)). (2) VBA + COM automation (`Excel.Application`). (3) Office Scripts (TypeScript, Excel; run via button or Power Automate; scheduling currently routed through Power Automate) ([Office Scripts](https://learn.microsoft.com/en-us/office/dev/scripts/overview/excel)). | Add-ins: runtime enforces the declared permission subset; "sandbox" is the web runtime. VBA: Trust Center macro settings; "Trust access to the VBA project object model" is off by default and often locked by Group Policy (`AccessVBOM`), so automation that touches VBA projects fails with error 6068. | Build 2025 added Copilot agent "add-in actions" via the unified manifest. Whether Microsoft ships a desktop-Office MCP server: UNKNOWN (search budget exhausted). Microsoft's official MCP catalog lists a remote "Microsoft 365 Copilot Chat" server (SECONDARY). |
| **Obsidian** | Plugin API (TypeScript on Electron; full Node access). Community directory Developer policies: must not obfuscate, no client-side telemetry, no self-update/install, no dynamic ads; must disclose network use, file access outside vaults, server-side telemetry (privacy policy link), closed source case-by-case. ([Developer policies](https://docs.obsidian.md/Community+directory/Developer+policies)) | No sandbox. Automated review scans every version; failing versions removed from search within 24h; manual review focused on popular/featured ([future of plugins](https://obsidian.md/blog/future-of-plugins/)). May 2026 announcement (SECONDARY): plugins will declare capabilities (network, file system, clipboard) shown to users pre-install. | No first-party MCP found; de-facto path is the community "Local REST API" plugin (HTTPS on 127.0.0.1:27124, bearer token) + `mcp-obsidian`, or "REST and MCP server" plugin (SECONDARY). Policies bar a *directory-listed* plugin from self-updating; a privately sideloaded bridge is not bound by them. |

### Browsers

| Host | Surface | Permission model | Notes |
|---|---|---|---|
| **Chrome / Chromium extensions** | Native Messaging: extension declares `nativeMessaging` permission; host manifest (`name`, `path`, `type: "stdio"`, `allowed_origins` without wildcards) registered per-OS (Windows registry `...\NativeMessagingHosts\<name>`, macOS/Linux JSON in fixed dirs). `connectNative` keeps one host process alive until the port closes; `sendNativeMessage` spawns one process per message. Length-prefixed UTF-8 JSON; 1 MB host→Chrome, 64 MiB Chrome→host. ([native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)) | Extension permissions + CWS review. Policy: no obfuscation, applies "to any external code or resource fetched by the extension package"; remote code only via documented APIs (Debugger, User Scripts). ([code readability](https://developer.chrome.com/docs/webstore/program-policies/code-readability)) | Natural shape for a control plane: the native host *is* the MCP server (or a sidecar of it), the extension is the resident agent in the browser. **First-party-ish MCP**: Google's `chrome-devtools-mcp` (public preview since 2025-09-23; supports Chrome and Chrome for Testing; telemetry on by default, `--no-usage-statistics`) ([repo](https://github.com/ChromeDevTools/chrome-devtools-mcp)). |
| **Electron apps (generic)** | `--remote-debugging-port=PORT` ("Enables remote debugging over HTTP on the specified port"); `--inspect[=host:port]` for the Node side (default 127.0.0.1:9229). ([command line switches](https://www.electronjs.org/docs/latest/api/command-line-switches)) Fuses: `EnableNodeCliInspectArguments` controls `--inspect*` and SIGUSR1 but (SECONDARY, HackTricks) not `--remote-debugging-port`. ([fuses](https://www.electronjs.org/docs/latest/tutorial/fuses)) | No auth on the CDP endpoint; anything on loopback can attach. | REA uses this read-only: `list_electron_targets` / `inspect_electron_page` accept only literal-port loopback endpoints and do not evaluate JS (`docs/electron-observation.md`), and `inspect_electron_fuses` reports whether the binary honours inspector flags (`docs/electron-fuses.md`). Requires relaunching the app with the flag; a packaged app that disabled fuses or checks argv may refuse. Boundary note: using CDP against a *third-party* Electron app the user runs is generally legitimate for the user's own data, but bypassing an app's fuse or integrity protections is not in scope. |

### Game engines

| Host | Surface | Permission model | Notes |
|---|---|---|---|
| **Unity** | C# Editor scripting; **Unity MCP Server** in `com.unity.ai.assistant` (Unity 6+): relay binary at `~/.unity/relay/` (`--mcp`) speaks stdio to the client and IPC (named pipes / Unix sockets) to an in-editor bridge; `McpToolRegistry`; custom tools via attributes/interfaces/runtime API; direct clients need approval in Project Settings, "gateway" connections auto-approved. ([overview 2.19](https://docs.unity3d.com/Packages/com.unity.ai.assistant@2.19/manual/integration/unity-mcp-overview.html)) | Approval-per-client; per-tool enable. | **Already deprecated**: the 2.19 page states "Unity MCP server is deprecated. Use the Unity command-line interface (CLI) instead." Unity CLI (`unity mcp configure <client>`, `unity eval`, `unity pipeline install`; Unity 6.0 LTS+; free, no Unity AI subscription) replaces it. ([replacement page](https://docs.unity.com/en-us/unity-cli/replace-mcp-server-unity-cli)) Good example of first-party-MCP churn within ~1 year. |
| **Unreal Engine** | Python Editor Script Plugin (embedded Python 3.11.8; console `py`, `-ExecutePythonScript`, `-run=pythonscript -script=`, `init_unreal.py`, startup scripts) ([Python scripting](https://dev.epicgames.com/documentation/en-us/unreal-engine/scripting-the-unreal-editor-using-python)). **Unreal MCP** plugin (`ModelContextProtocol`), Experimental, embeds an MCP server in the editor at `http://127.0.0.1:8000/mcp`, HTTP+SSE only, toolsets SceneTools/ActorTools/MaterialInstanceTools/ObjectTools, `ModelContextProtocol.GenerateClientConfig ClaudeCode`. ([Unreal MCP](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-mcp-in-unreal-editor)) | "There is no authentication layer; the plugin is not safe to expose beyond the local machine." Rejects non-loopback `Origin`. | Doc is under the UE 5.8 tree; first shipping version UNKNOWN. Python remote execution over UDP multicast: UNKNOWN this session. |
| **Godot** | GDScript/C# `EditorPlugin` (`plugin.cfg`), GDExtension (C/C++), CLI `--headless`, `--script <res path>`, `-e/--editor`, `--debug-server tcp://127.0.0.1:6007`, `--remote-debug`. ([CLI tutorial](https://docs.godotengine.org/en/stable/tutorials/editor/command_line_tutorial.html)) | No sandbox; Asset Library review is light (UNKNOWN detail). | No official MCP found; community `Coding-Solo/godot-mcp` (MIT) and others (SECONDARY). |

### Audio / DAW

| Host | Surface | Permission model | Notes |
|---|---|---|---|
| **Reaper** | ReaScript in Lua (built-in), EEL2 (built-in), Python (external install, `from reaper_python import *`, `RPR_*`); C/C++ extension SDK; API docs generated in-app (Help > ReaScript documentation), online copy lags. ([ReaScript](https://www.reaper.fm/sdk/reascript/reascript.php), [API help](https://reaper.fm/sdk/reascript/reascripthelp.html)) | None. | Lua/EEL run on the main thread per defer cycle; a socket server needs non-blocking polling via `reaper.defer` or the C++ extension route (INFER). |
| **Audacity / Tenacity** | `mod-script-pipe` named pipe; enable module in Preferences, restart; any language with named pipes. ([manual](https://manual.audacityteam.org/man/scripting.html)) | Manual warns enabling the pipe "gives outside programs control of the Audacity interface". | Python wrappers `pyaudacity`, `audacity-scripting` (SECONDARY). |
| **Ableton Live** | Only "MIDI Remote Scripts" (Python, `_Framework.ControlSurface`, Live Object Model) — **officially undocumented**; community bridges `exec()` Python sent over TCP (SECONDARY). | None. | Highest version fragility; vendor explicitly has not offered an API. Logic Pro Scripter: not researched. |

### CAD

| Host | Surface | Notes |
|---|---|---|
| **FreeCAD** | Full Python environment (`FreeCAD -c`, `FreeCADCmd`, in-GUI console) with Part/Draft/BIM modules ([Headless FreeCAD](https://wiki.freecad.org/Headless_FreeCAD)). Socket servers exist only as third-party (e.g. `mcp-freecad`). | LGPL app; no permission model. |
| **Autodesk Fusion** | Add-ins/scripts in Python and C++ (`adsk.core.Application.get()`, `run(context)` / `stop(context)` lifecycle). Only SECONDARY sources (Autodesk class handout, forum) retrieved. | Autodesk Terms: API use "to build applications solely for internal business use"; other uses need an Autodesk Platform Services developer licence; Acceptable Use bars automated collection of content and circumventing security ([Autodesk ToU](https://www.autodesk.com/company/terms-of-use), [AUP](https://www.autodesk.com/company/terms-of-use/en/acceptable-use)). First-party MCP: UNKNOWN. |
| **Rhino 8** | Rhino.Python (IronPython 2.7 and CPython 3.9), `rhinoscriptsyntax`, RhinoCommon, ScriptEditor, RhinoCode CLI; some CPython runtime docs marked "coming soon" ([What is Rhino.Python](https://developer.rhino3d.com/guides/rhinopython/what-is-rhinopython/), [guides](https://developer.rhino3d.com/guides/)). | First-party MCP: UNKNOWN. |

### Reverse-engineering hosts (REA's own targets)

- **Hopper**: Python scripting from the Scripts menu (community docs); REA launches Hopper with `--analysis -Y <bootstrap.py> --executable|--database <path>` (OBSERVED `src/hopper/BridgeLauncher.ts:400-408`). The bridge docstring says Hopper's "supported `--python` launcher option" (`bridge/hopper_bridge.py:3-4`) while the launcher passes `-Y`; I could not find Hopper's official CLI reference to reconcile the two — treat the flag name as UNKNOWN beyond "REA uses `-Y`".
- **Ghidra**: `analyzeHeadless <project> <name> -import <file> -preScript ... -postScript ...` (OBSERVED in Ghidra README and in `src/ghidra/GhidraLauncher.ts:997-1061`); REA's bridge is a Java GhidraScript (`bridge/ghidra/ReaGhidraBridge.java`) that opens a Unix-domain socket or `127.0.0.1:0` TCP listener (lines 268-288) and bearer-token authenticates.
- **IDA**: REA does not bridge directly; it adapts `mrexodia/ida-pro-mcp` (plugin inside IDA serving HTTP on `localhost:13337`, stdio proxy `server.py`; headless via idalib on a separate port) or `0xshlomil/ida-free-mcp` (native plugin; v1.1.2 binds `0.0.0.0` with no listen-address setting — REA doc tells users to firewall it) (`docs/ida-provider.md`).
- **Cutter**: REA ships a Cutter plugin (`bridge/cutter/rea_bridge.py`) that binds loopback, generates a per-process random token, writes a user-only session descriptor to a bridge directory for discovery, HMAC-signs frames, and tracks a `document_generation` counter so a client can detect that the open file changed under it (lines 467-609).

---

## Representative tools

Format: name — URL — one line — licence (if known).

- VS Code MCP extension API — https://code.visualstudio.com/api/extension-guides/mcp — register stdio/HTTP MCP servers from an extension; preview — MIT (VS Code OSS).
- JetBrains built-in MCP Server — https://www.jetbrains.com/help/idea/mcp-server.html — bundled since 2025.2; SSE/stdio/HTTP stream; per-tool toggles; brave mode — proprietary (IDE), plugin licence UNKNOWN.
- Neovim RPC — https://neovim.io/doc/user/api.html — MessagePack-RPC over socket/TCP, self-describing API — Apache-2.0.
- Blender Lab MCP Server — https://www.blender.org/lab/mcp-server/ — add-on v1.0.3 + `.mcpb` server; Blender 5.1+; executes LLM code unguarded — licence UNKNOWN (repo fetch 403).
- Unreal MCP (Epic) — https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-mcp-in-unreal-editor — Experimental in-editor HTTP/SSE MCP on 127.0.0.1:8000, no auth — UE EULA.
- Unity CLI MCP mode (replaces in-editor Unity MCP Server) — https://docs.unity.com/en-us/unity-cli/replace-mcp-server-unity-cli — `unity mcp configure`, `unity eval`; Unity 6.0 LTS+; free — proprietary.
- Figma MCP server — https://developers.figma.com/docs/figma-mcp-server/ — remote `mcp.figma.com/mcp` + desktop server; read GA, write-to-canvas beta — proprietary.
- Adobe for creativity remote MCP — `https://adobe-creativity.adobe.io/mcp` (SECONDARY: https://mcpservers.org/remote-mcp-servers/adobe-creativity) — hosted, OAuth, Express/Firefly-level tools — proprietary.
- Chrome DevTools MCP — https://github.com/ChromeDevTools/chrome-devtools-mcp — Google's MCP over CDP; public preview since 2025-09-23 — Apache-2.0 (INFER from repo norms; not verified).
- Chrome Native Messaging — https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging — stdio host process bound to specific extension IDs — n/a.
- Obsidian Local REST API (community, coddingtonbear) + `mcp-obsidian` — SECONDARY — HTTPS 127.0.0.1:27124 with API key — MIT (SECONDARY).
- Reaper ReaScript — https://www.reaper.fm/sdk/reascript/reascript.php — Lua/EEL/Python in-DAW scripting — proprietary host.
- Audacity mod-script-pipe — https://manual.audacityteam.org/man/scripting.html — named-pipe command interface — GPL.
- ida-pro-mcp — https://github.com/mrexodia/ida-pro-mcp — IDA plugin HTTP :13337 + stdio proxy; idalib headless — MIT (SECONDARY).
- ida-free-mcp — https://github.com/0xshlomil/ida-free-mcp — native IDA plugin MCP; 1.1.2 binds 0.0.0.0 — licence UNKNOWN.
- REA Hopper/Cutter/Ghidra bridges — `bridge/hopper_bridge.py`, `bridge/cutter/rea_bridge.py`, `bridge/ghidra/ReaGhidraBridge.java` in the checkout — in-host socket bridges with token auth — REA's licence per `LICENSE` in the checkout (not re-read here).
- Godot community MCP (`Coding-Solo/godot-mcp`) — SECONDARY — launches editor/runs project/captures debug output — MIT (SECONDARY).

---

## Strengths

- **Fidelity and determinism**: operations are addressed by object identity, not screen position; results are typed. Undo integration comes for free in most hosts.
- **Observability without heuristics**: full document models, diagnostics, selection, events.
- **Vendor-sanctioned**: a documented plugin API is the one surface the vendor has committed to supporting and that EULAs generally permit (subject to the clauses below).
- **Headless paths** exist for many hosts, allowing batch work and CI-style runs.
- **Growing first-party MCP coverage** means some targets need zero custom bridge code; the control plane only has to launch/connect and (optionally) proxy.
- **Reusable bridge template**: REA shows the same shape works for Python (Hopper, Cutter), Java (Ghidra) and third-party HTTP (IDA): loopback or Unix socket, random per-session token injected at launch, newline-delimited JSON, progress events, explicit `shutdown` semantics, idle timeouts so an orphaned bridge cannot pin the host (`hopper_bridge.py:1444-1472`).

---

## Failure modes

- **API drift / version fragility**: VS Code proposed APIs cannot ship to Marketplace and change across Insiders builds; JetBrains requires valid `since-build`/`until-build` and the API "changes between releases"; Hopper 4→5 moved to Python 3 and broke community scripts; Unity's first-party MCP was deprecated within roughly a year of launch in favour of a CLI; Unreal MCP is Experimental with "APIs and data formats … subject to change at any time"; Figma write features are beta and will become paid; ida-pro-mcp 2.0 renamed the legacy GUI tools and REA "fails closed with the missing tool names" (`docs/ida-provider.md`). A control plane needs per-host capability probing (`tools/list`, `nvim_get_api_info`, Hopper/IDA version checks) rather than assumed catalogs.
- **Threading constraints**: Hopper's API objects "are bound to its dedicated Python execution thread … moving calls to a worker can deadlock" (`hopper_bridge.py:1-6, 1473-1475`); Reaper Lua/EEL must yield to the UI; Blender `bpy` is main-thread; Qt hosts (Krita, Cutter) need signals to marshal onto the GUI thread (Cutter bridge imports `QObject, Signal` for this). Naive socket servers in a background thread corrupt state or crash the host.
- **Lifecycle**: hosts unload plugins on project switch (Fusion `stop(context)`), restart on preference change (Unreal plugin enable requires restart), or must be relaunched with a flag (Electron `--remote-debugging-port`, Blender `-P`, Hopper `-Y`). A resident bridge must advertise/withdraw itself (REA's Cutter descriptor directory) and the control plane must handle "document changed under me" (REA's `document_generation`, IDA input-hash checks).
- **No auth or wrong bind address**: Unreal MCP has "no authentication layer"; Electron CDP has none; ida-free-mcp 1.1.2 binds `0.0.0.0`; Obsidian REST-and-MCP plugin can be set to `0.0.0.0`. Loopback-only is necessary but not sufficient on multi-user hosts; prefer Unix sockets with 0600 perms or per-session tokens (REA does both).
- **Arbitrary-code tools**: Blender Lab's server and community Ableton bridges run LLM-generated code with no guard. Blender itself recommends a VM. A control plane that exposes such a tool must gate it behind explicit user approval per call or per session.
- **Store policy collisions**: Chrome Web Store forbids executing remote code and obfuscation; Obsidian forbids self-update and client-side telemetry; Adobe Marketplace forbids launching external applications; Figma bans "exploiting" the plugin API. A "resident agent plugin" that fetches instructions from an MCP server is *data*, not code, under most of these policies (INFER), but one that `eval`s server-supplied code generally is not publishable and must be sideloaded.
- **Sideload friction**: Adobe requires Developer Mode and (for hybrid) signed/notarised binaries plus admin credentials to install; VS Code warns on untrusted publishers; JetBrains warns on non-Marketplace plugins (SECONDARY); Office may have VBA object-model access locked by Group Policy.
- **Partial surfaces**: Office add-ins can't reach everything VBA/COM can; UXP cannot `fetch` arbitrary local ports reliably (forum SECONDARY); Figma plugins cannot run while the file is closed; Obsidian plugins need the Electron app running.
- **Telemetry and cloud dependence**: Chrome DevTools MCP sends usage stats by default; Figma's and Adobe's official servers are cloud-hosted (your document content transits their servers, and features are seat/plan-gated); IDA Free decompilation is cloud-based and "may request consent" (`docs/ida-provider.md`).

---

## Trade-offs vs other technique families

- **vs. GUI automation / computer-use (pixels, accessibility trees)**: plugin surfaces are far more reliable and cheaper per action, but exist only where the vendor provides them and need per-app adapter work; GUI automation is universal but brittle. Best pairing: plugin surface for the document model, GUI/accessibility fallback for dialogs and features the API omits (INFER).
- **vs. OS accessibility APIs (AT-SPI, UIA, AXUIElement)**: accessibility gives uniform read access to widgets across all apps with zero install, but no domain model and weak write semantics. Plugins give the domain model but require installation and per-host trust decisions.
- **vs. file-format manipulation (editing the project file on disk)**: file-level works with the app closed and is version-robust when the format is documented, but misses live state, triggers reload prompts, and can corrupt caches; Godot community servers explicitly split "file-level" vs "engine-level" (SECONDARY).
- **vs. debugger / instrumentation injection (Frida, CDP into unsuspecting processes, DLL injection)**: injection reaches apps with no API at all, but is the technique most likely to collide with EULAs' anti-reverse-engineering clauses, code-signing/integrity controls, and anti-cheat. Plugin surfaces are the sanctioned equivalent; REA itself keeps Frida and unrestricted debugger consoles behind separate opt-in tooling (`docs/reverse-engineering-tools.md:5`). For the agentic-anything boundary: prefer the plugin surface whenever one exists, and treat injection into third-party apps as out of scope unless the user owns the binary and the EULA permits it.
- **vs. first-party MCP servers specifically**: zero bridge code and vendor support, but you inherit their tool catalog, auth (often cloud OAuth), deprecation schedule, and sometimes pricing; a self-written bridge over the raw plugin API is more work but can be shaped to the control plane's uniform schema.

---

## Integration notes for an MCP control plane

Process model
- Three roles, as in REA: (1) **host process** (the app); (2) **resident bridge** loaded by the host's own plugin/script mechanism (Python add-on, Java GhidraScript, VS Code extension, Chrome extension + native host, Krita `Extension`, Reaper Lua defer loop, Unity `McpToolRegistry` tool, Unreal Python startup script); (3) **control-plane MCP server** out of process that owns discovery, auth, schema normalisation and permission prompting.
- Bridge transport should default to **Unix domain socket (0600) / Windows named pipe with a restrictive DACL** and fall back to `127.0.0.1:0` TCP with a per-session random token delivered out of band (env var, bootstrap file, descriptor file). REA injects `REA_SOCKET`/`REA_TOKEN` into a bootstrap script that `exec`s the bridge (`src/hopper/BridgeLauncher.ts:666-690`) and validates the token with `hmac.compare_digest` (`hopper_bridge.py:1353-1357`).
- Framing: newline-delimited JSON with `id`, `method`, `params`, plus out-of-band `event` messages for progress/diagnostics, and a size ceiling to stop a hung host (REA: 16 MiB default, configurable).
- Dispatch on the host's main/script thread; the socket accept loop may live on a worker but every API call must be marshalled (Qt signal, `bpy.app.timers`, `reaper.defer`, VS Code main loop, Unreal game thread tick).
- Idle/orphan handling: bounded accept timeout so an abandoned bridge exits (REA `REA_RECONNECT_TIMEOUT_SECONDS`), explicit `shutdown` method with "document retained vs closed" semantics, and a generation counter so stale clients fail fast.

Inventorying an app's extension surface quickly (checklist, INFER from the survey)
1. Vendor developer portal: search "<app> plugin API", "<app> scripting", "<app> SDK", "<app> MCP". Check for an in-app "Scripts"/"Plugins"/"Extensions" menu and a developer-mode preference.
2. Command-line help: `<app> --help` for `-P/--python`, `--script`, `--eval`, `--listen`, `--remote-debugging-port`, `-ExecutePythonScript`, `-run=`. For Electron apps inspect `app.asar` and fuses (REA `inspect_electron_fuses`).
3. Embedded interpreter presence: bundled `python3*.dll`/`libpython`, `lua5*.dll`, `v8`, `mono`/`.NET` — signals an in-process scripting runtime.
4. Existing automation surfaces: COM/AppleScript dictionaries (`sdef`), D-Bus services, named pipes, OSC/MIDI, HTTP admin ports; Office Trust Center settings.
5. Store/directory: does the app have a plugin marketplace, and what are its sideload rules (developer mode, signing, admin rights)?
6. Community: GitHub search "<app>-mcp", "<app> remote script", "<app> rpc" — if several exist, the surface is viable; read their threading and version caveats.
7. Licence/ToS scan for "automat", "script", "bot", "API", "internal business use", "reverse engineer".

Permission prompts
- Classify tools by effect: read-only (inventory, diagnostics) vs. document-mutating vs. process/shell (JetBrains `execute_terminal_command`, Unreal `ObjectTools`, any `eval`). JetBrains' model (per-tool enable + confirmation unless "brave mode") and Unity's (per-client approval remembered) are good templates.
- Arbitrary-code tools (`exec_python`, `eval_lua`, `nvim_exec_lua`, `emacsclient --eval`) should be off by default and require per-call approval; surface the code to the user before execution.
- Mirror the host's own disclosures: if a plugin store requires network/file-access disclosure, the control plane should record the same facts in its tool descriptions.

Latency and state
- In-process bridges answer in milliseconds for reads; heavy operations (Ghidra analysis, Unreal asset import) need progress events and cancellation. REA models startup deadlines and "cleanup owners" explicitly because host processes outlive requests.
- First-party cloud MCPs (Figma remote, Adobe) add network latency and rate limits and are plan-gated.
- Keep a per-host **capability probe** cache keyed by host version + bridge version, invalidated on reconnect; never assume tool names across versions (REA's IDA profile check is the model).

Packaging
- Ship bridges as the host's native package type: VSIX (sideload OK for proposed APIs), JetBrains plugin ZIP, Blender extension ZIP with manifest permissions, Krita `pykrita` folder + `.desktop`, Chrome extension + native host manifest installer (registry/JSON per OS), Office add-in manifest, Unreal plugin folder, Unity package. Make the control plane install/register them only with explicit consent (REA setup "asks before changing agent configuration or installing Hopper", `docs/installation.md:23`).

---

## Authorization & legal notes

- **Plugin surfaces are the sanctioned path**, but EULAs can still restrict them. Observed/secondary examples: Autodesk limits API use to "internal business use" without a developer licence and bars automated collection and circumvention of security; Adobe Marketplace rules bar plugins launching external applications (affects distribution, not private use); several SaaS/desktop EULAs flatly prohibit "bots, scripts, or any similar automation technology" (law-firm commentary, SECONDARY); Steam's agreement bans automation tools (SECONDARY). Fusion "personal use" automation limits: UNKNOWN.
- **Store policies ≠ law**: Obsidian/Chrome/Figma/Adobe policies bind what can be *listed*; a user sideloading their own bridge is governed by the app EULA, not the store policy. The control plane should not misrepresent a sideloaded bridge as store-reviewed.
- **Undocumented APIs** (Ableton Remote Scripts) are tolerated by the vendor but unsupported; using them is not circumvention, but versions break silently.
- **Boundary items for agentic-anything** (flag, do not implement): bypassing Electron fuses/ASAR integrity, attaching debuggers to apps whose EULA forbids reverse engineering, using CDP/injection on apps with anti-tamper, and using plugin APIs to extract licensed assets (e.g. Adobe Stock, Unity Asset Store content) beyond the licence. Hopper/IDA/Ghidra bridging is legitimate for binaries the user owns or is authorised to analyse; REA's `DISCLAIMER.md` and `docs/ida-provider.md` ("REA does not install, upgrade, activate, or license them") show the posture: never touch licensing/activation.
- **Cloud first-party MCPs** move document content to the vendor; check data-processing terms before enabling for confidential projects (Figma seat/plan gating; Adobe OAuth; IDA Free cloud decompiler consent).
- **Enterprise controls**: Office `AccessVBOM` GPO, Adobe enterprise blocking of `webview.domains: all`, Figma for Government desktop-only server — the control plane should detect and report "blocked by policy" rather than attempt workarounds.

---

## Open questions

- Does Microsoft ship (or plan) an MCP server for desktop Office / Excel beyond the cloud "Microsoft 365 Copilot Chat" server? (not verified; search budget exhausted)
- McNeel (Rhino) and Autodesk (Fusion/AutoCAD) first-party MCP status — UNKNOWN.
- Unreal Python "Remote Execution" (UDP multicast) specifics and whether Unreal MCP will gain auth — UNKNOWN.
- Blender Lab MCP server licence, transport and port — repo returned 403; needs a direct read of `projects.blender.org/blender-lab/mcp-server`.
- Hopper's documented CLI flag for executing a Python script at launch (`-Y` as used by REA vs "--python" in REA's docstring) — needs Hopper's official docs.
- Whether VS Code's `registerMcpServerDefinitionProvider` is finalized (release-note version) — the guide still says "preview".
- Emacs server auth details — primary manual unreachable this session.
- Obsidian's announced capability declarations (May 2026, SECONDARY): exact schema and whether sideloaded plugins are affected.
- For a uniform control plane: can one bridge "shape" (NDJSON over Unix socket + token, progress events, generation counter) be expressed as a reusable SDK across Python/Java/JS/Lua hosts, or do threading models force per-host designs? REA suggests the wire protocol is reusable but dispatch is host-specific.

---

## Sources

Primary (vendor) docs
- VS Code MCP extension guide: https://code.visualstudio.com/api/extension-guides/mcp
- VS Code extension runtime security: https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security
- VS Code Workspace Trust: https://code.visualstudio.com/docs/editor/workspace-trust
- VS Code proposed API: https://code.visualstudio.com/api/advanced-topics/using-proposed-api
- JetBrains IDEA MCP Server: https://www.jetbrains.com/help/idea/mcp-server.html
- JetBrains plugin security: https://plugins.jetbrains.com/docs/marketplace/understanding-plugin-security.html
- JetBrains build number ranges: https://plugins.jetbrains.com/docs/intellij/build-number-ranges.html
- Neovim API / remote: https://neovim.io/doc/user/api.html , https://neovim.io/doc/user/remote.html
- Blender Lab MCP server: https://www.blender.org/lab/mcp-server/
- Blender Scripting & Security: https://docs.blender.org/manual/en/4.2/advanced/scripting/security.html
- Blender extensions getting started: https://docs.blender.org/manual/en/latest/advanced/extensions/getting_started.html
- GIMP Python plug-in tutorial: https://docs.gimp.org/en/gimp-using-python-plug-in-tutorial.html
- Krita Python scripting: https://docs.krita.org/en/user_manual/python_scripting/introduction_to_python_scripting.html , https://docs.krita.org/en/user_manual/python_scripting/krita_python_plugin_howto.html
- Adobe UXP manifest v5: https://developer.adobe.com/photoshop/uxp/2022/guides/uxp-guide/uxp-misc/manifest-v5/
- Adobe UXP review guidelines: https://developer.adobe.com/uxp/guides/how-to/distribution/review-guidelines/
- Adobe UXP hybrid plugins: https://developer.adobe.com/uxp/guides/how-to/hybrid-plugins/
- Figma plugin manifest / network: https://developers.figma.com/docs/plugins/manifest , https://developers.figma.com/docs/plugins/making-network-requests
- Figma review guidelines: https://help.figma.com/hc/en-us/articles/360039958914
- Figma MCP server: https://developers.figma.com/docs/figma-mcp-server/
- Office add-in permissions: https://learn.microsoft.com/en-us/office/dev/Add-ins/develop/requesting-permissions-for-api-use-in-content-and-task-pane-add-ins
- Office Scripts overview: https://learn.microsoft.com/en-us/office/dev/scripts/overview/excel
- Microsoft "Programmatic access to Office VBA project is denied": https://support.microsoft.com/en-us/excel/programmatic-access-to-office-vba-project-is-denied
- Obsidian Developer policies: https://docs.obsidian.md/Community+directory/Developer+policies
- Obsidian "future of plugins": https://obsidian.md/blog/future-of-plugins/
- Chrome native messaging: https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging
- Chrome Web Store code readability policy: https://developer.chrome.com/docs/webstore/program-policies/code-readability
- Chrome DevTools MCP: https://github.com/ChromeDevTools/chrome-devtools-mcp
- Electron command line switches: https://www.electronjs.org/docs/latest/api/command-line-switches
- Electron fuses: https://www.electronjs.org/docs/latest/tutorial/fuses
- Unity MCP overview (2.19): https://docs.unity3d.com/Packages/com.unity.ai.assistant@2.19/manual/integration/unity-mcp-overview.html
- Unity CLI replacement for MCP server: https://docs.unity.com/en-us/unity-cli/replace-mcp-server-unity-cli
- Unreal MCP: https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-mcp-in-unreal-editor
- Unreal Python scripting: https://dev.epicgames.com/documentation/en-us/unreal-engine/scripting-the-unreal-editor-using-python
- Godot CLI: https://docs.godotengine.org/en/stable/tutorials/editor/command_line_tutorial.html
- Reaper ReaScript: https://www.reaper.fm/sdk/reascript/reascript.php , https://reaper.fm/sdk/reascript/reascripthelp.html
- Audacity scripting: https://manual.audacityteam.org/man/scripting.html
- FreeCAD headless: https://wiki.freecad.org/Headless_FreeCAD
- Rhino.Python: https://developer.rhino3d.com/guides/rhinopython/what-is-rhinopython/ , https://developer.rhino3d.com/guides/
- Ghidra headless README: https://www.ghidradocs.com/12.0_PUBLIC/support/analyzeHeadlessREADME.html
- Autodesk Terms of Use / Acceptable Use: https://www.autodesk.com/company/terms-of-use , https://www.autodesk.com/company/terms-of-use/en/acceptable-use

Secondary
- Unity blog on MCP open beta: https://unity.com/blog/unity-ai-mcp-how-to-get-started
- Figma Dev Mode MCP blog: https://www.figma.com/blog/introducing-figma-mcp-server/
- Adobe for creativity listing: https://mcpservers.org/remote-mcp-servers/adobe-creativity
- Adobe hybrid plugins for Premiere (Adobe blog, Apr 2026): https://blog.developer.adobe.com/en/publish/2026/04/uxp-hybrid-plugins-now-available-for-premiere
- OX Security on IDE extension verification: https://www.ox.security/blog/can-you-trust-that-verified-symbol-exploiting-ide-extensions-is-easier-than-it-should-be/
- HackTricks on Electron remote-debugging-port vs fuses: https://book.hacktricks.xyz/macos-hardening/macos-security-and-privilege-escalation/macos-proces-abuse/macos-electron-applications-injection
- ida-pro-mcp: https://github.com/mrexodia/ida-pro-mcp ; ida-free-mcp: https://github.com/0xshlomil/ida-free-mcp
- Obsidian Local REST API / mcp-obsidian guide: https://contextbolt.com/blog/obsidian-mcp-claude/
- Ableton Remote Script community bridges: https://github.com/Maboroshy/Generic-Python-Remote-Script , https://pypi.org/project/ableton-live-mcp/0.1.0/
- Godot community MCP: https://enterprisedna.co/directories/mcp/coding-solo-godot-mcp
- Law-firm commentary on EULA automation bans: https://www.wilsonlegalgroup.com/blogs/business-law/software-companies-ban-ai-agents-eula
- gihyo.jp on Chrome DevTools MCP preview date: https://gihyo.jp/article/2025/09/chrome-devtools-mcp-server

REA checkout files read (path prefix `/tmp/claude-1000/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/scratchpad/rea-src/`)
- `bridge/hopper_bridge.py` (lines 1-13, 1320-1477)
- `bridge/cutter/rea_bridge.py` (lines 1-40, 467-609 via grep)
- `bridge/ghidra/ReaGhidraBridge.java` (lines 1-40, 131-288 via grep)
- `src/hopper/BridgeLauncher.ts` (lines 392-425, 660-800)
- `src/ghidra/GhidraLauncher.ts` (lines 997-1061 via grep)
- `docs/ida-provider.md`, `docs/electron-observation.md`, `docs/reverse-engineering-tools.md` (lines 100-130), `docs/installation.md` (lines 677-700), `docs/ghidra-provider.md` (lines 1-40)
