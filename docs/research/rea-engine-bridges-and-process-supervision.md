# REA engine bridges and process supervision

Research date: 2026-11-05

Source read: local fork `/Users/nhanclie/rea` (rea-agents 6.8.0, `package.json`; HEAD `3484432fb`). REA paths are relative to that checkout and cited as `path:line`. This report replaces a lost full report; it checks and extends `docs/research/salvaged/engine-bridges-and-process-supervision.json`. Related reports, referenced and not repeated here: `rea-runtime-observation-surfaces.md` (the authority ladder, the effect vocabulary, Frida/CDP), `rea-contracts-and-evidence-model.md` (the Evidence envelope), `rea-mcp-server-adapter.md` (MCP delivery, byte budgets), `survey-cli-ipc-protocol-wrapping.md` and `survey-plugin-extension-points.md` (generic techniques).

Evidence legend: **OBSERVED** = read in REA source/docs (cited). **SECONDARY** = third-party/search summary. **INFERRED** = my interpretation. **UNKNOWN** = not established.

Scope note: everything here concerns driving engines the user installed and is licensed for, on artifacts the user may analyse. The Linux Hopper demo-dialog clicker (`scripts/hopper-demo-x11.py`) automates a vendor registration dialog. That is close to the licensing boundary the owner excludes, so agentic-anything should **not** copy it (see "What to distill").

---

## 1. Summary

- REA connects to external engines in **six shapes**, all behind one `AnalysisClient` seam (`src/application/AnalysisProvider.ts:190-212`) (OBSERVED; the salvaged summary listed four):
  1. **In-host script + authenticated local socket.** Hopper Python, Ghidra `GhidraScript` (Java) and the Cutter plugin (Python/Qt).
  2. **Owned REPL over stdio.** GDB/MI (`--interpreter=mi3|mi2`) and Rizin (`-0` NUL-framed).
  3. **Owned stdio MCP server wrapping a pinned third-party engine.** Jadx: REA's `ReaJadxBridge.java` re-registers a filtered tool catalog of `jadx-headless-mcp` 0.7.1.
  4. **Adapter over a third-party MCP server that REA does not launch.** IDA, JEB.
  5. **One-shot file-RPC helper.** A request JSON goes in, an `O_EXCL` reply file comes out, and a reserved exit code classifies the failure. Used by the mitmproxy addon, the pwndbg/pwntools launchers and the LLDB tracer.
  6. **One-shot compiled native helper.** The request is JSON in argv and the result is JSON on stdout. `ReaNativeUI.swift` does macOS Accessibility (AX) tree reads, AX actions and window screenshots. `ProcessRunTokenReader.swift` reads process run tokens.
- **REA already drives GUIs through accessibility.** `bridge/native/ReaNativeUI.swift:82-115` performs `AXPress`, `AXIncrement`/`AXDecrement` and `AXValue` set on a selected element path. Before acting it checks PID, executable realpath, executable SHA-256, launch time, the CG window ID and that the AX window geometry is unique. This answers the salvaged open question "does REA contain AX automation": yes, on macOS (OBSERVED). It is the closest REA precedent for agentic-anything's drive layer.
- **All three socket bridges use the same bootstrap.** REA creates a private runtime directory, a 32-byte random hex token and a UUID `run_id`. It writes them to a 0600 descriptor file, never to argv or env. The script binds a 0600 Unix socket (or `127.0.0.1:0` with the port published by atomic rename). REA polls under one absolute deadline. A `health`/`ping` reply must echo the `run_id` and the expected limits before any work (OBSERVED: `src/hopper/BridgeLauncher.ts:669-690`, `src/hopper/HopperClient.ts:470-570`, `bridge/ghidra/ReaGhidraBridge.java:191-321`, `SECURITY.md:27-36`).
- **Ghidra's handshake is the strictest.** `ping` must match all of these: `run_id`, `read_only`, the **exact** capability set, Ghidra version, profile digest, target SHA-256, and the language and compiler spec when expected. One mismatch fails startup (`src/ghidra/GhidraSessionValues.ts:170-200`). The bridge also checks the imported program's SHA-256 in constant time. On a mismatch it serves only refusals (`ReaGhidraBridge.java:225-237,404-410`).
- **Cutter is the only mutually authenticated bridge.** It uses HMAC-SHA256 challenge/response (`rea-auth-v2`): a client nonce, then a server nonce with a server proof, then a client proof over the request, then signed responses (`rea-cutter-response-v2`). Each connection carries one request (`bridge/cutter/rea_bridge.py:737-808,952`). Cutter is started by the user, so REA attaches to it, and the bridge must also prove *its own* identity.
- **Process ownership is proven, never assumed.** `REA_PROCESS_RUN_ID` is merged into the child env as the final layer, and the child is spawned detached into its own process group (`src/process/ProviderProcess.ts:277-283,311-319`). Cleanup token-validates **every live member** of each rooted group before it sends any `SIGKILL`. A group may be revalidated at most twice (`src/process/ProcessOwnership.ts:356-512`). On Linux the start-time identity (`/proc/<pid>/stat`) is read before and after the token read, so a reused PID is not trusted (`src/process/ProcessOwnershipObservation.ts:1245-1285`, `src/process/ProcessOwnership.ts:1721-1756`). On macOS a compiled Swift helper reads `KERN_PROCARGS2` and `p_starttime` (`bridge/process/ProcessRunTokenReader.swift:89-93,281,349`).
- **Stop is fail-closed and retryable.** `ProviderProcessSupervisor.stop()` first runs verified group cleanup. Without it, it falls back to TERM (250 ms) → KILL (1000 ms). Any doubt yields `status:"incomplete"`, which clears the shared stop promise so a later call retries. Ownership stays held, and MCP surfaces it as `recovery.kind=retry_cleanup` on `close_binary` (`ProviderProcess.ts:26-29,636-768`; `docs/mcp-contracts.md:176,1098`).
- **Debuggers invert the kill rule.** `stopOwnedDebuggerProcess` signals **only the debugger PID, never its group or the inferior**. It requires a verified detach that names a durable output sink. It refuses on Windows because Job Object teardown would kill inferiors (`src/process/ownedDebuggerSession.ts:259-366`). `PersistentDebuggerHost.ts:1-4` restricts persistent GDB/Rizin sessions to Linux because only `/proc/<pid>/fd/{1,2}` readback can prove where inferior output goes.
- **Timeouts poison the stream.** When a GDB MI command times out, the session is marked `protocolError` and the debugger is killed, because untagged output can no longer be correlated (`src/gdb/GdbSessionManager.ts:2111-2194`). Rizin does the same when its random completion marker does not arrive (`src/rizin/RizinDebugSessionManager.ts:709-728,879,924`). Cutter marks itself `poisoned` after a 20 s UI-thread wait (`rea_bridge.py:794-807`). Cancelling an active Ghidra request closes the whole session (`src/ghidra/GhidraRequestQueue.ts:123-141`).
- **Bounds are checked before allocation on both sides.** `REA_BRIDGE_MAX_REQUEST_BYTES` ranges 7464..536870887 bytes (default 16 MiB). The bridge reads with `readline(limit+1)`. The client measures escaped JSON before queueing. A `health` reply whose limit disagrees with the client's fails startup (`bridge/hopper_bridge.py:18-31,1318-1331`; `HopperClient.ts:556-570,1095-1110`). Other caps: GDB MI line 16 MiB, Rizin frame 16 MiB with 4096 frames per command, Cutter 8 workers and 64 receipts, 64 KiB late output after stop.
- **This is not a sandbox, and REA says so.** The token protects the IPC channel, not the user account. Engines parse untrusted input with the user's authority, and the GDB/Rizin consoles are unrestricted (`SECURITY.md:33-36`; `docs/reverse-engineering-tools.md:5,49`).

---

## 2. `src/process/`: the supervision kernel

