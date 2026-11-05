# REA desktop-application and agent-environment subsystems: what is observed, what is driven, and where REA stops

Research date: 2026-11-05

Source: the owner's local REA checkout at `/Users/nhanclie/rea` (same as upstream `morluto/rea` when this was written). REA code is cited as `src/path.ts:line`, docs as `docs/file.md:line`.

Evidence legend: **OBSERVED** = I read it in REA source or docs (path:line given). **SECONDARY** = third-party or search summary. **INFERRED** = my interpretation. **UNKNOWN** = not established.

Relation to existing reports, so this one does not repeat them:
- Electron fuses, passive CDP Electron inspection, the `capture_electron_scenario` active hook and its blocking policy, and Playwright browser scenarios are covered in `rea-runtime-observation-surfaces.md` §4–§5. Below they only get a short recap in §9, plus details that matter for driving.
- The `ToolEffects` → MCP annotation derivation and the HTTP transport are covered in `rea-mcp-server-adapter.md` and `rea-contracts-and-evidence-model.md`.
- The write-gating idea ("serve a write only when a later read confirms it") is already noted in `salvaged/tool-design-philosophy-and-verification.json`. Its open question, "does write-confirmation transfer to non-HTTP desktop state?", gets a concrete answer in §6 and §13.
- `survey-accessibility-trees.md` and `survey-computer-use-vision-agents.md` mention `observe_native_ui` briefly. §3.4 and §11 give the exact action semantics.

---

## 1. Summary

- **OBSERVED** "Desktop application" means four different things in REA, and each has its own tool:
  1. a **static installation directory**, triaged for runtime family (`inspect_desktop_application`, MCP + CLI);
  2. an **installed Linux X11 desktop app**, launched and watched as a session (`capture-desktop-application`, CLI only);
  3. an **Apple bundle inventory** projection (`project_apple_application_graph`, static);
  4. **one running macOS window** chosen by PID + window ID (`observe_native_ui` / `capture_native_ui_scenario`, MCP).
