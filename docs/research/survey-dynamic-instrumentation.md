# Survey: Dynamic instrumentation as an agent control interface (for software the owner controls)

Research date: 2026-11-05

Technique family: `dynamic-instrumentation`. This rebuilds the lost full report from its salvaged summary (`docs/research/salvaged/dynamic-instrumentation.json`), checks it against primary sources, and goes further. Comparison baseline: the local REA fork at `/Users/nhanclie/rea` (`rea-agents` 6.8.0, HEAD `3484432fb`). It was read only; nothing was built or run. REA paths below are relative to that checkout.

Evidence legend: **OBSERVED** = read in a primary source (vendor doc, spec, man page, or REA file at the cited line). **SECONDARY** = search-engine summary, third-party page, or the salvaged summary, not re-read at the primary source. **INFERRED** = my interpretation. **UNKNOWN** = not established.

Related reports (referenced here, not repeated):
- `rea-runtime-observation-surfaces.md` covers REA's CDP, Inspector, Electron-hook, Playwright, Frida, GDB/Rizin and process-capture providers, plus the `toolEffects.ts` effect vocabulary. This survey cites it rather than re-deriving it.
- `survey-cli-ipc-protocol-wrapping.md` covers CDP, DAP and GDB/MI as *protocols*. This survey treats them as *instrumentation channels*.
- `survey-safety-rails-and-authorization-models.md` covers the general authority model. Section 2 below adds only what is specific to instrumentation.
- `survey-plugin-extension-points.md` covers sanctioned scripting hosts. Those always rank above instrumentation.

---

## 1. Summary

- Dynamic instrumentation means observing or changing a program while it runs. It takes seven mechanism classes: (a) in-process code injection with hooking (Frida, Detours), (b) dynamic binary translation (DBT/DBI: Frida Stalker, DynamoRIO, Intel Pin), (c) loader preload (LD_PRELOAD, DYLD_INSERT_LIBRARIES, LD_AUDIT), (d) kernel-mediated tracing (eBPF uprobes/USDT, DTrace, ETW), (e) debuggers (ptrace or Mach task port via gdb/lldb, scripted through the GDB Python API, the LLDB SB API, or DAP), (f) runtime-native agent interfaces (JVMTI and java.lang.instrument, the .NET diagnostic port and profiling API, Harmony, Python `sys.monitoring` and `sys.remote_exec`), and (g) runtime debug protocols (V8 Inspector/CDP via Node `--inspect` and Electron).
- For an agent, every class has two halves. The **observe** half (events, arguments, return values, live state) is broadly available and relatively safe. The **drive** half (calling internal functions, rewriting arguments, patching methods, evaluating expressions) is the most powerful "de-facto API" available for software that has no API, and also the most dangerous. Kernel tracers (eBPF, DTrace, ETW) are effectively observe-only by design. Everything else can drive.
- The deciding constraint is **who launched the target**, not how capable the tool is. Spawning the target under the control plane, paused, then instrumenting before resume works with ordinary user privileges on stock Linux (Yama `ptrace_scope=1` allows descendants), on .NET (diagnostic-port suspend), on the JVM (`-javaagent` stays warning-free), on Node (`--inspect-brk`/`--inspect-wait`) and on Frida (spawn starts paused). Attaching to an already-running process is where the permission walls are: Yama, macOS `get-task-allow` and SIP, Windows PPL, JEP 451, and Electron fuses. **INFERRED**, from the sources in the platform sections.
- Platform protections are deliberate vendor boundaries, not routing problems. Hardened Runtime ignores DYLD variables unless the app opts in. SIP-protected processes refuse task ports even to debuggers. Windows PPL stops even admins from injecting threads or writing memory. Electron fuses can turn off `--inspect`. Python 3.14 and .NET both offer an opt-out switch. A control plane must detect these, refuse, and report. It must never relax them. **OBSERVED** (sources per section).
- REA's instrumentation surface is narrow and observation-biased:
  - **Frida**: 8 MCP tools plus a one-shot CLI. Caller-authored scripts are recorded by SHA-256 only, capture caps are fixed, and the target is never killed.
  - **GDB/MI and Rizin**: unrestricted console sessions, Linux-only, with proof that detach succeeded.
  - **V8 Inspector**: attach-only. It sends exactly `Runtime.enable` and `Debugger.enable`.
  - **macOS LLDB call tracer** (`observe_native_calls`). This one is not in the salvaged summary. It launches an owned process under LLDB, sets auto-continuing breakpoints, evaluates no expressions, and kills only the process it launched.
  - REA has **no** eBPF, DTrace, ETW, JVMTI, .NET profiler, Python-hook, DynamoRIO or Pin integration. It treats LD_PRELOAD and DYLD variables only as facts to analyze statically, not as channels it uses (section 7).