### 2.1 Spawn: `spawnOwnedProviderProcess` (`src/process/ProviderProcess.ts:254-341`) (OBSERVED)
- Order of operations:
  1. Check the abort signal.
  2. `prepareProcessOwnershipInspection(signal)` warms the native identity helper before launch. On macOS this compiles or loads the Swift token reader (`src/process/DarwinHelperCache.ts`).
  3. If requested, a baseline of live PIDs and identities is captured *before* launch for later token sweeps. A baseline that cannot be captured raises `ProcessAdmissionError("baseline_unavailable")` (`:264-270`).
  4. The env is merged, and `REA_PROCESS_RUN_ID` is applied **last** so that an override in another letter case cannot shadow it (`:277-283`).
  5. Spawn with `shell:false, windowsHide:true, detached: platform!=="win32"`, stdin `ignore` unless the protocol needs `pipe` (`:311-319`).
  6. Wait for the `spawn` event. A missing PID raises `launcher_pid_missing`.
- The returned `OwnedProcessGroup` holds `{runId, leaderPid, processGroupId=pid, expectedParentPid=process.pid, expectedCommand?, evidenceOwner}` (`:324-339`; type at `ProcessOwnership.ts:56-75`). `expectedCommand:null` turns off argv-prefix matching for interpreter launchers (`:36-37`).
- On Windows the same call goes to `WindowsOwnedProcess` (`:284-308`). Memory limits are honoured **only** there. On POSIX, `memoryLimitBytes` is refused with `memory_limit_unsupported` instead of being ignored (`:309-310`). This is fail-closed admission of a control.

### 2.2 Ownership proof and cleanup (`ProcessOwnership.ts`, `ProcessOwnershipObservation.ts`) (OBSERVED)
- **OS seam.** `ProcessOwnershipHost` has `prepare`, `close`, `listProcesses`, `environment(pid)`, `runTokens(processes)`, `processIdentities(processes)`, `captureBaseline` and `signalGroup(pgid, sig)` (`ProcessOwnership.ts:181-208`). The policy code is platform-neutral, and only this seam differs per OS.
- **Linux.**
  - `ps -axo pid=,ppid=,pgid=,uid=,stat=,command=` gives the table (`ProcessOwnershipObservation.ts:1219-1244`).
  - Identity is `linux-starttime:<field 22>` from `/proc/<pid>/stat`, and a parsed PID mismatch counts as malformed (`:1259-1285`).
  - The token comes from `/proc/<pid>/environ` (`:1489-1494`).
- **macOS.** The Swift helper returns identity `"<tv_sec>:<tv_usec>"` from `kp_proc.p_starttime`. It scans `KERN_PROCARGS2` for the `REA_PROCESS_RUN_ID=` prefix (`bridge/process/ProcessRunTokenReader.swift:89-93,281,349`). The compiled helper is cached under a key that hashes the Swift sources plus the `xcrun --find swiftc` and `--show-sdk-path` results, and it is re-checked by SHA-256 against a manifest before reuse (`src/process/DarwinHelperCache.ts:201-221,328`).
- **Algorithm** (`cleanupOwnedProcessGroup`, `ProcessOwnership.ts:372-512`):
  1. Read the live table. If the table cannot be read, return `cleaned:false`. Never signal blindly.
  2. Build a plan (`createOwnedCleanupPlan`). The plan contains the groups to signal, unresolved members and an "unverified" list. Unverified processes are unrelated processes whose token the OS withheld, for example macOS platform binaries or foreign-uid `EINVAL` (`:242-253`). They are recorded as evidence and not touched.
  3. For each group, revalidate it right before signalling. A group seen more than `OWNED_GROUP_REVALIDATION_LIMIT = 2` times fails with "no runtime override" (`:356-361,406-415`). Groups that members escaped into are queued too (`:425-437`).
  4. `SIGKILL` the group. `ESRCH` is fine, and any other error fails the cleanup (`:439-453`).
  5. When a token sweep is on, poll `verifyNoTokenOwnedProcesses` until a grace deadline expires, to confirm that no live process still carries the token (`:457-506`).
- **Failure reasons are typed per member**: `environment-unreadable | run-token-mismatch | process-identity-unavailable`, with an optional `identityAfter` that names the process but "is never ownership or signal authority" (`:224-240`).
- **Ledger.** `ProcessOwnershipEvidenceOwner` keeps candidates across retries. A *new* ownership object does not inherit signal authority (`:363-383`).
- **PID-reuse defence**: identities are read first, then the token, then identity again (`identityBefore`/`identityAfter`, `ProcessOwnership.ts:1721-1756`). Candidates are filtered against the pre-launch baseline: a PID that is in the baseline with an unchanged identity is not ours (`:1725-1736`).
- **Windows**: a suspended child is assigned atomically via `PROC_THREAD_ATTRIBUTE_JOB_LIST` to an unnamed Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Breakaway is disabled, and membership is checked before resume (`native/windows/src/process.cc:27,115,162`; `native/windows/README.md` "Boundaries"). The addon loader verifies package version, ABI, Node-API version, PE machine and SHA-256, and offers no env override (`native/windows/README.md:6-9`). The README does not mention Authenticode signing (UNKNOWN; same as the salvaged question).

### 2.3 Supervisor and stop state machine (`ProviderProcessSupervisor`, `ProviderProcess.ts:350-800`) (OBSERVED)
- **Output.** The supervisor captures stdout and stderr with byte-counted history and opaque checkpoints (`:144-171`). It never parses protocol: "wire protocol remains adapter-owned" (`:343-349`). When stdout carries the protocol, `captureStdout:false` (`:205`). Retained bytes obey a `ProcessDiagnosticBudget`; overflow is still drained and reported, just not retained (`:198-215`).
- **Output markers.** An `outputToken` is located before retention applies, so its latest occurrence survives an exhausted budget (`:209-213`). Ghidra uses this for `java.lang.OutOfMemoryError` (`src/ghidra/GhidraDiagnostics.ts:14`).
- **Sharing and retry.** `stop()` shares one in-flight attempt. An `incomplete` result clears `#stopPromise`, so the next call retries (`:636-646`). During stop, retention is capped at `MAX_LATE_OUTPUT_BYTES = 64 KiB` (`:29,689`). After a verified exit, pipes may drain for 1 s. A lost tail sets `outputIncomplete` but does not revoke the verified cleanup (`:694-702`).
- **`#stopWithin` outcomes** (`:710-768`):
  - `not-owned` when `ownsProcessLifetime:false`.
  - With a `cleanup` capability: verified cleanup, then wait for exit, then one retried cleanup. If cleanup failed but this supervisor itself observed the exit, that counts as positive proof and verification runs again (`:719-729`).
  - Without one: `already-exited`, or `SIGTERM` (250 ms) → `terminated`, or `SIGKILL` (1000 ms) → `killed`, or `incomplete`.
  - Any thrown error → `incomplete`.
- `snapshot().stopRequested` is set **before** the first signal, so an exit signal without that flag provably did not come from the owner (`:135-139`).
- After incomplete cleanup, `unref()` lets the host exit, and a retry calls `ref()` again (`:665-681`).

### 2.4 Deadlines (`src/process/ProviderDeadline.ts:31-158`) (OBSERVED)
- `ProviderStartupDeadline(timeoutMs, callerSignal)` holds one absolute monotonic `performance.now()` target.
- Its composite `signal` aborts on caller cancel or deadline expiry. `interruption` is `"cancelled" | "timeout"`, and a caller abort that lands after the deadline counts as `timeout` (`:47-58`).
- `remainingMs()` is passed to each phase: spawn, endpoint poll, connect, handshake and first request. Once the deadline expires the budget stays at 0 (`:81-87`).
- `wait(ms)` polls up to the earlier of the interval and the deadline, and re-arms when a timer fires early (`:95-119,144-157`).
- Defaults: Ghidra 330 s for import, analysis, bridge and handshake together; Hopper 120 s (`src/domain/analysisResourceLimits.ts:29,32`). Other values: GDB startup 10 s (`GdbSessionManager.ts:50`), Rizin startup 30 s (`RizinDebugSessionManager.ts:49`), native UI 180 s (`src/domain/native/nativeDeadlines.ts:18`).
- Every fixed cap says "This release has no runtime override" in its error. Configurable caps name the setting, its value, its source and its range (`ownedDebuggerSession.ts:37-67`).