- **OBSERVED** "Agent environment" means an **agentenv-framework bundle** (Scale's `agentenv-framework`, `docs/agent-environments-quickstart.md:20`). REA makes two kinds:
  - **(a) web/API environments.** A recorded HAR or browser scenario becomes a local MCP server (`rea environment-serve`). It answers from replayed records or from a derived in-memory state, with write tools served *only* when the recording confirmed their effect.
  - **(b) desktop "computer app" environments.** A desktop capture becomes a Python `ComputerAppEnvironment` class. It installs the real app in an `ubuntu-24.04` container, seeds the user's state, and exposes `data/get|add|reset`.
- **OBSERVED** REA is mostly a **recorder, projector and validator, not a driver**. The Linux desktop capture records no clicks or keystrokes (`docs/desktop-application-capture.md:304`). It expects a human or an *external* driver such as `xdotool` to work the app and to send checkpoint lines over a FIFO (`:95-100`). Agent environments replay or simulate. They never touch the live app (`docs/agent-environments.md:22-23`, `:1211-1214`).
- **OBSERVED** REA's real GUI-driving surfaces are three **batch, declarative, closed-vocabulary scenarios**, each run as one call with before/after captures:
  - browser (`click/fill/goto/press/select_option/wait_for/wait_for_timeout/request`, `src/domain/browserScenarioValues.ts:370-454`);
  - Electron (`click/wait/renderer-reload/renderer-crash/deep-link`, `src/browser/PlaywrightElectronActiveActions.ts:263-300`);
  - macOS native UI. It uses AX only: `AXPress`, `AXIncrement/AXDecrement`, setting `AXValue`, and bounded waits. It never injects global events (`bridge/native/ReaNativeUI.swift:111-113`, `src/native/NativeUiObservation.ts:311`).
  
  None of them is an interactive per-step loop, Windows UIA, Linux AT-SPI, or vision-coordinate input.
- **OBSERVED** The authority model is declarative and audit-based. There are no grants:
  - Every MCP tool has a `ToolEffects` row (`src/contracts/toolEffects.ts:2-22`). Active scenarios get the full destructive set (`:596-606`, `:656-680`).
  - Evidence carries `authority` (`controlled-replay` for the desktop capture, `src/application/desktop/DesktopApplicationCapture.ts:161`) and `environment.isolation` (`"none"` for desktop capture, `:167`).
  - Safety comes from refusals, identity binding, owned process lifetimes, budgets, and "leave-as-is / not a sandbox" statements in docs and Evidence limitations.
- **INFERRED** For agentic-anything the reusable assets are:
  - the **capture → state view → task-with-declarative-checks → seed-fails/end-passes gate** pipeline (a verifier spec for any app whose state lives in files);
  - **traffic → typed MCP tool synthesis** with confirmed-write admission;
  - **identity binding** of a drive target (executable SHA-256 + launch time + PID + window ID + unique AX geometry);
  - **run-token process ownership** for launched GUI apps.

  The missing pieces are a live per-step drive loop, cross-OS a11y and input adapters, per-action authority and confirmation, and undo/restore.

---

## 2. Surface map

| Surface | Entry point | Host | Interface | Observes | Drives | Effects row |
|---|---|---|---|---|---|---|
| Install-dir triage | `inspect_desktop_application` / `rea inspect-desktop-application` | any | MCP + CLI | dir entries, headers, bounded ranges | nothing | `evidence` (`toolEffects.ts:129`) |
| Linux desktop capture | `rea capture-desktop-application` | Linux + X11 + EWMH WM | CLI only (stdin control) | processes (`/proc`), X11 client windows (`xprop`), open fds, home-dir walks + content | **launches** the app; no input | none (not an MCP tool) |
| Desktop state view | `rea inspect-desktop-application-state` | any | CLI | the capture folder only | nothing | none |
| Desktop task draft | `rea describe-desktop-application-task` | any | CLI | capture + state view | nothing | none |
| Desktop export | `rea export-desktop-application [--task]` | any | CLI | capture + state view | nothing (writes a bundle) | none |
| Apple graph | `project_apple_application_graph` | any | MCP + CLI | `inventory_artifact` Evidence | nothing | `evidence` (`toolEffects.ts:715`) |
| macOS window observe | `observe_native_ui` | macOS 14+ | MCP | AX tree + window screenshot of one PID/window | nothing | session+launch(helper)+fs (`toolEffects.ts:591-595`) |
| macOS UI scenario | `capture_native_ui_scenario` | macOS | MCP | before/after AX + screenshot per step | AX press/increment/decrement/set-value | full destructive (`toolEffects.ts:596-606`) |
| Web env build/export | `rea environment-build`, `environment-export`; MCP `export_environment_bundle` | any | CLI + MCP | capture Evidence (HAR / scenario) | nothing (writes bundle) | `writesFilesystem` (`toolEffects.ts:693-696`) |
| Web env serve | `rea environment-serve` | any | CLI → serves HTTP MCP + JSON-RPC | retained Evidence | simulated writes on in-memory state only | n/a (it is the server) |
| Web env fidelity | `rea environment-compare` | any | CLI | env vs. probe captures, offline | nothing | none |
| Browser / Electron scenarios | `capture_browser_scenario`, `capture_electron_scenario` | any / Electron | MCP + CLI | network, DOM, a11y, IPC shapes | closed action vocab | full destructive (`toolEffects.ts:656-680`) |

OBSERVED: the CLI names for the desktop capture family are at `src/cliCommandNames.ts:153-156` and the environment family at `:147-150`. `TOOL_EFFECTS` is "Explicit effect audit for every public tool" (`src/contracts/toolEffects.ts:53`). The desktop capture/state/task/export commands and `environment-serve/compare/build` have no row there. INFERRED: they are deliberately **not MCP tools**. A launched, long-running, stdin-controlled GUI session is kept out of the agent tool catalog.

---

## 3. What a "desktop application" is in REA's model

### 3.1 Static installation directory (`inspect_desktop_application`)

- OBSERVED contract: "Inspect a local desktop application directory using bounded listings, file headers and selected metadata… No target execution, extraction, network or target changes" (`src/contracts/desktopApplicationToolContracts.ts:18-19`). Default `detail: "summary"` retains complete Evidence for paging through `inspect_analysis_view` kind `desktop-application` with collections `files|omissions|candidates|components|next_probes` (`docs/desktop-applications.md:16-56`).
- OBSERVED identification is **evidence-based runtime-family detection**, not "which app is this":
  - .NET: a PE CLI directory.
  - Unity Mono: a UnityEngine assembly ref.
  - Unity IL2CPP: metadata magic plus `GameAssembly.*`.
  - Godot: `GDPC`.
  - GameMaker: `FORM`/`GEN8`.
  - Unreal: `Content/Paks` or the IoStore TOC.
  - Electron: the `resources/app.asar` path convention.
  
  (`docs/desktop-applications.md:94-102`.) Candidates "do not prove loading or execution". Anti-cheat/mod-loader labels "identify path conventions only… No bypass is provided" (`:122-124`).
- OBSERVED: the output includes **next probes**, concrete follow-up tool calls such as `inspect_managed_members` or `inspect_il2cpp_metadata` at an admitted path (`:104-120`). Child symlinks are not followed (`src/artifacts/DesktopApplicationReader.ts:268`).
- OBSERVED: `snapshot_sha256` commits to the retained paths, metadata and read-range digests, "not a digest of all file contents" (`docs/desktop-applications.md:132-142`). Defaults are 20,000 entries and 8 MiB of reads, up to a cap of 100,000 entries and 64 MiB (`:146-148`).
- INFERRED: this is a **control-surface discovery** pattern. Classify the runtime, then emit typed next probes. agentic-anything can reuse it directly: classify an install as Electron / .NET / Qt / Java / web-wrapper, then propose the matching adapter (CDP, UIA, AT-SPI, plugin API).

### 3.2 Installed Linux desktop application (capture inventory)

OBSERVED: the identity and discovery pipeline in `src/desktop/LinuxApplicationInventory.ts:372-456` and `docs/desktop-application-capture.md:28-49`, `:127-230` works like this.
1. **Name it.** Use exactly one of `--desktop-entry <id|abs path>` or `--executable` (`src/application/desktop/DesktopCaptureRefusals.ts:173-181`). Desktop entries are searched in `$XDG_DATA_HOME/applications`, then in `$XDG_DATA_DIRS`. Symlinked entries (flatpak, Nix) are followed. `Exec` is expanded with no files, and `--argument` values replace the field codes (`docs/desktop-application-capture.md:36-48`; `src/domain/desktop/desktopEntry.ts:44,84,125`).
2. **Unwrap the launcher.** An `env NAME=value` prefix changes PATH/XDG for the program. A `flatpak run` launch takes the app id as its first operand (`src/domain/desktop/desktopEntry.ts:226`; `LinuxApplicationInventory.ts:396-406`). A program found only through a relative/empty PATH entry is refused, "since the exported computer would not have that entry" (`docs/desktop-application-capture.md:44-47`; `LinuxApplicationInventory.ts:413-417`).
3. **Identify the package.** Options are Debian (dpkg name/version/arch, apt sources with origin `o=`, vendor repos with a **gpgv-verified** signing key), snap, flatpak, or AppImage by path (`src/domain/desktop/linuxPackages.ts:166-182`; `docs/desktop-application-capture.md:127-171`). URI credentials are stripped everywhere (`:145-149`). `--package-file` records the `.deb` facts and SHA-256 but never copies the file: "it is the vendor's" (`:202-216`).
4. **Walk libraries.** For non-archive packages, REA does a DT_NEEDED walk emulating ld.so (RPATH/RUNPATH/`$ORIGIN`/ldconfig) and maps sonames to packages through `dpkg-query -S` (`:173-195`).
5. **Mark Electron.** The fuse sentinel or `resources/app.asar` (`LinuxApplicationInventory.ts:210-215`, using `src/electron/ElectronFuseReader.ts`).
6. **Guess state directories.** XDG config/data/state/cache plus a dot-folder for each of package / executable / entry id / last id part / entry name. Existing `~/.<name>rc`-style dotfiles are included, and so is `~/snap/<pkg>` or `~/.var/app/<id>` (`docs/desktop-application-capture.md:222-230`; `LinuxApplicationInventory.ts:421-431`).
7. **Window identity.** Match names are `StartupWMClass`, the executable basename, the entry id and its last part, and the flatpak id, all lowercased. Matching is by PID too, except for flatpak, whose sandbox PIDs are not host PIDs (`src/application/desktop/DesktopCaptureScope.ts:96-117`).

### 3.3 Apple bundles (`project_apple_application_graph`)

OBSERVED, execution-free. This is a projection over `inventory_artifact` Evidence for an IPA, `.app`, ZIP or DMG. It covers application roots, bundle anatomy (`macos-deep` / `versioned-framework` / `shallow`), executable candidates, signing paths, and runtime-family hints from whole path segments (`React.framework`, `hermes.framework`, `Flutter.framework`, `UnityFramework.framework`). It also emits JS↔native "bridge hypotheses" capped at 2 MiB. It "does not parse plist or CMS semantics or claim observed runtime calls" (`docs/apple-application-analysis.md:1-41`, `:70-120`). INFERRED: this is useful only as a static "which adapter applies" classifier for macOS apps. It drives nothing.

### 3.4 One running macOS window (`observe_native_ui`, `capture_native_ui_scenario`)

OBSERVED. This is REA's only surface that drives arbitrary installed native GUI software.
- **Binding.** The tool is "bound to the active Mach-O target" (`src/contracts/native/nativeToolContracts.ts:88`). The caller must `open_binary` the app's executable first. The Swift helper then checks the following (`bridge/native/ReaNativeUI.swift:45-78`):
  - `NSRunningApplication(pid)` exists and its realpath equals the active target;
  - `launchDate` equals the expected launch time ("process-replaced" otherwise);
  - the executable's SHA-256 equals the target digest (256 MiB cap);
  - exactly one CG window with that `window_id` exists, owned by that PID at layer 0;
  - and, for AX, exactly one AX window has the same geometry ("ambiguous-window" fails closed, `:84-93`).
- **Permissions.** `CGPreflightScreenCaptureAccess()` and `AXIsProcessTrusted()` are checked, and failure returns actionable remediation. There is "no broad capture fallback" and no automatic prompt (`ReaNativeUI.swift:79-85`; `docs/native-investigation.md:1050-1055`).
- **Observation.** The AX tree is walked to depth 32. Each node records path, role, title, value, actions and child count, and AX values JSON cannot represent are recorded as gaps (`ReaNativeUI.swift:121-141`). Screenshots come from a ScreenCaptureKit single-window filter (`:153`), scaled to ≤2048 px. Output is budgeted at 64 MiB (`docs/native-investigation.md:1085-1090`).
- **Actions.** Elements are addressed by **AX child-index path**, re-resolved per action. Missing paths fail with "element-missing" (`ReaNativeUI.swift:96-104`). Supported actions are `click` → `AXPress`, `scroll` → `AXIncrement`/`AXDecrement`, and `key-entry` → set `AXValue` (`:111-113`). There are also bounded `wait` steps, capped at 180 s with no override (`docs/native-investigation.md:1081-1084`). "No global event injection is used. Unsupported AX actions fail explicitly" (`:1073-1075`).
- **Result.** An ordered before/after capture per step, plus `restore: "leave-as-is"` and explicit limitations, including that "a failed after-capture can follow a completed action, so failure does not prove absence of application effects" (`src/native/NativeUiObservation.ts:304-315`). Partial reports survive cancellation (`:318-331`).
- **Helper.** One owned `swiftc`-compiled helper per scenario, with its compiler cache in a removable root (`src/native/NativeUiHelperRuntime.ts:25,55-81`). "It does not launch or own the selected application" (`docs/native-investigation.md:1090-1091`).
- **Related static tool.** `trace_native_ui_action` (`evidence` effects, `toolEffects.ts:510`) maps authored UI to handler code statically (`docs/mcp-contracts.md:948-952`). It is not a driver.

---

## 4. Desktop capture: the session lifecycle (Linux)

OBSERVED (`src/application/desktop/DesktopApplicationCapture.ts`, `src/desktop/DesktopApplicationSession.ts`).
1. **Admission, before any side effect** (`DesktopApplicationCapture.ts:173-218`; `DesktopCaptureRefusals.ts:134-189`). REA refuses when:
   - the host is not Linux;
   - `DISPLAY` is unset;
   - `xprop` is missing;
   - `HOME` is not absolute;
   - the output folder is not empty;
   - the WM does not advertise `_NET_CLIENT_LIST` (`no_client_list`);
   - the window list is unreadable;
   - **a window of the app is already open** (`alreadyOpen`). The reason: "A launch beside a running instance can hand off to it, and that instance's unsaved state would be missing from the baseline" (`docs/desktop-application-capture.md:50-53`; `DesktopApplicationSession.ts:88-108`).
   
   Native Wayland windows are invisible. XWayland windows are visible (`docs/desktop-application-capture.md:17-24`).
2. **Baseline** walk + content retention (`DesktopApplicationCapture.ts:242-244`).
3. **Launch.** `spawnOwnedProviderProcess` with a fresh `randomUUID()` run token in the environment, in its own process group. `expectedCommand: null`, because "launchers often exec through env, a shell or a wrapper script" (`DesktopApplicationSession.ts:267-294`, `:306`). Only 8 KiB of stderr is kept, and no stdout (`:30,291`).
4. **Watch loop.** Every 500 ms (`DesktopApplicationCapture.ts:52`) REA samples:
   - processes;
   - X11 windows (`xprop -root _NET_CLIENT_LIST`, then `xprop -id` for `WM_CLASS, _NET_WM_NAME, WM_NAME, _NET_WM_PID, _NET_WM_WINDOW_TYPE`; `src/desktop/X11WindowObserver.ts:78-116`, `src/domain/desktop/x11Windows.ts:11-17`);
   - open files via `/proc/<pid>/fd` (`src/desktop/DesktopSessionObservations.ts:267-280`).
   
   It handles one stdin request per sample "so queued lines cannot starve observation" (`DesktopApplicationSession.ts:202-206`). A window "belongs" by owner PID or class name. A matching window's PID *joins* the app's process set, so a D-Bus-activated service is followed but **never signalled**, because it carries no token (`docs/desktop-application-capture.md:245-255`).
5. **Control channel** (`src/desktop/DesktopSessionRequests.ts:12-49`):
   - an empty line or `checkpoint [name]` takes a checkpoint;
   - any other text names a checkpoint;
   - `stop` or EOF stops.
   
   Reading pauses at 16 queued lines and resumes at 4. Reserved names are `baseline|stop|time-limit|exit`. Duplicates are refused and the session continues (`docs/desktop-application-capture.md:57-66`).
6. **End conditions.** A stop request, `--max-duration-seconds` (default 3600), or "application exited". The exit test passes after two consecutive samples with the launched process gone, no token carrier left, and a *read* window list showing none of the app's windows. An unreadable list counts as unknown, not as empty (`DesktopApplicationSession.ts:185-216`).
7. **Release.** One SIGTERM each to the process group and token carriers ("some applications treat a second one as a forced exit"), a 5 s grace, then the supervisor stops the remaining token carriers, retried once. An unverified cleanup is returned as a labelled `AnalysisCleanupLease`, and the command fails naming the token and process group (`:218-265`; `docs/desktop-application-capture.md:78-93`). Ctrl-C writes the capture with `end_reason: cancelled` and exit 130. A failure before `capture.json` removes the retained content (`DesktopApplicationCapture.ts:319-333`).
8. **Checkpoints.** A walk compares `(size, mode, mtime_ns, dev:ino)` per path using `lstat`. It never crosses filesystems, and mount points are listed as limitations (`src/desktop/HomeStateWalker.ts:38-48,93-96`). Created/deleted is claimed only where the parent directory was fully listed. Otherwise the path is "unconfirmed" (`docs/desktop-application-capture.md:268-276`). Content is stored once per SHA-256 under `content/`, written `0o600` with `wx`, in a `0o700` folder (`src/desktop/DesktopContentStore.ts:64,133-146`).
9. **Evidence.** `predicateType: "rea.desktop-application-capture"`, `confidence: "observed"`, `authority: "controlled-replay"`, `environment.isolation: "none"` (`DesktopApplicationCapture.ts:145-170`). Limitations are generated from facts (`:87-143`).

OBSERVED explicit non-claims:
- "This is not a sandbox… treat [the capture folder] as private data" (`docs/desktop-application-capture.md:10-13`).
- "A change is not attributed to the process that made it" (`:297-302`).
- "Clicks and keystrokes are not recorded. Window titles are sampled every half second" (`:304-305`).

---

## 5. Desktop state: roles, typed views, diffs

OBSERVED (`docs/desktop-application-state.md`; `src/application/desktop/DesktopApplicationState.ts`; `src/domain/desktop/stateRoles.ts`).
- **Attribution reasons** per path: `state_directory`, `opened_by_application`, `named_in_window_title` (whole-name match, so `day.txt` does not match `today.txt - Notes`), `selected_path`, `dconf_application_keys`, `desktop_service` (`docs/desktop-application-state.md:24-40`).
- **Role precedence**: `application_state` → `document` → `application_state` (other open file) → `shared_desktop` → `unrelated` (`src/domain/desktop/stateRoles.ts:200-211`). The shared desktop paths are hard-coded: `.cache`, `.config/dconf`, `.config/xfce4`, `.config/gtk-3.0/4.0`, `.config/pulse`, `recently-used.xbel`, `gvfs-metadata`, Trash, `.Xauthority`, `.ICEauthority` (`:24-36`).
- **Typed views** per version:
  - sqlite: tables, columns, PK, ≤500 rows, read from a private copy with `-wal`/`-journal` staged. A hot journal is refused, and so is a database whose logs were not retained;
  - dconf: through `dconf dump /` against a private file profile;
  - json: integers above 2^53 are refused, not rounded;
  - xml: DOCTYPE refused;
  - keyfile;
  - text: ≤5,000 lines;
  - binary.
  
  (`docs/desktop-application-state.md:54-85`.)
- **Diff kinds**: `lines` (multiset), `values` (JSON pointers), `keys` (`group/key` with `~0/~1`), `rows` (`table:` + PK), `content`, `unreadable`, `created`, `deleted` (`:87-103`).
- Evidence confidence is `inferred` and authority `analyst-inference`, because roles are heuristic (`DesktopApplicationState.ts:466-467`; doc `:14-17`).

INFERRED: this is a **generic "what did the app change" model for file-backed state**. It is the closest thing in REA to an app-agnostic post-condition observer, and it needs no app-specific adapter.

---

## 6. Desktop tasks: declarative, validated verifiers

OBSERVED (`docs/desktop-application-tasks.md`; `src/domain/desktop/appTask.ts`, `appTaskChecks.ts`, `appTaskPython.ts`; `src/application/desktop/DesktopTaskExport.ts`).
- **Division of labour**: "An agent, not REA, writes the prompt and the checks… REA runs no model and no code the agent wrote: the checks are declarative" (`docs/desktop-application-tasks.md:4-10`).
- **Draft.** `describe-desktop-application-task` returns `owned_paths`, the files whose first and last versions differ, and suggested checks derived from the net change of `document`-role files, falling back to app state files. Values that look like app-stamped times are dropped (`:21-50`). The guidance warns that format-rewrite noise must be pruned by the author, giving the LibreCAD DXF R12→2007 example (`:52-56`).
- **Check kinds**: `file_present|file_absent|lines_present|lines_absent|line_count|value_equals|value_absent|row_present|row_absent|row_count`, 1–32 per task (`:82-93`). Semantics:
  - an absent file reads as empty;
  - a viewless file fails all checks except presence;
  - equality is JSON equality;
  - RFC 6901 pointers are used;
  - **no regular expressions**, because "a pattern run on what the agent wrote could stall the verifier, and JavaScript and Python read some patterns differently";
  - a check decided by what a truncated view leaves out *fails* (`:95-114`).
- **Refusals.** A check is refused, with the check named, if it points outside the owned paths, uses a leading-slash/`.`/`..` path, a pointer into text, the empty pointer, a value naming the captured home, or an ISO-8601 datetime value (`:116-121`).
- **Validation gate.** "It refuses a task whose every check passes on the seed, and a check that fails on the end state" (`:126-131`; `src/application/desktop/DesktopTaskExport.ts:213-226`).
- **Exported run.** `tasks/<name>.json` is `deploy_env → deploy_agent → add_skills → start_computer_recording → prompt_agent → env_outcome_verifier → end_computer_recording` (`src/domain/desktop/appTaskPython.ts:226-262`). `verify.py` finds the app among the computer's child environments, calls its `data/get` "under the run's grant of the computer's control token", and **raises if unreachable, so a failure does not count as a score** (`appTaskPython.ts:208-218`; doc `:141-144`). One evaluator is implemented in both TS and Python with identical semantics (`docs/desktop-application-tasks.md:7-10`).
- **Solve rate**: "REA does not run agents". Run k=3 times and count `passed`. A run that is `failed` is rerun, not counted as a miss (`:147-164`).

INFERRED answer to the salvaged open question: write-confirmation *does* transfer to desktop apps. It just moves from "a later recorded read confirms the HTTP write" to "**declarative post-condition checks over a typed view of persisted state, admitted only if they fail on the seed and pass on a demonstrated end state**". It does not cover in-memory or unsaved UI state, which this model cannot see.

---

## 7. Desktop export: agentenv `computer_app` bundle

OBSERVED (`docs/desktop-application-export.md`; `src/domain/desktop/appExport.ts`, `appExportPython.ts`, `appExportInstall.ts`; `src/application/desktop/DesktopApplicationExport.ts`, `DesktopExportInstall.ts`).
- **`env.toml`**: `type = "computer_app"`, `computer = "ubuntu-24.04"`, `driver = "reference"` (`src/domain/desktop/appExport.ts:279-286`).
- **Class decorators.** `@computer_app(supports=[{os: linux, display: [x11], arch}])`, then `@linux(packages=Packages(apt=…), sources=[AptSource(...)], downloads=[Download(url, sha256)], run=[RunScript], launch=Launch(at="boot", command, window={class}, ready=Ready(window=True)), paths=PATHS, skill="SKILL.md")` (`:374-384`).
- **Readiness.** `@ready_check main_window_shown`: a window of the captured class is listed in the taskbar and is not a splash or dialog (`src/domain/desktop/appExportPython.ts:2-7,64-82`).
- **Data hooks.** The seed is written once per boot. `keep_initial(paths)`. `data/add|reset` call agentenv helpers. `data/get` returns the owned files plus typed views computed in a worker thread. Declaring any hook switches off agentenv's default data plane (`appExport.ts:288-313`; doc `:123-128`). The home path is respelled in seeded text (`:130-139`). dconf keys are compiled offline because D-Bus is not up yet (`:141-149`).
- **Refusals** (`src/application/desktop/DesktopExportInstall.ts:44-69`; `src/domain/desktop/appExportInstall.ts:103-119`; doc `:63-96`):
  - snaps, flatpaks and AppImages;
  - packages from unverified-key repositories, ESM, credentialed URIs, or non-http(s) URIs;
  - packages whose arch restriction excludes the host's;
  - a capture that "saw no application window, so the export cannot match the app's window to tell when it is ready".
- **Electron.** `--no-sandbox` is added because the container lacks user namespaces, and this is disclosed in the README and limitations (doc `:108-112`).
- **Rights.** "you must be allowed to use that copy, and the package's licence… is yours to check" (`:181-185`). "The seed is the user's real data. Keep the bundle local and do not publish it" (`:221`). Evidence is `confidence: "derived"`, `authority: "analyst-inference"` (`DesktopApplicationExport.ts:501-502`).
- **SKILL.md** for the agent: "Work through the application's own window and dialogs; the result is read from those files" (`appExport.ts:399-400`).

INFERRED: **the actual GUI driving inside a desktop agent environment is agentenv's `reference` driver plus the computer-use agent, not REA.** REA's contribution is: install recipe + readiness predicate + seed + state read-back + validated verifier.

---

## 8. Agent environments from recorded web captures

### 8.1 Pipeline

OBSERVED:
- `rea inspect-web-network-capture` (HAR) or `capture-browser-scenario`;
- then the `inspect_analysis_view` `environment-manifest` view;
- then `rea environment-serve manifest.json --evidence capture.json`.

The one-step equivalent is `rea environment-build <capture> <dir>`, byte-identical to the composed steps (`docs/agent-environments.md:10-77`). The MCP equivalent is `export_environment_bundle`, which builds only and does not serve (`:218-233`).

### 8.2 Manifest = synthesized tool surface

OBSERVED:
- **Tool shape.** One tool per method + path template on the primary origin, with inferred path params, observed query names, value-free body shapes, and citations to parent records. No bodies or headers (`:368-378`).
- **Templating.** Uses opt-in rules `after-uid-literal`, `opaque-token`, `sibling-variance`, `prefix-variance` (`:79-84`).
- **Pairing check.** The manifest's `parent_evidence_id` must equal the served capture, else `invalid_request` (`:375-378`).
- **Catalog budget.** The catalog is admitted under a 1 MiB serialization reservation before binding. Fields are "never silently omitted" (`:387-394`).
- **Annotations.** `readOnlyHint` for safe methods and query-POSTs, `destructiveHint` only for DELETE, `idempotentHint` for safe/PUT/DELETE, and `openWorldHint: false` for everything because "answers come from the capture" (`:603-612`).
- **Schemas** close with `additionalProperties: false`, so "a misspelled parameter never degrades into an omitted one" (`:461-463`, `:624-626`).

### 8.3 Replay vs. state

OBSERVED:
- **Replay** picks the cited record with the most path/query matches and reports `replay.exact_match`. "Request bodies passed to POST, PUT and PATCH tools are accepted and ignored: nothing is written" (`:400-463`). "Replay is not emulation" (`:465`).
- **State (default).** At startup the `environment-state` view derives entities, documents and per-read render plans, kept only where they reproduce the recordings (`:480-492`, `:643-652`). One live state is shared across MCP sessions and the data plane (`:651`). If the state is unavailable, the server logs a warning and serves replay (`:654-672`).
- **Writes** are served only when the write plan was confirmed by later reads (`:735-740`). Withheld writes carry reason + hint: `no-state-target`, `no-call`, `body-unusable`, `no-observation`, `unexplained-change`, `contradicted`, `side-effect`, `unconfirmed`, `incomplete-entities`, `unknown-clock` (`:175-186`).
- **Effects modelled**: create/update/action (set/toggle)/delete/bulk/document. Generated values are deterministic: the clock starts at `write_clock` and advances +1 s per write, and UUIDs derive from the evidence id, tool, field and count (`:742-793`).
- **Unmodelled**: side effects on counters, relations and copies (`:937-947`).

### 8.4 `environment-serve` (the serve command)

OBSERVED (`src/cli/environmentCommands.ts:670-707`; `src/server/environment/serveEnvironment.ts`):
- **Endpoint.** `--host/--port` fall back to `MCP_HOST/MCP_PORT`, then `127.0.0.1` and an ephemeral port. The bound URL is logged on stderr as JSON. `ENVIRONMENT_NAME` overrides the name, and a blank value is refused (`docs/agent-environments.md:496-516`).
- **Routes.** `POST /mcp`, `GET /.well-known/agent-env.json` (the card), `POST /agentenv` (JSON-RPC `data/reset|add|get`; in replay, every method is `-32601`) (`:522-528`; `serveEnvironment.ts:66-68`).
- **Hardening.** One MCP client at a time, reusing REA's streamable-HTTP transport and Host-header contract (`:512-513`). A non-loopback bind refuses to serve without `MCP_ALLOWED_HOSTS` (`serveEnvironment.ts:430-431,483-485`; `src/config/mcpHttpEndpoint.ts:13,35-48`). Bodies are capped at 4 MiB. Batches hold ≤100 messages (`serveEnvironment.ts:87`). Responses are capped at 64 MiB, which is twice the 32 MiB state intake (`:152-153`).
- **Untrusted content.** Every tool result carries `_meta.provenance.untrusted_content: true, source: "analysis_target"`. Server instructions are `UNTRUSTED_CONTENT_INSTRUCTIONS + REPLAY_CATALOG_INSTRUCTIONS`, warning that tool names, descriptions and parameter names are captured data too (`serveEnvironment.ts:104-145`; `src/server/environment/targetContentTransport.ts:89`; doc `:470-478`).
- **`data/add`.** Takes inline data or a `file://` URI (local only; UNC paths and other hosts refused) or an `https://` URI (no redirects, 30 s). Errors never echo the URI or content (`docs/agent-environments.md:806-825`).
- **Isolation.** "No process is launched and nothing is written to disk. No outbound network request is made unless a `data/add` call names an `https://` uri" (`:1211-1214`).

### 8.5 Export and fidelity

OBSERVED:
- **Bundle layout.** `envs/<n>/Dockerfile` (pinned `node:24.22.1-bookworm-slim` + `rea-agents@<version>`), `manifest.json`, `evidence.json`, `verify.py` (smoke: tool count + one no-arg GET status), and `tasks/smoke.json` (`:978-987`, `:1038-1053`).
- **Container.** It binds `0.0.0.0:18765` with `MCP_ALLOWED_HOSTS` and runs as non-root `node` (`:1022-1036`). The README carries a rights note (`:1064-1065`).
- **Fidelity.** `environment-compare` replays offline *probe captures recorded against the live app* through the env. It maps ids by natural key and compares clocks by format, then reports agreement per probe and tool, with diff categories `status|missing-field|extra-field|value|length|order|clock-format|refused|not-served|no-tool|unmapped-id` (`:1067-1184`). "The command makes no network call: whoever records the probes drives the application with its own credentials" (`:1079-1080`).

---

## 9. Electron and browser scenarios (recap, focused on driving)

See `rea-runtime-observation-surfaces.md` §4–§5 for the full analysis. Additional points relevant here:
- OBSERVED: the browser locator vocabulary is `test_id`, `role` (14 ARIA roles: button, checkbox, combobox, dialog, link, listbox, menuitem, option, radio, slider, spinbutton, switch, tab, textbox) + `name`, or `css` (`src/domain/browserScenarioValues.ts:321-350`). INFERRED: this is a good minimal **role+name locator** model to reuse across UIA/AX/AT-SPI.
- OBSERVED: the `request` step was added *specifically* so agents can record API writes for environments. It sends a same-origin `fetch` with the session's cookies and CSRF values, follows no redirects, and refuses other origins (`docs/specs/agent-environments/environment-prompt/spec.md:12-45`; `docs/agent-environments.md:323-340`).
- OBSERVED: Electron active scenarios are "not a passive CDP observation… lifecycle ownership does not make it a sandbox" (`docs/electron-observation.md:167-168`). If the `EnableNodeCliInspectArguments` fuse is disabled, REA refuses before launch (`:309`). INFERRED: REA treats vendor hardening as a hard stop, which matches agentic-anything's scope rule.

### `src/windows/` and `src/captures/`, for completeness

- OBSERVED: `src/windows/` is **process supervision for REA-owned helpers**, not GUI automation:
  - a validated native addon exposes Job Objects with `jobAssigned`/`killOnOwnerClose` set to `true` and optional commit limits (`src/windows/WindowsOwnedProcess.ts:14-33`);
  - private-DACL runtime roots (`src/windows/WindowsPrivateRuntime.ts:59-90`);
  - NTFS file identity (`src/windows/WindowsNativeLoader.ts:21-44`).
  
  There is no UIA, no window enumeration, and no input. See `salvaged/engine-bridges-and-process-supervision.json`.
- OBSERVED: `src/captures/` is the tshark packet-capture provider (`TsharkCaptureProvider.ts`, `parsePacketOutput.ts`). It is relevant only as another source of API traffic for environment manifests (UNKNOWN whether pcap feeds the manifest view: the doc says only HAR and browser scenarios are accepted, `docs/agent-environments.md:61-77`).

---

## 10. Observe vs. drive ledger

| Subsystem | Launches target | Sends input to target | Mutates target state | Restores state | Who drives |
|---|---|---|---|---|---|
| `inspect_desktop_application` | no | no | no | n/a | — |
| `capture-desktop-application` | **yes** (owned, run token) | **no** | the app does, under the user | no (real files) | human or external `xdotool` + FIFO (`docs/desktop-application-capture.md:95-100`) |
| state / task draft / export | no | no | no | n/a | — |
| exported `computer_app` bundle | agentenv does, at boot | agentenv `reference` driver | inside the container | `data/reset` (files; dconf not restored, `docs/desktop-application-export.md:148-149`) | computer-use agent |
| `observe_native_ui` | no (helper only) | no | no | n/a | — |
| `capture_native_ui_scenario` | no | **yes**: AX press/increment/decrement/set-value | **yes** | **no** ("leave-as-is") | the calling agent, as a batch |
| `capture_browser_scenario` | browser (launch mode) or attach | **yes** (Playwright) | **yes** (incl. `request`) | provider temp profile deleted; app data no | the calling agent, as a batch |
| `capture_electron_scenario` | **yes** | **yes** (Playwright click + synthetic events) | **yes** (shell/permissions blocked) | no | the calling agent, as a batch |
| `environment-serve` | no | no (only simulated) | in-memory state only | `data/reset` | agent against the simulation |
| `environment-compare` | no | no | no | n/a | human recorded the probes |

---

## 11. Authority and safety boundaries (as implemented)

OBSERVED mechanisms, in order of strength:
1. **Interface placement.** Long-running, launching or stdin-controlled desktop capture is CLI-only. MCP gets only bounded, deadline-limited batch tools (§2).
2. **Effect declaration.** Each tool has a `ToolEffects` row, and the MCP hints are derived from it. Foreign-code execution or data discard means destructive (`src/contracts/toolEffects.ts:770-786`, destructiveHint at `:782`). Environment tools carry per-HTTP-method hints instead (`docs/agent-environments.md:603-612`).
3. **Evidence self-description.** `authority ∈ {shipped-artifact, controlled-replay, historical-reference, external-service, analyst-inference}` and `isolation ∈ {none, process, container, virtual-machine}` (`src/domain/evidence.ts:71-84`). Desktop capture honestly records `isolation: "none"`.
4. **Pre-side-effect refusals.** Wrong host or display, an already-running instance, an unreadable window list, a non-empty output folder, ambiguous identity, missing OS permission (no prompt, no broad fallback), disabled inspector fuse.
5. **Identity binding.** Executable SHA-256 + launch time + PID + window ID + unique AX geometry (macOS). Process start time defeats PID reuse (Linux, `docs/desktop-application-capture.md:241-243`). Manifest↔capture `parent_evidence_id` binding.
6. **Ownership-scoped termination.** REA only signals processes carrying its run token or in its group. A "foreign" instance whose window matched is followed but never killed (`src/desktop/DesktopApplicationSession.ts:218-223`).
7. **Data hygiene.**
   - Credential stripping from apt URIs.
   - Vendor `.deb` never copied.
   - Capture folder `0700` / files `0600`.
   - Standard auth headers excluded from HAR inspection.
   - Credential-named-member warnings that never claim the data is clean (`docs/agent-environments.md:188-216`).
   - Secrets redacted, and screenshots withheld when secrets could be in pixels (browser).
8. **Untrusted-content labelling** of every replayed result, plus server instructions (§8.4).
9. **Explicit non-claims.** "not a sandbox", "leave-as-is", "Replay is not emulation", "does not prove absence of application effects".
10. **Rights statements**: licence checks, test instance only ("Choose an instance where the agent is allowed to create, edit, and delete data", `docs/agent-environments-quickstart.md:16-17`), don't publish seeds.

OBSERVED absences:
- no per-action allow/deny;
- no confirmation or elicitation (consistent with the PR #555 removal noted in `PLAN.md`);
- no rate limits on UI actions beyond deadlines and budgets;
- no kill switch beyond SIGINT and cancellation;
- no automatic restore of real-app state.

---

## 12. Exactly where REA stops, and what an observe+DRIVE framework must add

| # | REA stops at | Evidence | New machinery agentic-anything needs |
|---|---|---|---|
| G1 | **Batch scenarios only.** Actions are declared up front and run in one call. The agent cannot observe, decide, act, and observe again inside one session handle | `capture_native_ui_scenario` steps array (`nativeToolContracts.ts:23-26,92-93`); browser `actions[]` | A **session-scoped drive loop**: `open_session(target)` → `observe()` / `act(step)` / `close()`, with a persistent handle, identity re-check per step, a per-step deadline, and the same before/after Evidence per call |
| G2 | **Linux desktop: observe-only, X11-only.** No input injection, no a11y tree, `xprop` polling at 500 ms, no Wayland | `docs/desktop-application-capture.md:17-24,304`; `X11WindowObserver.ts` | Adapters for **AT-SPI2** (Linux a11y tree + actions), **UIA** (Windows), XTest/libei/uinput input as a lower rung, Wayland portals (see `survey-accessibility-trees.md`, `survey-os-automation-apis.md`) |
| G3 | **macOS AX actions limited** to press, increment/decrement, and set-value. Child-index paths are brittle, and the docs admit "stale or ambiguous paths fail" | `ReaNativeUI.swift:96-113`; `NativeUiObservation.ts:312` | **Semantic locators** (role + name + ancestor chain, like the browser `role` locator) resolved per action. More AX actions (ShowMenu, Confirm, Cancel, Raise, focus). Keyboard/menu-bar invocation. An explicit opt-in rung for CGEvent input with a stronger authority level |
| G4 | **Windows: no GUI surface at all** | `src/windows/*` is job/DACL supervision only | UIA tree + patterns (Invoke/Value/Toggle/ExpandCollapse), window identity (HWND + PID + image hash + process creation time), reusing REA's Job Object ownership for launched apps |
| G5 | **No live app API synthesis.** Environments *simulate* from recordings. The live app is never called through synthesized tools | `docs/agent-environments.md:22-23,1211-1214` | A **live-proxy mode**: the same manifest-derived tool catalog, but forwarding to the real app (authorized instance) with per-tool authority, a write confirmation policy, and read-back verification. Simulation stays as a dry-run/sandbox lane |
| G6 | **No per-action authority or confirmation.** Effects are per *tool*, not per *step*, so one scenario call is fully destructive | `toolEffects.ts:596-606` | **Per-step effect classification** (read / UI-state / data-mutating / irreversible / external), policy evaluation before each step, MCP elicitation or out-of-band confirmation for irreversible steps, and allow/deny lists keyed by app identity and locator |
| G7 | **No restore.** Native/Electron/browser scenarios leave state as-is. Desktop capture runs on the user's real home | `NativeUiObservation.ts:307,313`; capture doc `:10-13` | **Checkpoint/restore** using REA's own capture machinery (baseline walk + content store + typed state views) to snapshot owned paths before a drive session and offer `restore`. A container/VM lane (agentenv-style) for risky drives. `Evidence.isolation` set truthfully |
| G8 | **Post-conditions only on files.** Task checks read persisted files. Unsaved or in-memory UI state is invisible | `docs/desktop-application-tasks.md:76-80` | **Multi-source post-conditions**: a11y-tree predicates (element with role/name/value), app API reads, file views. The same declarative and no-regex discipline, with the seed-fails/end-passes admission gate |
| G9 | **No attribution of effects to actions.** Changes are compared between checkpoints and not attributed to a writer | capture doc `:297-302` | Per-action checkpoints (cheap: scope the walk to owned paths), plus optional fanotify/ETW/FSEvents attribution, so each action record carries its observed side effects |
| G10 | **Driver is external or third-party.** `xdotool` + FIFO (Linux capture); agentenv `reference` driver (export) | capture doc `:95-100`; `appExport.ts:284` | First-party driver adapters behind the same contract, so the capture → task → verify pipeline can be *agent-driven end-to-end* and the recording of actions becomes part of the Evidence (REA records none) |
| G11 | **Discovery is classification, not capability enumeration.** It says "Electron candidate", not "these 40 commands exist" | `docs/desktop-applications.md:94-120` | A **control-surface enumerator**: from the runtime family, probe the actual surfaces (CDP port, AppleScript dictionary `sdef`, COM type library, D-Bus introspection, CLI `--help`, plugin API) and emit typed candidate tools with authority levels, following REA's "next_probes" shape |

---

## What to distill for agentic-anything

1. **Adopt the "four meanings of app" split as first-class target kinds.** Use `install_dir` (static classify), `installed_app` (launchable, by desktop-entry / bundle id / AppUserModelID / executable), `running_window` (PID + window + image hash + start time), and `service_api` (origin + manifest). Each kind carries its own identity schema and refusal set. This comes from §3.
2. **Reuse REA's identity-binding recipe verbatim for every drive call.** Check: executable realpath equals the declared target, SHA-256 of the image, process launch/creation time, PID, window ID owned by that PID at a normal layer, and a unique a11y-window geometry match. Fail closed on ambiguity, and re-check before every step (`ReaNativeUI.swift:45-93`).
3. **Launch ownership via run token + process group + start time.** Never signal a process you did not start, even if its window matches (follow, don't kill). Send one SIGTERM, wait 5 s, sweep token carriers, retry once, and return an explicit cleanup lease on failure (`DesktopApplicationSession.ts:218-294`). Refuse to launch beside an already-running instance when state fidelity matters.
4. **Keep long-running, launching sessions out of a flat tool list, or wrap them in an explicit session resource.** REA keeps desktop capture CLI-only. agentic-anything needs it over MCP, so model it as `session/open` → handle → `session/step` → `session/close`, with deadlines, budgets and a server-side kill on disconnect.
5. **Make every action return before/after observations plus an honest outcome**: `completed | action_failed | capture_failed_after_action`. Include the limitation "failure does not prove absence of effects" (`NativeUiObservation.ts:294-313`), and keep partial reports on cancellation.
6. **Closed action vocabulary + role/name locators across backends.** Start from REA's browser set (`click/fill/press/select_option/wait_for/wait_for_timeout`) and its 14 ARIA roles + `name` (`browserScenarioValues.ts:321-350`). Map them to AX/UIA/AT-SPI patterns. Unsupported means explicit failure, never a silent fallback to coordinate input. Coordinate/global input is a separate, higher authority rung.
7. **Typed state views + diff as the generic post-condition observer.** Port REA's view formats (sqlite via a private copy with WAL staging, json without lossy numbers, xml without DOCTYPE, keyfile, dconf via a private profile, text, binary) and diff kinds (`lines/values/keys/rows/content`). Add an a11y-tree view as one more format (§5).
8. **Verifier admission gate.** A task or post-condition is admitted only if it **fails on the seed and passes on a demonstrated end state**. Checks are declarative, with no regex, and a check decided by omission fails on truncated views. Make an unreachable app a run failure, not a score. Run the same evaluator in the server and the exported verifier (§6).
9. **Confirmed-write admission for synthesized API tools.** When generating tools from traffic, *serve a write only if a later read confirmed its effect*. Otherwise withhold it with `{reason, hint}` drawn from REA's ten-reason table (`docs/agent-environments.md:175-186`). Close schemas with `additionalProperties: false`. Derive MCP hints from the HTTP method. Label all captured strings as untrusted content in both `_meta` and server instructions.
10. **Simulation lane + fidelity metric.** For any service-backed app, offer an offline simulated environment (deterministic clocks and ids, `data/reset|add|get`). Offer `compare` against probe recordings from the live app, with categorized diffs, as the regression and safety lane before live driving (§8.5).
11. **Use Evidence `isolation` and `authority` truthfully on every drive record** (`none | process | container | virtual-machine`). The policy engine can then require `container`/`virtual-machine` for actions whose effect class is irreversible.
12. **Use agentenv-compatible export as an output format.** REA's desktop export already gives a reproducible container recipe (verified apt sources, a hash-pinned `.deb` download, a readiness predicate by window class with splash exclusion, seed + `keep_initial`, and a SKILL.md). agentic-anything can emit the same bundle to give users a sandbox for rehearsing agent tasks against their own apps.
13. **Treat vendor hardening as a hard stop**, as REA's fuse gate does. If an app disables its automation or inspection surface, refuse with a pointer to supported surfaces instead of working around it.

---

## Open questions

1. Will agentic-anything support Wayland-native apps? REA refuses them. AT-SPI2 still works under Wayland, but input injection needs portals or libei. What authority level does that get?
2. Should the per-step drive loop reuse REA's macOS Swift-helper model (compile once per scenario) or keep a persistent helper per session? A persistent helper is faster but holds TCC-granted authority for longer.
3. AX child-index paths are brittle. What is the canonical locator: role+name+ancestor path, platform automation IDs (UIA `AutomationId`, AX identifier), or a hybrid with a re-resolution score? How should ambiguity surface to the agent?
4. Can REA's checkpoint walk be cheap enough to run after every action (scoped to owned paths), or is per-action attribution only feasible with fanotify / ETW / FSEvents? What is the latency budget?
5. For live-proxy API tools (G5), how should credentials be held? REA's environment never needs them; the probe recorder "drives the application with its own credentials". Where do secrets live, and how is redaction applied to live responses?
6. Is the agentenv-framework licence and API stable enough to target as an export format? (UNKNOWN: not checked here. REA pins 0.9.1334 / 0.9.1431, `docs/agent-environments-quickstart.md:22`, `docs/desktop-application-export.md:222-223`.)
7. How is a confirmation prompt (MCP elicitation) reconciled with batch scenarios? Do we forbid irreversible steps in batches, or pause mid-batch?
8. `trace_native_ui_action` statically maps UI labels to handlers. Could it pre-classify the effect of an AX action (e.g. "this button calls `-[NSDocument saveDocument:]`") to inform the per-step authority decision? Not verified here.
9. Does REA's untrusted-content labelling cover screenshots and a11y text returned by `observe_native_ui`? `TARGET_CONTENT_FREE_TOOLS` implies these tools are marked untrusted by default (`src/contracts/toolEffects.ts:800-816`), but the exact `_meta` on native UI results was not traced.

---

## Sources

- REA docs: `docs/desktop-applications.md`, `docs/desktop-application-capture.md`, `docs/desktop-application-state.md`, `docs/desktop-application-tasks.md`, `docs/desktop-application-export.md`, `docs/apple-application-analysis.md`, `docs/agent-environments.md`, `docs/agent-environments-quickstart.md`, `docs/browser-scenario-contract.md`, `docs/electron-observation.md`, `docs/native-investigation.md` (1048–1092), `docs/web-network-captures.md` (478–517), `docs/mcp-contracts.md` (948–952), `docs/specs/agent-environments/environment-prompt/spec.md`, `.agents/skills/reverse-engineer-anything/references/environments.md` (291–347).
- REA source: `src/application/desktop/{DesktopApplicationCapture,DesktopCaptureRefusals,DesktopCaptureScope,DesktopApplicationState,DesktopApplicationExport,DesktopExportInstall,DesktopTaskExport,DesktopTaskDraft}.ts`; `src/desktop/{DesktopApplicationSession,X11WindowObserver,DesktopSessionRequests,DesktopSessionObservations,LinuxApplicationInventory,HomeStateWalker,DesktopContentStore}.ts`; `src/domain/desktop/{x11Windows,stateRoles,appExport,appExportPython,appExportInstall,appTask,appTaskPython,linuxPackages,desktopEntry}.ts`; `src/contracts/{toolEffects,desktopApplicationToolContracts}.ts`; `src/contracts/native/nativeToolContracts.ts`; `src/native/{NativeUiObservation,NativeUiHelperRuntime}.ts`; `bridge/native/ReaNativeUI.swift`; `src/server/environment/{serveEnvironment,targetContentTransport}.ts`; `src/cli/environmentCommands.ts`; `src/config/mcpHttpEndpoint.ts`; `src/domain/{evidence,browserScenarioValues}.ts`; `src/browser/PlaywrightElectronActiveActions.ts`; `src/windows/{WindowsOwnedProcess,WindowsNativeLoader,WindowsPrivateRuntime}.ts`; `src/captures/`.
- Prior agentic-anything reports: `rea-runtime-observation-surfaces.md`, `rea-mcp-server-adapter.md`, `rea-contracts-and-evidence-model.md`, `salvaged/tool-design-philosophy-and-verification.json`, `salvaged/engine-bridges-and-process-supervision.json`, `survey-accessibility-trees.md`, `survey-os-automation-apis.md`, `survey-computer-use-vision-agents.md`.
- External (SECONDARY, not fetched): agentenv-framework, https://github.com/scaleapi/agentenv-framework (cited by `docs/agent-environments-quickstart.md:20`).