- Current versions: Frida 17.25.2 shipped 2026-11-02, and REA pins Frida Core 17.23.2 (`package.json:433`). DynamoRIO 11.91.20755 (2026-10-30). Intel Pin 4.4 (2026-09-10, SECONDARY). Python 3.14 shipped PEP 768 remote exec. .NET 11 adds `DOTNET_`-prefixed profiler variables and lossless EventPipe `Block` mode. Node 24.12/25 adds `--allow-inspector` under the Permission Model. **OBSERVED** except where marked.
- **Where it fits:** instrumentation is the second-to-last rung of the technique ladder. It sits below the plugin API, existing IPC, OS automation and accessibility, and above vision. It is enabled per target only after an explicit "I own/authorize this" declaration, and defaults to observe. Its best use is often *distillation*: watch the app once with instrumentation to discover its internal API, then put a typed adapter on a higher rung (a plugin, IPC or runtime-native agent) where possible (section 8).

## 2. Authorization boundary (read first)

- **Scope.** Instrumentation in agentic-anything is only for (1) software the user builds or has the source of, (2) software the user owns and has the right to automate under its licence and ToS, or (3) targets with explicit written authorization. It is **not** for defeating DRM or licence checks, extracting protected assets (keys, models, media, game assets), bypassing anti-cheat or anti-tamper, or evading security controls. This matches REA `DISCLAIMER.md:41-43`: "Public availability of software, possession of a file, or access to a debugging endpoint does not by itself establish permission for every operation". It also matches `DISCLAIMER.md:50-52`: "Authorization to inspect an artifact does not necessarily authorize execution, interception, device changes, or interaction with another party's system". **OBSERVED**
- **REA governance precedent.** `AGENTS.md:7` requires "the required maintainer scope decision before substantial implementation" for any capability "involving target execution, injection, mutation, or protection recovery". **OBSERVED** agentic-anything should copy this as a project rule: every new instrumentation adapter needs an ADR naming its authority tier and its refusal conditions.
- **Hard stops** (refuse, report, never route around). The plane must not suggest, automate or document workarounds for any of these:
  - macOS SIP and processes without `get-task-allow`.
  - Hardened Runtime apps that have not opted into DYLD variables or debugging.
  - Windows Protected Process / PPL.
  - Kernel-mode anti-cheat or anti-tamper drivers.
  - Electron fuses that disable inspect, run-as-node or `NODE_OPTIONS`.
  - Python's remote-debug opt-out, `DOTNET_EnableDiagnostics=0`, the JVM's `-XX:-EnableDynamicAgentLoading`, and Node's `--disable-sigusr1`.
  - Secure-execution (setuid) binaries.
  
  Each of these is a vendor's or owner's explicit statement that injection is not authorized. **INFERRED**, grounded in vendor text quoted in sections 4-6.
- **Relaxing system-wide protections is out of scope even on the user's own machine.** Examples: SIP components via Recovery, Yama `ptrace_scope`, or code-integrity policies. Relaxing them weakens the host for every process, not just the target. The plane may report that a protection is the blocking cause. It must not offer to change it. **INFERRED**
- **ToS and legal.** Licences and ToS often forbid reverse engineering or instrumentation even of software the user possesses. Online and multiplayer software adds third-party harm, since tampering with a client can affect other users or services. Laws differ by jurisdiction (interoperability exceptions, anti-circumvention rules). The plane records the user's authorization declaration per target and does not adjudicate it. **INFERRED**; the REA disclaimer takes the same stance.

## 3. Observe vs drive: a taxonomy for an agent

| Class