### 2.5 Bounded output primitives (OBSERVED)
- `BoundedOutputExcerpt` keeps a head and tail byte excerpt (half and half) and marks the gap `… [output omitted] …` (`src/process/BoundedOutputExcerpt.ts:1-46`).
- `readBoundedControlLines(chunks, maxBytes)` handles LF and CRLF frames. An oversized line is reported **once** and drained before the next frame. Decoding expansion is bounded by slicing input at `maxBytes+1` (`src/process/BoundedControlLines.ts:9-87`).
- `redactCapturedTransportCredential` redacts the token even when a budget cut leaves only a **prefix** of it at the end of retained text (`src/process/ProviderDiagnosticRedaction.ts:4-31`). Ghidra applies it with every token issued in the session (`src/ghidra/GhidraClient.ts:1285-1296`).
- `DecodedNdjsonLineBuffer`, used by Hopper responses, splits lines **without** a size cap (`src/process/DecodedNdjsonLineBuffer.ts:1-31`). Response size therefore relies on the authenticated bridge self-limiting (INFERRED; the bridge-side `_json_safe` and the payload budget apply).

### 2.6 Private runtime roots (`src/process/PrivateRuntimeRoot.ts:30-97`) (OBSERVED)
- On POSIX, `mkdtemp` under a realpath'd parent; mkdtemp creates the directory owner-only atomically, so no chmod is needed afterwards (`:70-80`). On Windows, a native `WindowsPrivateRuntime` with a protected DACL that admits only the current user and SYSTEM. If that capability is unavailable, creation throws `PrivateRuntimeRootUnavailableError` with a `retryable` flag (`:58-62`).
- `close()` is idempotent and shared. A failed removal is not cached, so a later close retries (`:83-97`).
- Hopper uses `/tmp` on macOS to keep the socket path short (`HopperClient.ts:98`). Ghidra moves the socket into a separate short `/tmp/rea-ghidra-socket-*` root when `runtimeRoot/bridge.sock` exceeds `sockaddr_un` (103 bytes on macOS, 107 on Linux) (`src/ghidra/GhidraTransport.ts:42-60`).

### 2.7 Leases, shared startup, one-shot commands (OBSERVED)
- **`acquireOwnedLease`** (`src/process/OwnedLease.ts:80-431`). This is a liveness lock implemented as a *listening socket*: a Unix socket, or `\\.\pipe\rea-lease-<key>` on Windows. The owner replies `{runId, processId}`, and `ECONNREFUSED` means the lease is stale. Recovery runs under a `.recovery` mutex. The lease directory is opened `O_DIRECTORY|O_NOFOLLOW` and chmod'd 0700 via the fd. Hopper uses it to prevent two REA processes from opening the same canonical target, kind and loader args (`src/process/HopperLease.ts:17-50`). The lock dies with its owner and has no PID file to go stale.
- **`SharedStartupAttempt`** (`src/process/SharedStartupAttempt.ts:18-78`). One producer and many waiters. A waiter that cancels just leaves; only the **last** departing waiter aborts the producer and then awaits its cleanup (`status:"stopped"`). `close()` aborts with `STARTUP_OWNER_CLOSED`, which is distinct from a caller cancel.
- **`runOwnedCommand`** (`src/process/OwnedCommand.ts:163-210`). One-shot children run under a `ProviderStartupDeadline`. `OwnedCommandFailure.reason` is one of `cancelled | timeout | output-limit | process` and carries the snapshot, the cleanup result and a `cleanupOwner` for later retry (`:33-50`).
- **Linux resource ceilings**: `prlimit --as --fsize --cpu --core=0 --` pins soft and hard together. `clampLinuxResourceLimits` never raises inherited limits. `BOUNDED_PRODUCER_LIMITS` sets 32 MiB input, 4 MiB output, 30 s, 1 GiB AS, 64 MiB file, 30 CPU-s (`src/process/LinuxResourceLimits.ts:20-60`). Cgroup v2 `memory.events` `oom_kill` is read for OOM attribution (`src/process/LinuxCgroupMemory.ts:46-58,131`).
- **Typed admission codes**: 24 stable codes, e.g. `runtime_closing`, `baseline_unavailable`, `token_pid_incomplete`, `report_exceeds_budget` (`src/process/ProcessAdmissionError.ts:1-28`).

### 2.8 `PersistentDebuggerHost` and `ownedDebuggerSession` (OBSERVED)
- `persistentDebuggerHostSupported = platform === "linux"`. The rationale in the comment: "Both persistent debugger managers require Linux procfs descriptor readback" (`src/process/PersistentDebuggerHost.ts:1-4`). GDB and Rizin both check this before starting (`GdbSessionManager.ts:677`, `RizinDebugSessionManager.ts:393`).
- **Inferior identity** is `{pid, startTime}` from `/proc/<pid>/stat`. `ENOENT` means ended and is distinct from unreadable (`ownedDebuggerSession.ts:97-129`). `verifyOwnedInferiorsEnded` fails if any remembered identity is still alive with the same start time (`:132-140`).
- **`stopOwnedDebuggerProcess`** (`:259-366`):
  - The caller must close command admission first.
  - If the debugger already exited and the inferior's disposition was unconfirmed, the inferiors must be proven ended, or the result is incomplete.
  - On Windows it refuses outright (`:302-306`).
  - Otherwise detach must succeed within 1 s, and the result must be `no-inferior` or `detached` with `outputSink:"durable"` (`:311-343`).
  - Then `SIGTERM` the debugger only (300 ms), then `SIGKILL` (1 s).
- `closeOwnedDebuggerSessions` stops all sessions with `allSettled`. Failed sessions are retained in the map for retry and `unref`'d, and the call throws `OwnedDebuggerCleanupError` with their PIDs (`:377-422`).

---

## 3. Bridges in detail

### 3.1 Hopper (`bridge/hopper_bridge.py`, `src/hopper/*`) (OBSERVED unless noted)
- **Launch.**
  - Hopper's documented CLI is called with `[...loaderArgs, "--analysis", "-Y", bootstrap.py, --executable|--database, target]` (`src/hopper/BridgeLauncher.ts:400-407`).
  - On macOS, argv containing `"`, `\`, CR or LF is **rejected, not rewritten**, because the vendor helper embeds argv in AppleScript literals (`:412-437`).
  - On macOS REA owns only the short-lived launcher helper's run-token group. It does **not** claim the GUI process that LaunchServices creates or reuses, and the window may take focus (`:180-193`).
  - The bridge docstring names Hopper's `--python` option (`hopper_bridge.py:3-4`) while the launcher passes `-Y`. UNKNOWN whether these are aliases.
- **Bootstrap.**
  - `bootstrap.py` is written with mode 0600 and chmod'd again (`BridgeLauncher.ts:587-604`).
  - It contains `REA_SOCKET`, `REA_TOKEN`, `REA_RUN_ID`, `REA_TARGET_PATH`, `REA_OWNS_PROCESS_LIFETIME`, `REA_RECONNECT_TIMEOUT_SECONDS=30` and the optional limit, then `exec(compile(open(<packaged bridge>)...))` (`:669-690`).
  - On every non-success path the file is deleted (`:530-537`).
  - The token is `randomBytes(32).hex` and `runId` is a UUID (`HopperClient.ts:482-488`). `ownership.json` (run_id, pid, pgid, parent, launcher) is written atomically with mode 0600 (`:509-525`).
- **Server.**
  - One `AF_UNIX` socket, chmod 0600, `listen(1)`. Both the first accept and reconnects are bounded by `settimeout(30 s)`, "so an orphan cannot pin Hopper's dedicated Python thread". The socket is unlinked in `finally` (`hopper_bridge.py:1443-1471`).
  - All dispatch stays on Hopper's Python thread: "moving dispatch to a worker can deadlock Hopper" (`:1-6,1474-1477`).
- **Framing.** NDJSON. Requests are read with `readline(MAX+1)`; a line without LF fails as oversize or incomplete and closes the connection (`:1318-1332`).
- **Request validation.**
  - The key set must equal exactly `{id, token, method, params}`.
  - `id` is a non-bool int ≥ 0, `method` a non-empty str, `params` a dict.
  - The token must be ASCII and match under `hmac.compare_digest` (`:1342-1354`).
  - After authentication the idle timeout is lifted: "a connected owner's idle time remain[s] unbounded" (`:1356-1359`).
- **Responses.**
  - The bridge emits `{"id", "event":{type:"progress",...}}` before and after each request, and then `{"id","result"}` or `{"id","error":{code:-32000,message,type}}` (`:1360-1400`).
  - Error `type` is a closed enum: `capability_unavailable | authorization | invalid_request | bridge_exception`.
  - Authorization text is always the fixed string "Invalid bridge capability", and generic exceptions are reduced to `"<Type>: Hopper bridge operation failed"` (`:1419-1440`).
  - The client validates with strict zod schemas and refuses ids from the future or ids still queued (`src/hopper/protocol.ts:11-79`; `src/hopper/HopperResponseStream.ts:29-44`).
- **Health.** `health` returns `{name, version, run_id, process_id, max_request_bytes}` (`hopper_bridge.py:989-998`). The client requires the `run_id` to match and, for an external application, the **same `process_id`** within a run. A `max_request_bytes` mismatch fails startup (`HopperClient.ts:539-600`).
- **Target binding.** The bridge binds to exactly one Hopper document whose executable or database realpath equals `REA_TARGET_PATH`. If several match it reports "ambiguous". Hopper's Python wrappers are recreated on each call, so the bound wrapper is retained to keep cache identity stable (`hopper_bridge.py:178-218`).
- **Teardown.**
  - `shutdown` / `shutdown_document` stops the serve loop only when it can report `document_closed && analysis_stopped`, or `document_retained` when REA does not own the application lifetime (`:1000-1012,1371-1376`).
  - After a lost transport, `#reconnectForCleanup` polls the socket every 50 ms for up to 30 s, tolerating only `ENOENT`/`ECONNREFUSED`, and re-authenticates with `health` against the same `run_id` and pid before it sends shutdown (`HopperClient.ts:740-800`).
  - The bridge's 30 s accept window matches that budget. This resolves the salvaged open question on reconnection (OBSERVED; behaviour across a full REA *restart* is still UNKNOWN, because the new process lacks the old token).
- **Startup polling** checks a launcher exit before each attempt. An exit the owner already observed wins over a later cancellation when the failure is classified (`HopperClient.ts:860-905,918-932`).

### 3.2 Ghidra (`bridge/ghidra/ReaGhidraBridge.java`, `src/ghidra/*`) (OBSERVED)
- **Launch.**
  - `analyzeHeadless <project> -import <snapshot> ... -readOnly -deleteProject -log -scriptlog -scriptPath <bridgeDir> [-preScript ...] -postScript <absolute bridge path> <session.json>`.
  - For a pre-imported project it uses `-process <name> -noanalysis -readOnly` (`src/ghidra/GhidraLauncher.ts:983-988`).
  - The bridge script is selected **by explicit path**: "Ghidra checks the caller's cwd before scriptPath for a basename" (`:1061-1065`).
- **Private environment.**
  - `HOME`, `TMPDIR` and `XDG_{CACHE,CONFIG,DATA}_HOME` point inside the runtime root; on Windows also `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, `TEMP`, `TMP` and `ComSpec` (`:1124-1155`).
  - `-Duser.home` and `-Djava.io.tmpdir` are set as argv on POSIX or via `JDK_JAVA_OPTIONS` on Windows (`:1159-1176`).
  - Inherited `_JAVA_OPTIONS`, `JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS` and `GHIDRA_JAVA_OPTIONS` are cleared (`src/ghidra/GhidraInstallation.ts:845-848`).
  - Directories are created 0700 (`GhidraLauncher.ts:1081-1121`).
- **Descriptor.** `session.json` contains `{transport, endpoint_path, token, run_id, target_sha256, provider_version, profile_digest, [existing_project], [decompile_timeout_seconds], [analysis_extensions]}` (`GhidraLauncher.ts:376-396`).
  - The bridge reads it only if it is a regular file read with `NOFOLLOW_LINKS`, and the key set must match exactly (`ReaGhidraBridge.java:4990-5021`).
  - The file is **deleted immediately after reading on POSIX**. Windows keeps a native immutable lease until close (`:200-207`).
- **Admission inside the host.**
  - `currentProgram` must exist.
  - The raw/DOS-COM preparation marker must be present.
  - The Ghidra version must equal `provider_version`.
  - The imported `getExecutableSHA256()` must match `target_sha256` under `MessageDigest.isEqual`.
  - On a mismatch the bridge still serves, but every request except `shutdown` gets `import_identity_mismatch|unavailable`, so the client can read the refusal (`:225-237,404-410`).
  - It ends `GhidraScript`'s implicit transaction before serving, so every edit owns a real outer transaction and can roll back (`:241-244`).
- **Transports** (`:261-321`).
  - `unix-socket`: `ServerSocketChannel(UNIX)`, permissions `rw-------`, accept one client, delete the socket in `finally`.
  - `authenticated-loopback-tcp`: bind `127.0.0.1:0` and write `{host,port}` to `.pending`, then `ATOMIC_MOVE` it to the endpoint path. On Windows it uses `CREATE_NEW` without rename. Each accepted peer must deliver an authenticated first frame of ≤64 KiB within 1 s; otherwise the bridge drops it and accepts another, so "a rejected unauthenticated peer cannot consume the bridge session" (`:327-369`).
  - The client mirrors this choice with `GhidraTransportKind` (`src/ghidra/GhidraTransport.ts:25`).
- **Requests.**
  - Exact key set `{id, token, method, params}` plus an optional `decompile_timeout_seconds`. `id` must be > 0, and the token is checked in constant time (`:5023-5045,5146-5151`).
  - Responses are `{id, ok:true, result}` or `{id, ok:false, error:{code,message}}` (`:5153-5175`), validated by `z.strictObject` (`src/ghidra/protocol.ts:7-23`).
  - On the Unix path, lines after the first are read with unbounded `BufferedReader.readLine()` (`:394`). The peer there is already filtered by socket mode 0600 (INFERRED: this is acceptable only because same-user processes are trusted).
- **Health.**
  - `ping` returns `sessionInfo`: run_id, profile_digest, provider version, `read_only`, `analysis_complete`/`analysis_timed_out`, the capability list (mutators filtered out when read-only), the target (name, language, compiler spec, image base, sha256) and PDB/seed/analyzer reports (`:509-560`).
  - The client checks every field against expectations (`GhidraSessionValues.ts:170-200`). `analysis_timed_out` is a startup failure (`src/ghidra/GhidraClientStartup.ts:67-76`).
- **Read-only enforcement.** For existing projects and non-Unix transports, the mutating methods `annotate_native_function`, `set_address_name` and `set_addresses_names` are refused on the bridge side (`:451,505-507`) and also dropped from the advertised capabilities.
- **Concurrency.** A strict FIFO admits one request at a time. Cancelling a *queued* request just removes it. Cancelling the *active* request fails every queued request and stops the provider session (`src/ghidra/GhidraRequestQueue.ts:32-141`), because a busy Ghidra script cannot be interrupted safely (INFERRED from the design).
- **Laziness.** The 330 s deadline covers import, analysis and handshake, which run on the first query (`DeferredAnalysisClient` + `SharedStartupAttempt`; §5).

### 3.3 Cutter (`bridge/cutter/rea_bridge.py`, `src/cutter/*`) (OBSERVED)
- **Attach model.** The user installs the plugin, which is marked experimental (`:1-6`). REA does not launch Cutter.
- **Descriptor.**
  - Published to `$REA_CUTTER_BRIDGE_DIR`, `%LOCALAPPDATA%\REA\CutterBridge` or `~/.cache/rea/cutter-bridge` (`:451-458`).
  - The directory is opened `O_DIRECTORY|O_NOFOLLOW`, its uid checked, and `fchmod 0700` applied and verified (`:404-419`).
  - The descriptor itself is written with `mkstemp` + `fchmod 0600`, uid and mode verified, `fsync`, then `os.replace` (`:640-682`). Windows gets a DACL-protected `CreateFileW` followed by `MoveFileExW`.
  - It contains `{session_id, pid, host:127.0.0.1, port, token, document_generation, current_file, cutter_version, identity_status:"partial", max_request_bytes}`.
- **Auth (`rea-auth-v2`)**, per connection:
  1. Client sends `hello{kind:"rea-auth-v2", nonce(64 hex)}` within 1 s.
  2. Server replies `{session_id, nonce: ordinal‖random, proof: HMAC(token, "rea-cutter-server-v2:"+sid+":"+cn+":"+sn)}`.
  3. Client sends `{request:<json string>, proof: HMAC(token, "rea-cutter-client-v2:...:" + request)}`.
  4. Server compares with `compare_digest` and requires `session_id` to match.
  5. Responses are `{"response":"<escaped json>","proof":HMAC("rea-cutter-response-v2:...")}`.

  (`:737-770,952`; client side `src/cutter/CutterBridgeClient.ts:149-160,1102-1114`; streaming decoder `src/cutter/cutterAuthenticatedResponse.ts:7`.)
- **UI-thread marshalling.**
  - Requests are emitted through a Qt `Signal` to a `QObject` dispatcher, and the connection thread waits on a `threading.Event` for at most `SOCKET_TIMEOUT_SECONDS = 20` (`:461-467,794-807`).
  - On timeout it sets `cancelled`, marks the server `poisoned`, and replies `ui-thread-request-timeout, execution_state:"unknown"` with "restart Cutter before issuing further commands". After that only `status` and `ack` are served (`:776-781`).
- **Exactly-once accounting.**
  - Command `request_id` must be a canonical UUID.
  - Each command carries `expected_generation`, which is checked against a `document_generation` counter that increments whenever `file.path` changes (`:541-550,606-610`).
  - Up to `MAX_COMPLETION_RECEIPTS = 64` receipts (`not_started|complete|failed`) let a later client find out whether a timed-out command ran. They are removed by `ack`. When full, new commands are refused (`:596-604,783-791`).
  - A connection-completion watermark reports the longest finished prefix of connection ordinals (`:810-830`).
  - An exception from `cutter.cmd` is reported as `execution_state:"unknown"`, "Do not retry automatically" (`:584-593`).
- **Caps.** 8 connection workers, after which further accepts are closed (`:65-67`). A 25 s request deadline (`:164`). Signed responses are written in 64 KiB chunks so the frame is not held in memory several times over (`:37-63`).

### 3.4 GDB/MI (`src/gdb/GdbSessionManager.ts`) (OBSERVED)
- **Spawn.** `gdb --nx --quiet --interpreter=mi3` with stdin `pipe` and a fresh `runId` (`:809-817`). It falls back to MI2 if MI3 initialisation fails and cleanup succeeds (`:769-795`). Executable errors (`ENOENT`, `EACCES`, `EPERM`, `ENOEXEC`, `ENOTDIR`, `EISDIR`) become `GdbExecutableRefusal` (`:824-833`).
- **Supervision.** `ProviderProcessSupervisor({ownsProcessLifetime:false}, {captureStdout:false, maxDiagnosticBytes:64 KiB})` (`:843-846`). The debugger is stopped by `stopOwnedDebuggerProcess`, not by group kill (§2.8).
- **Startup.**
  - Wait for the `(gdb)` prompt within 10 s.
  - Send `-gdb-set auto-load off`, which must return `^done`, otherwise the session is refused (`:968-980`).
  - If `inferior_tty` is given, send `-inferior-tty-set` and read it back with `-inferior-tty-show`; the reported value must match exactly (`:981-1018`).
  - The session starts with `noInferiorVerified:true`, since `--nx` and no exec commands mean no inferior can exist (`:873-875`).
- **Framing.** Each command is prefixed with an incrementing numeric token, and result records `^(\d+)\^(done|running|connected|error|exit)` are matched to it (`:1774-1784,2104`).
  - For `-interpreter-exec console` commands, output can lack a token, so when the prompt returns REA issues a fresh read-only `-gdb-show mi-async` as a **completion token**. A `^running` triggers the same thing (`:1754-1800`).
  - Caps: MI line 16 MiB; 16,384 records per command; 64 KiB recent history (`:86-89`).
- **Timeout.** Set `protocolError` ("untagged stream output can no longer be safely correlated"), build partial Evidence with limitations, and call `#stopDebuggerOnly`. If that stop is incomplete, the error carries `cleanup:{reason, resources:[session_id]}` (`:2111-2194`).
- **Close.** One inferior plus a verified TTY → `-target-detach` must return `^done`. Before detaching, `/proc/<pid>/fd/1,2` must resolve (dev/ino, char device, realpath) to the selected TTY, with the inferior start time unchanged across the readback (`:436-472,1962-2035`). Several live inferiors → refuse and tell the agent to use `gdb_console` explicitly (`:1495-1510`).

### 3.5 Rizin (`src/rizin/RizinDebugSessionManager.ts`) (OBSERVED)
- **Spawn.** `rizin -N -0 -d -R stdout=<tty> -R stderr=<tty> -e dbg.exitkills=false [-D backend] <resolved path>` (`:444-456`).
  - `-0` makes every reply NUL-terminated.
  - `-N` skips rc files.
  - `dbg.exitkills=false` means quitting rizin does not kill the inferior.
- **Framing.** NUL-delimited frames. Caps: 16 MiB per frame, 4096 frames per command, 4096 queued unsolicited frames / 4 MiB (`:50-56,1062-1085,1176-1182,1731,1798`).
- **Correlation.** Each command is sent as `<cmd>\n!echo __REA_COMMAND_COMPLETE_<uuid>__\n`, and the command ends when that marker frame arrives (`:709-747`). Cleanup synchronises with a separate fresh marker and discards earlier frames, which are "never reused as inferior readback" (`:1143-1150,1362-1421`). A timeout or a missing marker stops the session (`:879,924`).
- **Detach verification** uses the same `/proc/<pid>/fd` readback (`:1326-1341`) and an `olj` descriptor check (`:1604-1610`).

### 3.6 Jadx (`bridge/android/ReaJadxBridge.java`, `src/android/*`) (OBSERVED)
- An "REA-owned metadata boundary for the unmodified jadx-headless-mcp 0.7.1 JAR" (`:47`).
- **Stdio discipline.** The real stdout is captured as `protocolOut`, and `System.setOut(System.err)` keeps library prints out of the MCP stream (`:1002-1004`).
- **Catalog filtering.** REA removes every upstream tool except a consumed set, re-registers those with REA schemas and descriptions, and adds `rea_jvm_status`, `rea_native_declarations`, `get_app_info` and `get_android_manifest` (`:1019-1070`). It serves over `StdioServerTransport` (`:1078-1081`).
- **Pinning.** The host side spawns this via `spawnOwnedProviderProcess` with stdin `pipe` (`src/android/JadxMcpTransport.ts:41,54-92`). The JAR is pinned by SHA-256 (`src/android/JadxRelease.ts:6`), and evidence records `jar_sha256` and `bridge_sha256` (`src/android/JadxSession.ts:278-332`).
- **Auth.** None beyond pipe ownership. stdio is private to the parent (INFERRED: the right choice when the host can be spawned as a child).

### 3.7 Third-party MCP servers (IDA, JEB) (OBSERVED; details in salvaged JSON)
- IDA HTTP endpoints must be `http(s)` on `localhost`, `127.0.0.1` or `[::1]` without URL credentials (`src/ida/IdaConfiguration.ts:63-71`). Redirects are refused with `redirect:"error"` (`src/ida/IdaMcpConnection.ts:502`).
- `listTools` runs right after `connect` (`:521-527`) and is checked against a pinned profile, as the salvaged summary describes.
- JEB re-reads its live catalog with `listTools(signal)` under an absolute catalog deadline (`src/jeb/JebMcpConnection.ts:42-43,86`).
- Response bytes are UTF-8-validated **before** the SDK can replace bad bytes, and failure closes the transport, because SDK 2.3.1 does not settle pending `tools/call` on an SSE read error (`src/providers/Utf8McpResponse.ts:10-51`).

### 3.8 One-shot helpers: native, mitmproxy, pwndbg/pwntools (OBSERVED)
- **`ReaNativeUI.swift`** (macOS). REA compiles it per observation with `/usr/bin/xcrun swiftc` into a fresh `mkdtemp` root, with the module cache and `TMPDIR` kept inside that root, and removes the root on close (`src/native/NativeUiHelperRuntime.ts:25-124`).
  - Input is a JSON argv: `{pid, window_id, executable, sha256, launch_time?, screenshot, accessibility, max_nodes, action?}` (`ReaNativeUI.swift:7-17`).
  - **Target admission** (`:46-79`): `NSRunningApplication(pid)` → executable realpath equals the requested path → launch time matches → executable ≤256 MiB and SHA-256 equal → exactly one CG window with that ID, owned by that PID, at layer 0.
  - **Permissions** are named precisely: "Grant Accessibility / Screen Recording … then retry; no broad capture fallback is used" (`:79-84`).
  - **AX**: messaging timeout 2 s. The AX window is matched to the CG window by **unique geometry** (`:85-93`). An action follows a child-index path, and the element's owner PID must equal the target (`:94-108`). Actions are only `click`→`AXPress`, `scroll`→`AXIncrement|AXDecrement` and `key-entry`→set `AXValue` (`:109-115`).
  - **Tree capture**: DFS bounded by `max_nodes`, depth 32, with explicit `gaps` and `truncated` flags (`:116-142`). Screenshots go through ScreenCaptureKit for that window only, scaled to ≤2048 px, with SHA-256 (`:143-160`).
  - The output is always one JSON object on stdout with `ok` or `{ok:false, code, message}` (`:162-174`).
  - The observation's limitations state that a failed after-capture does not prove the action had no effect (`src/native/NativeUiObservation.ts:312-314`).
- **`rea_lldb_tracer.py`** runs inside `lldb --batch`. It "launches one owned process … records each entry from registers and memory reads only … No expression is evaluated" (`:1-10`). Caps: trace 8 MiB / 65,536 frames, target output 1 MiB via 0600 FIFOs (`:28-38,141-146`). It holds the target at entry until the host writes an `identity_ack_path` file, which gives the host time to read the start identity (`:128-138`).
- **`mitmproxy/capture.py`** is an offline tnetstring decoder. It has no proxy listener and makes no target requests (`:1-5`). It sets `RLIMIT_AS` 768 MiB, CPU 30 s, FSIZE 96 MiB and CORE 0. It writes a single reply, or a fixed `limit` reply when the result is too large, then shuts down (`:358-384`). `BoundedReader` refuses declared lengths before allocation (`:35-40`).
- **pwndbg/pwntools.** A wrapper runs only a fixed sibling file (`launch.py`/`layout.py`). Caller input stays in `request.json` (`bridge/pwndbg/launch.py:1-4`).
  - `MemoryError`/`ENOMEM` → exit 75 plus an `O_EXCL` marker `M`; `EFBIG` → exit 76 plus `F` (`:21-39`).
  - `decoder_runtime.lower_resource_limits` takes the min of 3 GiB AS, 30 CPU-s and 64 MiB FSIZE and the inherited soft/hard limits, and never raises them (`bridge/pwntools/decoder_runtime.py:10-23`). It keeps 1 MiB of emergency headroom for building a `MemoryError` reply and caps the reply at 64 MiB (`:33-58`).
  - The GDB core launcher `execve`s `gdb -nx -nh --batch -iex 'set auto-load off' -iex 'set debuginfod enabled off' ...`. Only fixed package code appears in the GDB command line, and selected paths stay in JSON (`bridge/pwndbg/launch_impl.py:18-28`). It refuses any pwndbg version other than the pinned one (`bridge/pwndbg/core_context.py:31-33`).

---

## 4. Side-by-side comparison

| Aspect | Hopper | Ghidra | Cutter | GDB/MI | Rizin | Jadx |
|---|---|---|---|---|---|---|
| Who starts host | REA (via vendor CLI; GUI may be LaunchServices-owned) | REA (`analyzeHeadless`) | User (plugin pre-installed) | REA | REA | REA |
| In-host code | `bootstrap.py` → `exec` packaged `hopper_bridge.py` | `-postScript` packaged `ReaGhidraBridge.java` | `rea_bridge.py` plugin | none (native MI) | none (native `-0`) | `ReaJadxBridge` main wrapping pinned JAR |
| Transport | AF_UNIX 0600, `listen(1)` | AF_UNIX 0600 or `127.0.0.1:0` (port file via atomic rename) | `127.0.0.1:0`, `listen(8)` | stdin/stdout pipe | stdin/stdout pipe | stdio MCP |
| Secret delivery | 0600 bootstrap.py, deleted on failure | 0600 `session.json`, deleted after read (POSIX) | 0600 descriptor in 0700 dir (user-wide) | n/a (pipe) | n/a (pipe) | n/a (pipe) |
| Auth | bearer token per frame, `compare_digest` | bearer token per frame, `MessageDigest.isEqual`; TCP first frame ≤64 KiB/1 s | HMAC challenge/response, signed responses, per connection | parent-owned pipe | parent-owned pipe | parent-owned pipe |
| Framing | NDJSON, `readline(limit+1)` | NDJSON (`readLine`) | LF JSON frames, auth envelope | MI records + numeric tokens | NUL frames + random echo marker | MCP JSON-RPC (stdio) |
| Request shape | exact `{id,token,method,params}` | exact `{id,token,method,params}` (+`decompile_timeout_seconds`) | `{kind, request_id(uuid), expected_generation, command, json}` | `<n>-cmd` / `-interpreter-exec console` | raw command | `tools/call` |
| Handshake | `health` → run_id, pid, max_request_bytes | `ping` → run_id, version, profile, sha256, capability set, read_only | `status` → session_id, generation, receipts | `(gdb)` prompt, `auto-load off` ^done, TTY readback | startup marker | MCP initialize + filtered `tools/list` |
| Target identity | realpath of exe/db → one document | imported SHA-256 = descriptor SHA-256 | `file.path` + `document_generation` | inferior `{pid,starttime}` | inferior + `olj` | APK SHA-256, JAR SHA-256 |
| Threading | Hopper Python thread only | single script thread; client FIFO | Qt UI thread via Signal; 8 socket workers | serial command queue | serial | SDK |
| Hang handling | 30 s idle bound only pre-auth | cancel active ⇒ close session | 20 s UI wait ⇒ `poisoned` | timeout ⇒ protocolError + kill debugger | missing marker ⇒ stop session | operation timeout (`JadxOperationTimeout.ts`) |
| Teardown | `shutdown(_document)` must report closed/retained; reconnect-for-cleanup 30 s | `shutdown` → `project_ephemeral`; group cleanup; root removal retained on failure | none (user app); receipts/ack | verified detach then debugger-only TERM/KILL | sync marker, detach, TERM/KILL | close MCP session → unload |
| Process kill scope | token-verified launcher group | token-verified group (Job Object on Win) | n/a | debugger PID only | debugger PID only | owned group |

INFERRED takeaways from the table:
- When REA can spawn the host, it prefers a **pipe** (no token needed).
- It uses a **socket + token** when the host must run its own launcher or event loop.
- It adds **mutual HMAC** when it attaches to a host someone else started, because then it cannot trust where the descriptor came from.

---

## 5. `src/providers/`: lifecycle glue (OBSERVED)
- **`DeferredAnalysisClient`** (`src/providers/DeferredAnalysisClient.ts:13-84`). A closeable client is returned *before* any provider work starts.
  - `execute` acquires through `AnalysisClientCreation`, and refuses with `client_closed` if the ready client changed underneath it (`:48-53`).
  - `executeIfReady` answers only when a client is already live. It never starts one (`:18-27`).
  - `reuseReadiness()` is `ready | blocked | unknown`.
- **`AnalysisClientCreation`** (`src/providers/AnalysisClientCreation.ts:20-128`). Wraps creation in a `SharedStartupAttempt`.
  - A stored failure with `cleanupIncomplete` blocks every new acquire, so you cannot open a new client on top of a leak (`:82-89`).
  - A released client lease forces re-creation (`:93-96`).
- **`retainOwnedCommandCleanup`** (`src/providers/OwnedCommandCleanup.ts:9-96`). Converts an `OwnedCommandFailure` or a Darwin inspection failure into an `AnalysisCleanupLease` held in the caller's `RetainedResourceScope`. Releasing the lease re-runs `supervisor.stop()`, and a still-`incomplete` result rebinds a fresh `ProviderCleanupError`. This is how `retry_cleanup` reaches MCP.
- **`AnalysisClientFailure`** copies the original facts (operation, request id, provider code, diagnostic type) into a per-operation error. The comment says: "semantic attribution never inspects a later signal" (`src/providers/AnalysisClientFailure.ts:26-60`).
- **`StructuredContentMcpClient`** subclasses the MCP SDK `Client` to keep the original `structuredContent` dictionary, including own `__proto__` keys, after the SDK's era-specific validation (`src/providers/StructuredContentMcpClient.ts:16-77`). **`Utf8McpResponse`** is described in §3.7.

---

## 6. Session lifecycle (generic, as REA implements it) (INFERRED synthesis of OBSERVED parts)

```
ADMIT ─▶ PREPARE ─▶ LAUNCH ─▶ DISCOVER ─▶ HANDSHAKE ─▶ READY ⇄ BUSY ─▶ CLOSING ─▶ CLOSED
  │         │          │          │            │          │       │         │
  └─refuse──┴──────────┴──fail────┴────fail────┴──poison──┴───────┘   incomplete ─▶ RETAINED ─retry─▶ CLOSING
```

- **ADMIT**: check the host binary, version and digest. Refuse controls the platform cannot enforce (`memory_limit_unsupported`, `persistentDebuggerHostSupported`, Windows native capability).
- **PREPARE**: lease (one owner per target), private root, token + run_id, descriptor 0600, ownership baseline, warm the identity helper.
- **LAUNCH**: `spawnOwnedProviderProcess` (env token last, own group/job). Write `ownership.json`.
- **DISCOVER**: poll the socket or port file every 50 ms under the shared deadline. A launcher exit seen during polling is a typed startup failure.
- **HANDSHAKE**: `health`/`ping` must echo run_id, identity, limits and the capability set. Any mismatch → stop.
- **READY/BUSY**: serial FIFO. Caller cancel of a queued request just removes it. Cancel or timeout of an active request on an uninterruptible host leads to **poison/close**.
- **CLOSING**: an orderly `shutdown` on the protocol → token-verified group cleanup or verified debugger detach → remove the private root.
- **RETAINED**: any doubt is `incomplete`. Ownership and the lease stay held, and the next close retries.

---

## 7. Failure modes and fail-closed behaviour (OBSERVED)

| Failure | Detection | Behaviour | Source |
|---|---|---|---|
| Wrong peer on socket | token/HMAC mismatch | fixed "Invalid bridge capability"/`authentication-failed`; TCP peer dropped, session not consumed | `hopper_bridge.py:1349-1354,1429-1435`; `ReaGhidraBridge.java:345-369`; `rea_bridge.py:744-770` |
| Wrong process answered | run_id/pid mismatch in health | startup failure / "bridge process changed within the same run" | `HopperClient.ts:583-600`; `GhidraSessionValues.ts:177-200` |
| Wrong target loaded | SHA-256 / realpath / generation | refusal-only serving; ambiguous document; `active-document-generation-changed` | `ReaGhidraBridge.java:229-237`; `hopper_bridge.py:211-216`; `rea_bridge.py:542-550` |
| Limit disagreement | `max_request_bytes` in health ≠ client | startup refused | `HopperClient.ts:556-570` |
| Oversize frame | `readline(limit+1)` / byte caps | error reply then connection close; GDB/Rizin session stop | `hopper_bridge.py:1318-1332`; `GdbSessionManager.ts:1683-1705`; `RizinDebugSessionManager.ts:1073-1085` |
| Host hangs (UI/analysis) | event wait / command timeout | Cutter poisons; GDB/Rizin kill debugger; Ghidra closes session | §3.3–3.5; `GhidraRequestQueue.ts:123-141` |
| Unknown execution state | exception after start | `execution_state:"unknown"`, "Do not retry automatically"; receipts | `rea_bridge.py:584-593,596-604` |
| Launcher exits early | supervisor exit observation during polling | typed startup failure that a later cancel cannot override | `HopperClient.ts:860-932` |
| PID reuse / foreign process | start-time identity before+after token | not signalled; recorded `unverified` | `ProcessOwnership.ts:242-253,1721-1756` |
| Group keeps respawning | >2 revalidation passes | `cleaned:false`, no runtime override | `ProcessOwnership.ts:356-415` |
| Process won't die | TERM/KILL grace exceeded | `incomplete`; retained; `retry_cleanup` | `ProviderProcess.ts:742-758`; `OwnedCommandCleanup.ts:72-93` |
| Inferior disposition unknown | detach unconfirmed / no durable sink | debugger kept alive, close refused | `ownedDebuggerSession.ts:270-343` |
| Credential leak into logs | token present in captured output | redacted, including a truncated prefix | `ProviderDiagnosticRedaction.ts:4-31` |
| Orphan bridge after REA death | accept timeout | Hopper bridge exits after 30 s; Windows job kill-on-close; private residue possible | `hopper_bridge.py:1451-1466`; `native/windows/README.md` |
| Resource exhaustion in helper | rlimits, reserved exit 75/76 + marker | classified `resource-limit`; unclassified otherwise | `bridge/pwndbg/launch.py:10-39`; `decoder_runtime.py:44-52` |

---

## 8. Generic "bridge kit" spec for agentic-anything (INFERRED, derived from the OBSERVED patterns)

A **bridge** is in-host code (a plugin, script, extension or helper) that exposes a host app's API to the control plane. A new bridge must implement the following. The REA precedent for each item is in brackets.

### 8.1 Choose the channel (decision rule)
1. The control plane spawns the host, and the host can speak on stdio → **stdio pipe**, no token. Redirect library stdout to stderr [Jadx, GDB, Rizin].
2. The control plane spawns the host, but the host owns its own event loop or launcher → **Unix socket 0600** (Windows: loopback TCP plus a private DACL dir) with a **bearer token** [Hopper, Ghidra].
3. The host is started by the user or another party (attach) → **loopback TCP with mutual HMAC** challenge/response and signed responses [Cutter].
4. The host already ships an MCP/HTTP server → **adapter**: literal loopback, no redirects, a pinned tool-catalog profile, UTF-8 checked before parsing [IDA, JEB].
5. Stateless single action → **one-shot helper**: request JSON in (argv or 0600 file), one JSON reply out (stdout or `O_EXCL` file), reserved exit codes [NativeUI, mitmproxy, pwntools].

### 8.2 Descriptor (secret handoff)
- Path: `<private_root>/session.json`, mode 0600, parent 0700, created via mkstemp + fsync + atomic rename. On Windows: a protected DACL limited to the user and SYSTEM.
- Schema (exact keys, unknown keys rejected):
  `{protocol:"aa-bridge/1", transport:"stdio|unix|tcp", endpoint_path, token(64 hex), run_id(uuid), target:{kind, path, sha256}, host:{name, version}, profile_digest, limits:{max_frame_bytes, request_timeout_ms}, mode:"read_only|mutating"}`.
- The bridge reads it with no-follow and deletes it right after reading. The secret never appears in argv, env, or logs (redact prefixes too).

### 8.3 Endpoint publication
- Unix: bind → chmod 0600 → accept. Keep the path ≤103 bytes, or relocate to a short private root.
- TCP: bind `127.0.0.1:0` → write `{host,port}` to `.pending` → atomic rename to `endpoint.json`. The control plane polls for the file and connect under the session deadline.

### 8.4 Handshake (must complete before any operation)
- Bearer mode: the first frame is `{id:1, token, method:"hello", params:{}}`. Unauthenticated peers get ≤64 KiB and ≤1 s, and are dropped without consuming the session.
- Mutual mode:
  1. `C→S {kind:"aa-auth-v1", nonce}`
  2. `S→C {session_id, nonce, proof=HMAC(token,"server:"+sid+cn+sn)}`
  3. `C→S {request, proof=HMAC(token,"client:"+…+request)}`
  4. Every response is `{response, proof=HMAC(token,"response:"+…)}`.
- The `hello`/`health` reply is the **identity contract**: `{run_id, process_id, host_version, target_identity (sha256 / document id / generation), capabilities[] (exact set), read_only, limits{max_frame_bytes}}`. The client compares **every** field with what it expected, and the set comparison is exact. Any mismatch → teardown.

### 8.5 Framing and validation
- NDJSON, one object per LF. Read with `readline(max+1)`; a missing LF means oversize, so send an error and close. Measure the escaped JSON on the sender *before* queueing.
- Request: exact `{id:int>0, token, method, params:object}`. Response: exact `{id, ok:true, result}` or `{id, ok:false, error:{code∈enum, message}}`. Optional events: `{id, event:{type:"progress"|"diagnostic", …}}`.
- Closed method table. Unknown method → `invalid_request`. Error codes are a fixed enum: `authorization | invalid_request | capability_unavailable | target_mismatch | timeout | host_exception | busy`. Auth failures always carry the same fixed text, and host exceptions are reduced to their type.
- Client: strict schema, ids neither from the future nor still queued, UTF-8 validated before parsing.

### 8.6 Concurrency and host-thread rules
- One serial FIFO per session. Calls into the host API are marshalled onto the host's required thread [Hopper Python thread, Qt signal].
- Every operation carries `request_id` (uuid) and, where state can move, `expected_generation`. The bridge keeps a bounded receipt ring `{request_id → not_started|complete|failed|unknown}`, read via `status` and cleared via `ack`. That lets a retry decide whether the earlier attempt already happened.

### 8.7 Health, deadlines, hang policy
- `health` must be cheap and must work while the session is poisoned. It reports `poisoned`, `busy{request_id, elapsed_ms}` and `queued`.
- One absolute startup deadline is shared by spawn, discovery, connect and handshake, and is classified as `cancelled` or `timeout`. Each request has its own deadline; a fixed cap must *say* in its error that it has no runtime override.
- On a timeout in a host that cannot be interrupted, **poison the session**: refuse everything except `status`/`ack`/`shutdown`, report `execution_state:"unknown"`, and require a restart. Never let late output be credited to a later request.

### 8.8 Teardown
1. Close admission.
2. Send protocol `shutdown`. The bridge confirms what it closed versus retained (`document_closed`, `document_retained`), and the owner never closes or saves state it did not create.
3. Owned process: kill only the token-verified group or job. Revalidate identity right before each signal; ESRCH is OK.
4. Attached host or debugger: never kill the host or the inferior. Detach, verify, and kill only your own helper.
5. Remove the private root. If removal fails, keep the owner for retry.
6. Result is `verified | already_exited | not_owned | incomplete(reason)`. `incomplete` is retained and surfaces to MCP as a retryable `retry_cleanup` recovery.
7. Orphan guard: the bridge's own accept/idle timeout (e.g. 30 s), so it exits if its owner disappears.

### 8.9 Conformance tests every bridge must pass (INFERRED)
Wrong token; wrong run_id; extra request key; oversize frame without LF; future response id; target swapped after handshake; host hang longer than the deadline (expect poison); owner killed mid-session (expect the bridge to exit within its idle bound); PID reuse simulated (cleanup must refuse); cleanup failure (expect `incomplete` then a successful retry).

---

## What to distill for agentic-anything

1. **Use REA's six-shape taxonomy as the adapter catalogue**: pipe-REPL, pipe-MCP, socket+bearer, socket+mutual-HMAC, external-MCP adapter, one-shot helper. Pick the shape with the §8.1 rule, and record it in each adapter manifest (`channel:`).
2. **Ship one `bridge-kit` library per host language** (Python, Java/Kotlin, JS, Swift, C#). Each contains:
   - a descriptor reader (no-follow, exact keys, delete after read);
   - a listener (0600 Unix or loopback+port file);
   - a frame reader with `limit+1`;
   - a constant-time token check (bearer) and an optional HMAC v1 module;
   - closed method dispatch with a fixed error enum;
   - `health`/`status`/`ack`/`shutdown` built in;
   - a progress-event helper;
   - an idle/accept orphan timeout.

   A plugin author then writes only a method table plus a thread marshaller. Hopper (1477 lines) and Cutter (1330 lines) show how much boilerplate this saves.
3. **Port the supervision kernel as small, separately tested units, not a monolith**:
   - `spawnOwned` (env token last, own group/Job Object, refuse unenforceable limits);
   - `verifyAndSignalGroup` (identity → token → identity; revalidation cap; unverified ledger);
   - a `Supervisor.stop()` state machine whose `incomplete` result releases the shared promise so it can be retried;
   - `Deadline` (absolute, `cancelled|timeout`, `remainingMs`);
   - `SharedStartup` (last waiter aborts);
   - `OwnedLease` (listening-socket liveness lock);
   - `PrivateRoot`;
   - bounded excerpt/line readers;
   - token redaction that handles truncated prefixes.

   Start with Linux (`/proc`) and Windows (Job Object). Add macOS after picking a token-read strategy (§Open questions).
4. **Make "poison on uncorrelatable state" a framework invariant.** Any adapter whose host can't cancel an in-flight call must switch to `poisoned` when a deadline expires and report `execution_state:"unknown"`. Expose `request_id` receipts (`status`/`ack`) so the agent can check before retrying a mutating action. Most GUI and plugin hosts will be in this category.
5. **Handshake = identity contract.** Every session's `health` must return run_id, host PID, host version, target identity (digest, document id, generation counter) and the exact capability set. The client compares all of them. For driving, also require a `generation` that changes when the active document or window changes, and send `expected_generation` on every mutating call [Cutter].
6. **Adopt `ReaNativeUI.swift` as the macOS drive-adapter seed instead of `hopper-demo-x11.py`.**
   - Before each act it verifies PID, executable realpath, launch time, SHA-256, window ID and unique AX geometry.
   - It offers a small closed action set (press, increment/decrement, set value).
   - Permission errors name the setting to grant.
   - Tree capture is bounded and reports gaps.
   - Generalise the same contract to UIA (Windows) and AT-SPI (Linux) as one-shot helpers or a persistent bridge.
   - **Exclude** licensing, registration and consent dialogs by policy, and do not port the Hopper demo clicker.
7. **Model "kill scope" per adapter explicitly**: `owned_group | owned_job | own_pid_only | none`. Debugger-style and attach-style adapters default to `own_pid_only`/`none` and must prove detach before they release. Put this in the capability descriptor next to REA's `CapabilityEffects` (`src/application/AnalysisProvider.ts:262-270`) and the drive-effect extensions proposed in the salvaged summary.
8. **Prefer a pipe over a socket whenever the control plane can be the parent.** Only add sockets for hosts with their own launcher or event loop, and only add HMAC for attach. Tokens never go in argv or env; REA puts only the *ownership* token in env, because that token is not a credential.
9. **Bounded everything, measured before allocation**: one `max_frame_bytes` negotiated in the descriptor and echoed in health (refuse on mismatch), late-output caps after stop, rlimits/Job memory on helpers, reserved exit codes plus `O_EXCL` markers for resource failures in one-shot helpers.
10. **Surface cleanup debt to the agent.** `incomplete` → retained resource → `recovery:{kind:"retry_cleanup", operation}` on the next close, and block new sessions on the same target while debt remains [`AnalysisClientCreation.ts:82-89`].

## Open questions

- **macOS ownership strategy.** REA compiles a Swift `KERN_PROCARGS2` reader at runtime, which needs `xcrun`/Xcode CLT (`DarwinHelperCache.ts`). The alternatives are a prebuilt signed helper, or ownership by `posix_spawn` + process-group/kqueue `NOTE_EXIT` tracking without reading tokens. Which one fits a distributable kit, and how does Apple's withheld-env behaviour for platform binaries affect GUI targets?
- Hopper reconnect after a full REA **restart**: the new process does not have the old token, and the bridge's 30 s window then expires. Should a kit persist a resumable session record (token in a 0600 file under a lease), or always treat the restart as teardown?
- Ghidra's Unix path reads post-handshake frames with unbounded `readLine()` (`ReaGhidraBridge.java:394`). Should the kit require bridge-side frame caps even on 0600 sockets (defence in depth), at some cost in complexity?
- Is the Windows native addon Authenticode-signed, or only hash/manifest-pinned (`native/windows/README.md` mentions only the latter)? Does a kit for Windows UIA need a signed helper for UIAccess?
- What is the right **generation** signal for arbitrary GUI hosts (window title, document path, AX tree hash, app-provided revision)? Cutter uses `file.path` only (`identity_status:"partial"`).
- Hopper bridge docstring `--python` versus launcher `-Y`: aliases or a version-dependent flag? (UNKNOWN; not verifiable without Hopper.)
- `ReaNativeUI` matches AX windows by **geometry**, which fails for overlapping or same-size windows. Does a kit need a stronger CG↔AX link (e.g. the private `_AXUIElementGetWindow`), and is that acceptable from a policy standpoint?
- How should mutual-HMAC descriptors for attach bridges be scoped when several control-plane instances or users share a machine? Cutter uses one user-wide directory (`~/.cache/rea/cutter-bridge`).
