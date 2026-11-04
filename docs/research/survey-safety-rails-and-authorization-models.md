# Survey: Safety rails and authorization models for an agent that drives arbitrary local software

Technique family: `safety-rails-and-authorization-models`
Date of research: 2026-11-04 (web facts marked with their own dates where known)
Scope anchor: the "agentic-anything" control plane -- one MCP/API through which an AI agent observes AND drives software on the user's own machine (including GUI apps and binaries with no public API). Out of scope by design: defeating DRM/licensing, extracting protected assets, evading security controls.

Legend: **OBSERVED** = read directly in a primary source or in REA source code at the cited path. **INFER** = my inference. **UNKNOWN** = could not verify. Web-search budget for the session ran out partway through (the last five planned searches, on Agent SDK `canUseTool`, Microsoft MCP-on-Windows, Apple two-VM EULA limit, EU AI Act Art. 14, and Codex sandbox internals, were not executed); affected items are flagged UNKNOWN or sourced from earlier fetches.

---

## 1. Summary

- Every shipped computer-use agent (Anthropic computer-use tool, OpenAI Operator/CUA, Microsoft Copilot Actions / UFO2, Claude Code, Codex CLI) converges on the same defence-in-depth stack, in roughly this order of load-bearing-ness: (1) an OS-level isolation boundary (VM / container / separate OS account), (2) a policy gate in front of every action (allow/deny/ask rules, classifiers, confirmations for consequential actions), (3) model-side robustness plus a screenshot/content classifier for prompt injection, (4) scoped data access (no credentials, allowlisted domains/folders), (5) monitoring, rate limits and audit logs. No vendor claims any single layer is sufficient; all primary docs say residual prompt-injection risk remains.
- MCP (2025-06-18) gives a vocabulary but not enforcement: `readOnlyHint` / `destructiveHint` / `idempotentHint` / `openWorldHint` are explicitly **untrusted hints**; the spec also says there SHOULD always be a human able to deny tool calls, servers MUST rate limit and implement access control, clients SHOULD log for audit. Elicitation (`elicitation/create`, accept/decline/cancel) is the only in-protocol way for a server to ask the user a structured question; it MUST NOT be used for sensitive data.
- REA (the downloaded third-party repo) is the best local example of **authority declared as data**: every public tool has a `ToolEffects` record (mutatesTarget, writesFilesystem, launchesProcess, accessesNetwork, changesUiState, mayDiscardData, executesForeignCode, mutatesDevice, idempotent) from which the four MCP hints are derived mechanically (`src/contracts/toolEffects.ts:764-786`). REA's prose vocabulary (observe / capture / launch / attach / inject / mutate) appears in docs, not as a typed enum; the typed representation is the effects record.
- Capability-based thinking (Hardy 1988 confused deputy; Miller 2006 POLA; Deno `--allow-*` with per-path/per-host scoping and deny-overrides-allow; DeepMind CaMeL's capability-tagged data flows) is the strongest conceptual model for a control plane that hands an LLM the power to drive arbitrary apps: the agent should hold narrow, revocable, target-scoped authorities, not ambient user-level power.
- Legal scoping: the lawful basis for reverse-engineering-for-interoperability is 17 U.S.C. 1201(f) in the US and Directive 2009/24/EC Art. 6 in the EU (contract terms contrary to Art. 6 are void under Art. 8; UK CDPA s.296A does the same). Both are narrow: lawfully-obtained copy, only the parts necessary, only to achieve interoperability, no infringement. The Ninth Circuit (MDY v. Blizzard, 2010) holds 1201(a) circumvention needs no infringement nexus, and Bungie v. AimJunkies (2023-24) applied 1201(a)(1)/(a)(2)/(b) to anti-cheat circumvention. That is why DRM / license-check / anti-cheat / security-control circumvention must be a hard exclusion for this project, not a soft policy.

## 2. What this technique family gives an agent

- **A bounded blast radius.** OS isolation (VM/sandbox/separate account) caps what a confused or injected agent can touch, independent of model behaviour. Claude Code docs: the sandbox "holds regardless of what the model chose to run" and "for a single enforced boundary, run the Claude Code process itself in a container, VM, or the sandbox runtime" (OBSERVED, code.claude.com/docs/en/sandboxing).
- **A per-action decision point.** Policy rules + confirmations give the human (or a classifier) a veto before side effects. OpenAI Operator: confirmation recall 92% (Jan 2025), 94% and 100% on financial transactions for o3 Operator (May 2025) (OBSERVED, system cards).
- **Machine-readable authority.** Effect/annotation metadata lets a client auto-approve read-only calls and route destructive/open-world calls to a prompt, without string-matching tool names (REA does exactly this; see section 4).
- **Injection containment.** Screenshot classifiers (Anthropic, OpenAI) that pause or force a "did the user really ask for this?" check; instruction/data separation (spotlighting, dual-LLM, CaMeL) so untrusted on-screen text cannot become control flow.
- **Recoverability.** Snapshots (VM/filesystem) + append-only action logs make a bad run diagnosable and (for local state) reversible.
- **Legal defensibility.** Declared authority + audit logs + explicit exclusions (no DRM/anti-cheat/security-control circumvention) keep the project inside 1201(f)/Art. 6 and away from CFAA/ToS exposure.

## 3. Platforms and permission models

### 3.1 Vendor agent products (what they actually ship)

**Anthropic computer-use tool** (platform.claude.com/docs/.../computer-use-tool, fetched 2026-11-04; OBSERVED):
- Recommended precautions, verbatim: "1. Using a dedicated virtual machine or container with minimal privileges... 2. Avoiding giving the model access to sensitive data, such as account login information... 3. Limiting internet access to an allowlist of domains... 4. Asking a human to confirm decisions that might result in meaningful real-world consequences and any tasks requiring affirmative consent, such as accepting cookies, completing financial transactions, or agreeing to terms of service."
- Prompt-injection warning: "Claude follows commands found in content even when they conflict with your instructions... instructions on webpages or contained in images might override your instructions."
- Classifier layer: "classifiers will automatically scan what the tools return, such as screenshots, to flag potential prompt injections... they will automatically steer the model to check whether the instruction really came from you before acting on it." Opt-out requires contacting support. "The precautions listed earlier remain important even with these classifiers."
- "Inform end users of relevant risks and obtain their consent prior to enabling computer use in your own products."
- System-card numbers: Claude 4 card (May 2025) computer-use prompt-injection prevention 71% -> 89% with safeguards (Opus 4), 69% -> 86% (Sonnet 4), ~600 scenarios (secondary summary of card; the card PDF itself was not read). Claude for Chrome post (anthropic.com/research/prompt-injection-defenses, dated 2025-11-24, OBSERVED): adaptive best-of-N attacker, 100 attempts/environment: Opus 4.5 8.0% model-only -> 1.4% with improved safeguards; Anthropic states a 1% rate "still represents meaningful risk."

**OpenAI Operator / CUA** (Operator System Card, 2025-01-23, updated 2025-03-11; o3 Operator addendum, 2025; both PDFs read via pdftotext; OBSERVED):
- Risk policy "categorized tasks and actions by their risk severity, considering the potential for harm to the user or others, and the ease of reversing any negative outcomes."
- Confirmations: "ask the user for confirmations before finalizing actions that affect the state of the world (e.g., before completing a purchase or sending an email)"; recall 92% on 607 tasks / 20 categories (Jan 2025); 94%, 100% on financial transactions (o3, May 2025).
- Proactive refusals: "refuse certain high-risk tasks like banking transactions and making high-stakes decisions", recall 94%.
- Watch Mode: "On certain websites, the impact of mistakes may be higher. For example, on email services... we require that the user supervise Operator's actions by automatically pausing execution when the user becomes inactive or navigates away from the page."
- Blocked sites: "we restrict Operator from navigating to websites that could enable potentially harmful or illicit activities that are prohibited by OpenAI's Usage Policies." No published list (UNKNOWN which domains).
- Prompt-injection monitor: "will pause execution if a suspected prompt injection is detected on the screen"; 99% recall / 90% precision on 77 red-team attempts; 46 false positives out of 13,704 benign screens; "can be rapidly updated... increased our recall from 79% to 99% in just one day." Base model susceptibility 23% (Jan 2025) -> 20% (o3); monitor is "the primary layer of defense."
- Mitigation table also lists "Rate limits at various levels to deter or prevent scaled abuse" and "Human and automated monitoring and review for deceptive and fraudulent activities."
- API caveat (OBSERVED, card s.5): "Since the API can be used in non-browser environments, the impact of a successful prompt injection is heightened given the potential for use on Local OS." Directly relevant: this project IS the local-OS case.
- Takeover mode for credential entry: described in the launch blog (openai.com/index/introducing-operator, 403 on fetch) -- UNKNOWN exact wording; the card's safety text does not use the term.

**Microsoft Windows 11 Copilot Actions / agent workspace** (learn.microsoft.com Windows 11 security book, Agentic security chapter, ms.date 2025-11-13; Windows Experience Blog 2025-10-16; support.microsoft.com "Experimental agentic features"; all OBSERVED):
- Four principles: "Distinct agent accounts", "Limited agentic privileges" ("has no ability to make changes to your device without your intervention. You can revoke this access at any time"), "Operational trust" ("must be signed by a trusted source so that maliciously or poorly behaved agents can be revoked and blocked"), "Privacy-preserving design".
- Controls: off by default (Settings > System > AI components > Agent tools > Experimental agentic features; admin-only to enable; applies to all users once on); "Agent accounts: A separate standard account on your device that agents use when acting on your behalf"; "Agent workspace: A contained environment where agents can work in parallel with a human user... its own desktop while limiting the visibility and access the agent has to the user's desktop activity... built on recognized security boundaries that Microsoft defends in accordance with our longstanding security servicing criteria"; "User Transparency: A way for users to authorize, monitor, and take over agent actions."
- Data scope: six known folders (Documents, Downloads, Desktop, Music, Pictures, Videos) requestable; per-agent file permission "Allow Always / Ask every time / Never allow"; anything else needs explicit authorization; enforced by ordinary ACLs.
- Explicit XPIA acknowledgment: "malicious content embedded in UI elements or documents can override agent instructions, leading to unintended actions like data exfiltration or malware installation."
- Microsoft's MCP-on-Windows post (2025-05-19, OBSERVED): proxy-mediated MCP traffic "for centralized policy enforcement, consent handling, and auditing"; registry admits only servers meeting baseline criteria; "Users must explicitly approve each client-tool pair, with support for per-resource granularity"; mandatory code signing; "Servers' definition of tools cannot be changed at runtime"; runtime isolation; servers must declare required privileges.
- UFO2 (arXiv 2504.14603, Apr 2025; secondary summaries only): HostAgent + per-app AppAgents, hybrid UIA + vision control detection, Picture-in-Picture virtual desktop on Windows remote-desktop infrastructure so the agent does not fight the user for mouse/keyboard. INFER: this is a *non-interference* boundary, not a security boundary; the Windows agent-account/workspace feature is the security one.

**Claude Code** (code.claude.com docs, fetched 2026-11-04; OBSERVED):
- Permission modes: `default` (ask), `acceptEdits`, `plan`, `auto` (classifier reviews), `dontAsk` (deny anything that would prompt), `bypassPermissions` ("offers no protection against prompt injection or unintended actions"; "Only use this mode in isolated environments like containers, VMs"; refuses to run as root/sudo outside a recognized sandbox; admins can disable via `permissions.disableBypassPermissionsMode`).
- Rule evaluation: deny -> ask -> allow, first match wins; SDK order: PreToolUse hook -> ask -> deny -> mode -> allow -> `canUseTool` -> PostToolUse.
- Auto-mode classifier sees user messages, non-read-only tool calls and CLAUDE.md; "Tool results are stripped, so hostile content in a file or web page can't directly manipulate it." Never auto-approves: explicit ask rules, MCP tools marked `requiresUserInteraction`, recognized critical-path `rm`/`rmdir`. Circuit breaker: 3 consecutive or 20 total classifier blocks -> auto mode pauses and prompting resumes; server-side kill (Anthropic can switch auto mode off; session stays off).
- Sandbox: macOS Seatbelt, Linux/WSL2 bubblewrap + socat proxy + optional seccomp; writes limited to cwd + temp + added dirs; protected paths (`.claude/settings*`, `.mcp.json`, `.bashrc`, `.git/hooks`) write-denied; network via hostname-allowlist proxy, allowlist starts empty, per-domain prompt. Explicitly *outside* the sandbox: Read/Edit/Write/WebFetch tools, hooks, local MCP servers, LSP servers. Default fallback when sandbox unavailable is to run unsandboxed unless `failIfUnavailable: true`. Native Windows: no sandbox.

**OpenAI Codex CLI**: legacy presets suggest / auto-edit / full-auto; newer split into approval policy (untrusted / on-request / never) x sandbox mode (read-only / workspace-write / danger-full-access); `--dangerously-bypass-approvals-and-sandbox`. Mechanisms (Seatbelt on macOS, Landlock/seccomp on Linux) are from secondary sources; the official security page fetched only pointed at sub-pages. UNKNOWN exact current defaults.

### 3.2 OS isolation options

| Option | Boundary strength | GUI/input story | Snapshot/undo | Notes (OBSERVED unless marked) |
|---|---|---|---|---|
| Separate OS user account (macOS/Linux/Windows) | Weak-moderate: same kernel, ACL/TCC-based | Full native GUI; on macOS TCC grants (Accessibility, Screen Recording, Automation/`kTCCServiceAppleEvents` per target app) are per-user and some (Screen Recording) cannot be self-approved by a standard user post-Big Sur without an MDM/PPPC profile (secondary MDM-vendor docs) | None intrinsic (APFS/Btrfs snapshots are host-wide) | This is what Windows agent workspace productizes. Cheapest way to drive the user's real installed apps without re-installing them. INFER: good default for "software the user owns" when combined with policy gates. |
| Windows Sandbox | Hypervisor (separate kernel) | Full Windows desktop, vGPU, clipboard/folder mapping via .wsb config, networking on by default | Disposable: "closing it deletes all software, files, and state" (persists across in-sandbox reboots since 22H2) | Pro/Enterprise/Education only; one instance at a time; same build as host; host-installed apps NOT present -- must reinstall inside. |
| Full VM (QEMU/KVM, Hyper-V, VMware, Apple Virtualization.framework via Lume/Tart) | Hypervisor | Needs virtio-gpu/SPICE/VNC or vendor display; macOS guests only on Apple Silicon hosts | Native checkpoints (QEMU savevm, VMware snapshots, Hyper-V checkpoints) | Lume (trycua, MIT per listing) and Tart (cirruslabs, "fair-source") wrap Virtualization.framework for macOS/Linux guests. Apple EULA limit on macOS guest count: UNKNOWN (search budget). |
| Firecracker microVM | Hypervisor + jailer (chroot, namespaces, cgroups, per-thread seccomp) | **None**: device model is virtio block/net/vsock + serial + partial i8042; no GPU, no PCI passthrough; Linux guests | Full and diff snapshots; restore in ms; doc warns resuming the same snapshot twice is "considered insecure" (RNG/token reuse) and network connectivity not guaranteed after resume | Great for headless/CLI targets; not a GUI-app sandbox. |
| Linux container (bubblewrap/Flatpak, Docker) | Namespaces; shares kernel | Xephyr/Xpra nested X is "not a sandbox" (Arch forum; FSFE list); Wayland + xdg-desktop-portal RemoteDesktop is the consent-gated path for synthetic input (`CreateSession`->`SelectDevices`->`Start` shows a dialog; `persist_mode`/`restore_token` for re-use); `/dev/uinput` tools (ydotool) bypass that consent model and need a privileged daemon | Overlayfs / Btrfs subvolume snapshots | Anthropic's `@anthropic-ai/sandbox-runtime` and Codex use this class for shell, not GUI. |
| Qubes OS | Xen VM per domain; dom0 draws trust-coloured borders so "compromised VMs can't spoof them"; GUI agent/daemon protocol instead of direct X access; GUI domain separate from dom0 (experimental in 4.1) | Native per-VM windows | Per-VM | Gold standard reference architecture for GUI isolation; impractical as a deployment target for most users. |

### 3.3 Protocol-level model: MCP 2025-06-18 (OBSERVED, modelcontextprotocol.io)

- Tools page: "there SHOULD always be a human in the loop with the ability to deny tool invocations"; applications SHOULD "Provide UI that makes clear which tools are being exposed", "Insert clear visual indicators when tools are invoked", "Present confirmation prompts". "clients MUST consider tool annotations to be untrusted unless they come from trusted servers." Servers MUST "Validate all tool inputs, Implement proper access controls, Rate limit tool invocations, Sanitize tool outputs"; clients SHOULD "Prompt for user confirmation on sensitive operations", "Show tool inputs to the user before calling the server", "Implement timeouts", "Log tool usage for audit purposes".
- Annotation semantics (from spec diff + secondary): `readOnlyHint` default false; `destructiveHint` default true (meaningful only when not read-only); `idempotentHint` default false; `openWorldHint` default true.
- Elicitation: `elicitation/create` with flat primitive JSON schema; result `action: accept|decline|cancel`; "Servers MUST NOT use elicitation to request sensitive information"; clients SHOULD rate limit, show which server is asking, allow decline at any time. INFER: elicitation is the right transport for "confirm this destructive action?" when the host client lacks a native permission prompt, but it cannot carry passwords/tokens -- credential entry must stay a human "takeover" step outside the protocol.
- Sampling: "there SHOULD always be a human in the loop with the ability to deny sampling requests"; clients SHOULD let users view/edit prompts and review completions. INFER: a server-side classifier that wants a second model opinion on a screenshot could use sampling, but the spec's review expectations make it heavyweight.
- Claude Code additionally honours an MCP tool flag `requiresUserInteraction` that is never auto-approved (OBSERVED in Claude Code docs; not in the MCP spec text I read -- treat as client extension).

### 3.4 Capability / authority-declaration patterns

- **Confused deputy** (Hardy 1988, SIGOPS OSR 22(4)): a program with more authority than its invoker is tricked into using it. INFER: an LLM agent running as the user is the archetypal deputy; on-screen text is the manipulating invoker.
- **POLA / object capabilities** (Miller, *Robust Composition*, JHU 2006; *Capability Myths Demolished* 2003): authority should be designated and transferred together; ACL systems separate designation from authority, enabling confused deputies.
- **Deno** (docs.deno.com/runtime/reference/permissions): default-deny; `--allow-read=foo.txt`, `--allow-net=api.example.com:443`; matching `--deny-*` flags "take precedence over the allow flags"; runtime `Deno.permissions.revoke`; `Deno.errors.NotCapable`. INFER: closest ergonomic model for a control-plane CLI/config -- per-app, per-window-title, per-path, per-host grants with deny-overrides-allow.
- **CaMeL** (DeepMind/ETH, arXiv 2503.18813, 2025): "explicitly extracts the control and data flows from the (trusted) query... untrusted data retrieved by the LLM can never impact the program flow"; capabilities on values; policies enforced at tool-call time; 77% of AgentDojo tasks solved with provable security vs 84% undefended. Limits (secondary): policy-authoring burden; control-flow side channels.
- **Dual-LLM / "lethal trifecta"** (Willison; secondary summaries): privileged planner never reads untrusted content; quarantined reader has no tools. Trifecta = private data + untrusted content + external communication. One follow-up study reports dual-LLM drove attack success to 0% while utility fell 49.7% -> 14.6% (secondary).
- **Spotlighting** (Microsoft, arXiv 2403.14720, Mar 2024): datamarking / random interleaving / base64 of untrusted input; ASR >50% -> <2% in GPT-family tests; LLMail-Inject CTF showed it "did not hold up against determined adaptive adversaries" (secondary).
- **REA's effects record** (OBSERVED, `src/contracts/toolEffects.ts`): see section 4.

### 3.5 Prompt injection via on-screen content: the threat numbers

- Pop-up attacks (Zhang, Yu, Yang, 2024): 86% average ASR on OSWorld; system-prompt defences did little (secondary).
- VPI-Bench (arXiv 2506.02456, ICLR 2026; OBSERVED abstract): 306 cases, 5 platforms; CUAs deceived up to 51%, browser agents up to 100%; "System-prompt defenses... offer only limited improvements."
- EVA (arXiv 2505.14289): adaptive attacker tunes injected content using the agent's GUI attention; ~+32 pp over static baselines in pop-up scenario (secondary).
- Vendor base rates: Operator 23% -> 20% (model only), Opus 4 71%/89% prevention, Opus 4.5 browser 8.0% -> 1.4%. INFER: for a control plane that renders arbitrary local app windows (email clients, PDF viewers, terminals showing untrusted logs), assume double-digit base susceptibility and design so a successful injection cannot reach a consequential action without an out-of-band gate.

### 3.6 Audit, replay, reversibility, kill switches

- MCP: clients SHOULD log tool usage for audit (OBSERVED).
- Windows agent accounts: separate account "clearly separates the work done by the agent from other actions on the system" -- i.e. OS event logs attribute actions to the agent principal (OBSERVED).
- Firecracker snapshot/restore (OBSERVED): full/diff snapshots, ms-level restore; warns about duplicate randomness/tokens on multi-restore.
- Agent-checkpoint literature (secondary, 2026): "Checkpointing restores the agent's state, not the outside world. You can restore the state that existed before the call. You cannot undo the call"; ACRFence paper describes "semantic rollback" where a restored checkpoint re-executes an irreversible action (10/10 duplicate commits in experiments). Agent VCR (PyPI) records JSONL frames + git-backed savepoints and replays without model calls; Scarab runtime merges audit log + observation log + workspace snapshots into one timeline with `rollback`.
- Kill switches as shipped: Operator pause on injection detection and on user inactivity (Watch Mode); Claude Code auto-mode circuit breaker (3-in-a-row / 20-total blocks) and server-side disable; Windows "take over agent actions". Vendor-published runaway rate-limit designs: UNKNOWN beyond Operator's "rate limits at various levels".
- Incident evidence that confirmations alone are insufficient (secondary, forum/blog): Antigravity agent `rmdir /s /q d:\` while clearing a Vite cache (late 2025); Gemini CLI misread a failed `mkdir` as success and moved files into a nonexistent destination; Cursor setting flipped from "Use Allowlist" to "Run everything" after an update (team-confirmed bug). A forum point worth keeping: blocklisting `rm` does not help if the agent writes `rm -rf` into a script and runs the script -- textual command matching is not a boundary (Claude Code docs say the same about `excludedCommands`: "Matching is textual").

## 4. REA's approach (comparison input; OBSERVED in repo)

- `DISCLAIMER.md`: lists lawful uses ("interoperability, software preservation, and debugging or auditing software you own or are authorized to examine") and refuses "circumventing DRM, license checks, anti-cheat systems or other technical protection measures; developing game cheats...; bypassing authentication, bot protection or access controls...; evading the usage policies or safety measures of AI model providers." Key sentence for this project: "Authorization to inspect an artifact does not necessarily authorize execution, interception, device changes, or interaction with another party's system." and "Public availability of software, possession of a file, or access to a debugging endpoint does not by itself establish permission for every operation."
- `SECURITY.md`: "This is not a sandbox and does not protect against malicious processes already running as the same operating-system user." Per-session random capability tokens for Hopper/Ghidra bridges, current-user Unix sockets (POSIX) / loopback TCP with private NTFS DACL (Windows); tokens passed via private session descriptors, not argv/env. "REA never invokes `sudo`"; privileged installs go through `pkexec`.
- `src/contracts/toolEffects.ts:1-22`: `ToolEffects` = {mutatesTarget, mutatesSession, writesFilesystem, launchesProcess, accessesNetwork, changesUiState, mayDiscardData, executesForeignCode ("Runs or drives code REA does not own on the host"), mutatesDevice, idempotent}. `TOOL_EFFECTS` (line ~53 onward) is an explicit per-tool table ("Explicit effect audit for every public tool"), e.g. `gdb_console` = mutatesTarget+writesFilesystem+launchesProcess+accessesNetwork+executesForeignCode, non-idempotent.
- `src/contracts/toolEffects.ts:764-786` `annotationsFromEffects`: readOnlyHint = none of the effect flags set; destructiveHint = mayDiscardData || executesForeignCode || mutatesDevice ("Running foreign code or changing a device cannot be bounded to additive effects"); idempotentHint = idempotent; openWorldHint = launchesProcess || accessesNetwork. Comment: "Derive MCP annotations without tool-name or tool-family heuristics."
- `src/contracts/toolEffects.ts:788+`: a second classification of tools whose results "carry only REA-authored session state or caller-supplied values, never text originating from an analysis target" -- i.e. REA tags which tool outputs are *untrusted text* (target-originated). INFER: this is an instruction/data-separation primitive at the contract level; a control plane should carry the same bit on every observation (screenshot OCR, window title, clipboard, UIA text).
- `docs/tool-design.md:16-27`: tool shapes `inspect / search / list / trace / compare / workflow / observe / capture`; "add one [tool] for a distinct outcome or materially different authority"; line 172: "Declare process, filesystem, network, UI, and mutation effects truthfully; enforce the requested target and lifecycle." Line 129: "Keep recovery within the selected target, provider, project and authority... do not switch or select a candidate automatically."
- `docs/frida-instrumentation.md:7,29`: spawn starts paused so scripts load first; "REA never kills the target"; "Frida scripts execute inside the selected target with that process's memory, filesystem, and network authority"; script source is NOT stored in Evidence, only path + SHA-256; authentication values excluded from Evidence. INFER: a clean model for `inject`-class authority -- the control plane records *what* was injected (hash) and *where* (target identity) but never claims more than it observed ("final target liveness remains unknown").
- `src/process/WindowsAuthority.ts:8-33`: Windows isolation claims are "proven by the packaged native boundary, not POSIX flags" -- a `WindowsNativeCapabilitySet` {job_object_process_ownership, private_runtime_dacl, reparse_safe_path_admission} each with `proof: "native-authority" | "not-proven" | "not-applicable"`. INFER: pattern worth copying -- never *assert* a sandbox/ownership property; carry a proof status and refuse to claim isolation when not proven.
- `docs/design/limits-and-recovery.md`: settings have "a typed value, resource bounds, an enforcement owner, and a defined application point"; "Exposing a setting requires a working enforcement path, not just an entry in the tool schema."
- `src/server/mcpClientMetadata.ts:58-66`: REA inspects the client's `elicitation` capability (form vs URL mode) at initialize time. UNKNOWN whether REA uses elicitation for confirmations (grep found only capability detection).
- The literal vocabulary "observe/launch/attach/inject/mutate" as a *typed* authority enum was not found in `src/` (grep on quoted literals returned nothing); it exists as prose in `docs/tool-design.md` ("Authority, launch/attach effects, lifecycle, and cleanup status") and in the Frida/process docs. The typed artifact is `ToolEffects`.

## 5. Representative tools (name, URL, license, one line)

- MCP specification 2025-06-18 -- https://modelcontextprotocol.io/specification/2025-06-18 -- spec text MIT/CC (UNKNOWN exact) -- tool annotations, elicitation, sampling, security considerations.
- Anthropic `@anthropic-ai/sandbox-runtime` -- referenced from https://code.claude.com/docs/en/sandboxing -- license UNKNOWN (not fetched) -- Seatbelt/bubblewrap + hostname-allowlist proxy sandbox for shell commands.
- Claude Code permissions / auto-mode classifier -- https://code.claude.com/docs/en/permissions , https://code.claude.com/docs/en/permission-modes -- proprietary -- deny/ask/allow rules, hooks, classifier, circuit breaker.
- OpenAI Operator / CUA system cards -- https://cdn.openai.com/operator_system_card.pdf , https://cdn.openai.com/pdf/4375e605-f9a6-438d-bcc8-190599c183a6/o3_cua_system_card.pdf -- proprietary -- confirmations, Watch Mode, refusals, screenshot injection monitor.
- Microsoft Windows 11 agent workspace / agent accounts -- https://learn.microsoft.com/en-us/windows/security/book/operating-system-agentic-security -- proprietary OS feature (preview) -- separate standard account + contained desktop + known-folder scoping.
- Microsoft UFO2 -- https://github.com/microsoft/UFO (docs https://microsoft.github.io/UFO/ufo2/overview) -- MIT (per repo; not re-verified this session) -- Windows multi-agent GUI+API automation with PiP virtual desktop.
- Windows Sandbox -- https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/ -- Windows Pro+ feature -- disposable hypervisor-isolated desktop, .wsb config.
- Firecracker -- https://github.com/firecracker-microvm/firecracker -- Apache-2.0 -- microVM with jailer/seccomp and snapshot/restore; no GPU.
- Lume (trycua) -- https://github.com/trycua/lume -- MIT (per listing) -- macOS/Linux VMs on Apple Silicon for computer-use agents.
- Tart (Cirrus Labs) -- https://github.com/cirruslabs/tart -- "fair-source" (terms not read) -- macOS/Linux VMs via Virtualization.framework, OCI distribution.
- Qubes OS -- https://www.qubes-os.org -- GPL-2.0 -- per-VM GUI isolation with dom0-drawn trust borders.
- xdg-desktop-portal RemoteDesktop -- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html -- LGPL -- consent-gated synthetic input on Wayland (libei / Notify* methods, restore tokens).
- Deno permissions -- https://docs.deno.com/runtime/reference/permissions -- MIT -- reference design for scoped allow/deny flags.
- CaMeL -- https://arxiv.org/abs/2503.18813 -- paper -- capability-tagged data flow + policy at tool call.
- Spotlighting -- https://arxiv.org/abs/2403.14720 ; https://github.com/realArcherL/spotlighting-datamarking -- paper / MIT (lib) -- marking untrusted input.
- VPI-Bench -- https://arxiv.org/abs/2506.02456 -- paper + dataset -- visual prompt injection benchmark for CUAs.
- OWASP Top 10 for Agentic Applications (2025-12-09) -- https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/ -- CC BY-SA -- ASI02 "Tool Misuse" etc.; full PDF not read.
- REA -- local checkout -- license at `rea-src/LICENSE` (not read this session) -- effects-declared MCP tools with derived annotations.
- Agent VCR -- https://pypi.org/project/ai-agent-vcr/ -- license UNKNOWN -- JSONL run recording + git savepoints + replay.
- Scarab runtime replay debugger -- https://scarab-project.github.io/scarab-runtime/cli/replay.html -- UNKNOWN -- unified audit/observation/snapshot timeline with rollback.

## 6. Strengths

- OS boundaries (separate account, VM, Windows Sandbox) hold regardless of model behaviour; Claude Code docs make this the explicit argument for layering.
- Declared effects -> derived annotations (REA) remove tool-name heuristics and give clients a principled auto-approve/prompt split.
- Confirmations are measurable and improvable (OpenAI publishes recall), and cover "model mistake" and "injection" simultaneously.
- Screenshot classifiers are cheap to update ("79% to 99% in just one day") relative to retraining.
- Separate agent OS account gives attribution for free (event logs, file ownership) and a revocation point (disable account).
- Snapshot/restore (VM or FS) makes exploratory automation of unfamiliar apps safe to retry.
- 1201(f) / Art. 6 / s.296A give a real legal footing for interop RE of software the user lawfully holds, and Art. 8 / s.296A void contrary EULA terms in the EU/UK.

## 7. Failure modes

- **Textual matching is not a boundary**: `rm` blocklists bypassed via scripts; Claude Code `excludedCommands` matching is textual; Cursor allowlist silently flipped.
- **Sandbox fallbacks**: Claude Code runs unsandboxed by default if the sandbox cannot start; many tools (Read/Edit/MCP servers/hooks) are outside it anyway. INFER: a control plane whose MCP server itself runs on the host is outside any shell sandbox it spawns.
- **Separate OS user is porous**: same kernel; shared `/tmp`, D-Bus/session buses, X11 (any client can read others' input), clipboard; macOS TCC grants are per-user but Screen Recording needs admin/MDM for a standard user; Windows agent-account preview grants read access to "other resources that are accessible to all accounts".
- **Nested X servers are not sandboxes** (Xephyr/Xpra); `/dev/uinput` tools bypass Wayland portal consent.
- **Reversibility is local only**: "You cannot undo the call" -- emails sent, purchases made, external API writes are not snapshot-revertible; checkpoint restore can *re-execute* irreversible actions (semantic rollback).
- **Confirmation fatigue / over-refusal**: Operator over-refused "substantially more than GPT-4o" initially (overrefuse 0.3 -> 0.13 o3); Claude Code's `dontAsk` and `bypassPermissions` exist precisely because users turn prompts off ("YOLO mode", "Turbo").
- **Classifier blind spots**: Operator monitor 90% precision = ~1 in 10 flags is a false positive; VPI-Bench shows CUAs up to 51% deceived even with system-prompt defences; adaptive attackers (EVA, Gray Swan 4.7% single -> 33.6% at 10 attempts -> 63% at 100 against Opus 4.5) beat static defences.
- **Annotations are untrusted** by spec; a malicious or sloppy server can mark a destructive tool read-only. Only the client's own effect inference or a trust list fixes that.
- **Elicitation cannot carry secrets**, so credential-entry needs a UI path outside MCP.
- **Snapshot re-use** duplicates RNG/tokens/network identity (Firecracker doc).
- **Host-level agent features are admin-gated and preview**: Windows agent workspace needs an admin toggle, is "experimental", and Tom's Hardware reported early Insider builds were non-functional beyond the toggle.

## 8. Trade-offs vs other technique families

- vs **accessibility-API / UI-automation observation** families: those decide *how* to see and act; this family decides *whether* a given act is allowed and *what happens if it was wrong*. Authority metadata must be attached at the primitive level (UIA invoke vs. UIA read; AppleScript `tell app` vs. `get`) or the gates have nothing to key on.
- vs **process injection / Frida / debugger** families: injection grants the target process's full authority ("with that process's memory, filesystem, and network authority" -- REA). INFER: `inject`/`attach` should be the highest tier of declared authority, require per-target explicit grant, and be unavailable to the agent inside the same session that reads untrusted screen content unless gated by a human confirmation.
- vs **VM/full-desktop sandboxes**: strongest boundary, but the user's actual installed apps and licences are on the host; Windows Sandbox explicitly lacks host apps. Separate-account-on-host trades boundary strength for "drives the real software the user owns".
- vs **browser-only agents** (Operator, Claude for Chrome): they get site-level permissions and URL blocklists for free; a local control plane has no URL to key on -- the analogue is app identity (bundle ID / signed exe / window class) + action class, and vendor "restricted categories" become "restricted apps/actions" (password managers, banking apps, security tools, licence dialogs).
- vs **pure policy engines (CaMeL-style)**: provable control-flow guarantees but a policy-authoring burden and utility loss (77% vs 84%; dual-LLM 49.7% -> 14.6% in one study). INFER: a pragmatic middle is effect-derived annotations + classifier + confirmations, with CaMeL-style data-provenance tags reserved for the small set of exfiltration-capable actions (network send, clipboard write to another app, file write outside workspace).

## 9. Integration notes for an MCP control plane

Process model
- Run the control plane as a *separate OS principal* (dedicated standard account on Windows/macOS/Linux) with its own desktop/session; keep the user's primary session untouched. On Windows 11 the platform now provides this (agent account + workspace, admin opt-in, preview); elsewhere create the account yourself and grant TCC/portal permissions to that account only. Fall back to a VM (Lume/Tart on Apple Silicon; QEMU/Hyper-V/Windows Sandbox elsewhere) when the target app can be installed inside it.
- Keep the MCP server process minimal-privilege; never run as root/admin (Claude Code refuses `bypassPermissions` under sudo; REA never invokes sudo and uses `pkexec` for installs).
- Follow REA's `WindowsAuthority` pattern: isolation claims carry `proof` status; the server reports "isolation: not-proven" rather than silently running unsandboxed (contrast Claude Code's default fallback).

Authority declaration
- Give every tool a typed effects record (copy REA's `ToolEffects` fields; add `readsScreen`, `readsClipboard`, `sendsInput`, `targetScope`) and derive the four MCP hints from it. Add REA's "result may contain target-originated text" bit so the client can mark untrusted observations.
- Authority tiers (INFER, informed by REA prose + Windows principles): `observe` (screenshot, UIA tree read, window list) < `launch` (start an app the user allow-listed) < `drive` (send input / invoke UIA actions in an allow-listed window) < `attach` (debugger/accessibility hooks on a running process) < `inject` (code into process) < `mutate-system` (install, registry, settings). Default grant: observe + drive on allow-listed apps; everything above requires per-target elicitation/confirmation and is denied in `dontAsk`-style headless modes.
- Expose a Deno-style config: `allow-app`, `allow-path`, `allow-host`, `deny-*` overrides, per-session vs persistent grants (mirror Claude Code "Yes, and don't ask again" scoping and the portal `persist_mode`/`restore_token` model).

Auth / permission prompts
- Use MCP elicitation for structured confirmations (what/where/why + accept/decline/cancel) when the client supports it (check `capabilities.elicitation` as REA does at `src/server/mcpClientMetadata.ts:58`); honour decline AND cancel as "do not act". Never request secrets via elicitation; for credential entry implement an Operator-style human takeover (agent pauses, user types into the real window, agent resumes).
- Set a `requiresUserInteraction`-style flag (Claude Code recognises it) on destructive/inject/mutate tools so classifier-based clients never auto-approve them.
- Implement a server-side rule engine in deny -> ask -> allow order; evaluate on *structured* action descriptors (app id, window, action class, target path/host), not on command text.

Latency
- Screenshot/UIA observations are the hot path; the cheap checks (effects lookup, allow/deny rules) cost microseconds. Classifier scans of screenshots (Anthropic/OpenAI pattern) add a model call per step -- run them only when the next intended action is non-read-only, and only on observations tagged "untrusted text present".
- Confirmations are the real latency: budget one per consequential action, batch via "plan then execute with diff" (show the plan, approve once, execute with a per-step kill switch) rather than per-click prompts, which drive users to YOLO modes.

State
- Append-only action log: {timestamp, tool, effects, target identity (pid/bundle/window), args hash, observation hash before/after, approval source (user/rule/classifier), outcome}. Store screenshots by hash, redact secrets as REA does (`explicitSensitiveFailure.ts` redaction; Frida Evidence excludes authentication values).
- Reversibility: VM/FS snapshot before each `mutate-system` or bulk-file action; per-action undo where the app exposes it (Cmd/Ctrl-Z, trash instead of delete); mark external-effect actions as irreversible in the log and require idempotency keys on retry (ACRFence's semantic-rollback warning).
- Kill switches: global hotkey/UI "stop" that halts input injection and releases the target (REA: never kill the target; release instrumentation and report "liveness unknown"); auto-pause on user-inactivity for sensitive apps (Watch Mode analogue); circuit breaker on N consecutive denials; rate limit on actions/minute and on app launches.
- Dry run: every `drive` tool should accept `dry_run: true` returning the resolved target + effects + predicted UI change without sending input; plan mode = all tools forced dry-run.

## 10. Authorization and legal notes

Software ownership vs licence
- Most software is licensed, not sold. Automation of a GUI you lawfully run is generally not a copyright act, but EULAs may prohibit automation, bots, or reverse engineering; in the US such terms have been enforced as contract (Bowers v. Baystate, 320 F.3d 1317 (Fed. Cir. 2003): no-reverse-engineering shrinkwrap term not preempted by the Copyright Act, Dyk dissenting; Davidson v. Jung / bnetd, 422 F.3d 630 (8th Cir. 2005): EULA/ToU enforced and 1201(f) defence rejected on its facts).
- CFAA after Van Buren (2021): ToS violations alone are not "exceeds authorized access"; liability keys on code-based gates. Contract remedies survive. INFER: automating the user's own local software does not implicate the CFAA; automating *another party's service* through that software (e.g., a game client talking to a server) can breach ToS and, if it bypasses code-based gates, more.

Interoperability RE safe harbours (OBSERVED statutory text via Cornell LII and legislation.gov.uk)
- 17 U.S.C. 1201(f)(1): a person who "has lawfully obtained the right to use a copy of a computer program" may circumvent access controls on "a particular portion of that program for the sole purpose of identifying and analyzing those elements... necessary to achieve interoperability of an independently created computer program with other programs," to the extent not previously readily available and not infringing. (f)(2) permits developing the means "if such means are necessary to achieve such interoperability"; (f)(3) permits sharing "solely for the purpose of enabling interoperability"; (f)(4) defines interoperability as "the ability of computer programs to exchange information, and of such programs mutually to use the information which has been exchanged." Copyright Office later clarified 1201(f) does permit a consumer's own acts of circumvention for program-to-program interoperability (secondary summary of the 1201 report).
- Directive 2009/24/EC Art. 6: reproduction/translation without authorisation permitted when "indispensable to obtain the information necessary to achieve the interoperability of an independently created computer program", if (a) by licensee/person with right to use or on their behalf, (b) "the information necessary to achieve interoperability has not previously been readily available", (c) confined to the parts necessary. Art. 6(2): not for other goals, not passed to others except as necessary, not for a substantially similar program. Art. 6(3): Berne three-step limit. Art. 8: contractual provisions contrary to Art. 6 (and Art. 5(2),(3)) are null and void. Art. 5(3) separately permits observing/studying/testing the program's functioning while loading/running it. CJEU Top System C-13/20 (2021-10-06): lawful acquirer may decompile to correct errors under Art. 5(1) without meeting Art. 6; contract may not exclude the possibility of correction entirely.
- UK CDPA s.50B (decompilation) / s.50BA (observe, study, test) / s.296A(1): any term "shall be void in so far as it purports to prohibit or restrict... (b) where the conditions in section 50B(2) are met, the decompiling of the program; or (c) the observing, studying or testing of the functioning of the program in accordance with section 50BA."
- Fair use backdrop (US): Sega v. Accolade (9th Cir. 1992) and Sony v. Connectix (9th Cir. 2000): intermediate copying for interoperability is fair use.

Why DRM / licence / anti-cheat / security-control circumvention is out of scope
- 1201(a)(1)(A): "No person shall circumvent a technological measure that effectively controls access to a work"; (a)(3)(A) circumvent = "descramble... decrypt... or otherwise to avoid, bypass, remove, deactivate, or impair a technological measure, without the authority of the copyright owner." 1201(f) saves only interoperability RE of *computer programs*; courts hold it "does not authorize circumvention of technological systems that control access to other copyrighted works" (secondary quote of case law).
- Circuit split on infringement nexus: Chamberlain v. Skylink, 381 F.3d 1178 (Fed. Cir. 2004) requires a "reasonable relationship" to infringement; MDY v. Blizzard, 629 F.3d 928 (9th Cir. 2010) rejects any nexus requirement and held trafficking a bot (Glider) that evaded Warden violated 1201(a)(2). Bungie v. AimJunkies: arbitrator (2023) found liability under 1201(a)(1), (a)(2) and (b) for cheat software ($4.3M), jury (2024) found copyright infringement, affirmed on appeal (secondary). A local-software control plane that interacts with anti-cheat, licence checks, or DRM is therefore in the MDY/AimJunkies fact pattern regardless of the user's "I own it" intuition.
- "Security controls" (EDR, sandbox escapes, authentication, bot protection) are excluded for a different reason: CFAA code-based gates (Van Buren), plus REA's and every vendor's usage policy ("bypassing authentication, bot protection or access controls"; OpenAI: users "prohibited from bypassing any protective measures... including rate limits or restrictions and safety mitigations").
- Triennial 1201 exemptions (ninth rulemaking, final rule effective 2024-10-28, runs to Oct 2027; next cycle begins ~June 2026): the Register recommended renewing all petitioned exemptions and granting three of seven new classes; repair exemptions are "limited to lawfully acquired equipment and cannot be used to access other copyrighted works." Class-by-class list not read (UNKNOWN which three). INFER: exemptions are purpose-specific (repair, security research, accessibility) and do not create a general "my computer, my rules" licence for an automation agent.

Operational guidance for the project (INFER from the above)
- Scope statement in the product: "software you own or are authorized to automate"; hard-exclude apps/processes whose identity matches known anti-cheat/DRM/licensing/EDR components (deny-list by signer/publisher), and refuse `inject`/`attach` on them even with user confirmation.
- Keep REA's distinction: "Authorization to inspect an artifact does not necessarily authorize execution, interception, device changes, or interaction with another party's system." Make `observe` vs `drive` vs `attach/inject` separately grantable.
- Log the authority basis per session (user-asserted ownership/licence, purpose) alongside the action log; this is what a 1201(f)/Art. 6 analysis would need later.
- Where the agent must talk to a remote service through a local client, treat that service's ToS as the governing document, not the local EULA.

## 11. Open questions

- Does any MCP client today surface `destructiveHint`/`openWorldHint` as distinct prompt styles, and how many honour `requiresUserInteraction`? (Only Claude Code observed.)
- Exact wording/scope of Operator's "takeover mode" and the blocked-site categories (launch blog returned 403).
- Current state of Windows agent workspace: is the workspace a true separate session with its own window station, and does the agent account get Screen Recording-equivalent access to the user's desktop or only its own? Microsoft text says it limits "visibility and access the agent has to the user's desktop activity" but gives no mechanism.
- macOS: can a standard (non-admin) agent account self-grant Accessibility and Automation (Apple Events) TCC permissions, or is MDM/PPPC required? Secondary sources say Screen Recording needs admin/profile; Accessibility/Automation UNKNOWN.
- Apple EULA limit on concurrent macOS guest VMs per host (commonly cited as two) -- not verified (search budget).
- Whether REA actually invokes `elicitation/create` anywhere (only capability detection found).
- Agent SDK `canUseTool` contract details and `permission_prompt_tool` headless flow -- page redirected three times and was not fetched.
- OWASP Agentic Top 10 full text (ASI IDs, mitigations) -- only landing page fetched.
- Which three new 1201 exemption classes were granted in 2024.
- EU AI Act Art. 14 human-oversight / "stop" requirements as applied to consumer agents -- not researched.
- Empirical: does an OS-account boundary materially reduce injection impact when the agent's whole purpose is to read the user's documents/email in the user's apps? (It changes *attribution and revocation*, not *exposure*.)

## 12. Sources

Primary (fetched/read 2026-11-04 unless dated)
- Anthropic computer-use tool docs: https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/computer-use-tool
- Anthropic, "Prompt injection defenses" (Claude for Chrome), 2025-11-24: https://www.anthropic.com/research/prompt-injection-defenses
- Claude Code sandboxing: https://code.claude.com/docs/en/sandboxing
- Claude Code permissions: https://code.claude.com/docs/en/permissions ; permission modes: https://code.claude.com/docs/en/permission-modes
- OpenAI Operator System Card, 2025-01-23 (updated 2025-03-11): https://cdn.openai.com/operator_system_card.pdf
- OpenAI o3 Operator / CUA system card addendum: https://cdn.openai.com/pdf/4375e605-f9a6-438d-bcc8-190599c183a6/o3_cua_system_card.pdf
- Microsoft, Windows 11 security book, Agentic security (ms.date 2025-11-13): https://learn.microsoft.com/en-us/windows/security/book/operating-system-agentic-security
- Microsoft, "Securing AI agents on Windows", 2025-10-16: https://blogs.windows.com/windowsexperience/2025/10/16/securing-ai-agents-on-windows/
- Microsoft, "Securing the Model Context Protocol...", 2025-05-19: https://blogs.windows.com/windowsexperience/2025/05/19/securing-the-model-context-protocol-building-a-safer-agentic-future-on-windows/
- Microsoft support, Experimental agentic features: https://support.microsoft.com/windows/experimental-agentic-features-a25ede8a-e4c2-4841-85a8-44839191dfb3
- Windows Sandbox overview (ms.date 2026-03-29): https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/
- MCP spec 2025-06-18 Tools: https://modelcontextprotocol.io/specification/2025-06-18/server/tools ; Elicitation: https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation ; Sampling: https://modelcontextprotocol.io/specification/2025-06-18/client/sampling
- Firecracker design: https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md ; snapshot support: https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md
- xdg-desktop-portal RemoteDesktop: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html
- Deno permissions: https://docs.deno.com/runtime/reference/permissions
- CaMeL, arXiv 2503.18813: https://arxiv.org/abs/2503.18813
- VPI-Bench, arXiv 2506.02456: https://arxiv.org/abs/2506.02456
- Spotlighting, arXiv 2403.14720: https://www.microsoft.com/en-us/research/publication/defending-against-indirect-prompt-injection-attacks-with-spotlighting/
- 17 U.S.C. 1201: https://www.law.cornell.edu/uscode/text/17/1201
- Directive 2009/24/EC Art. 6: https://www.legislation.gov.uk/eudr/2009/24/article/6
- CDPA 1988 s.296A: https://www.legislation.gov.uk/ukpga/1988/48/section/296A
- Copyright Office ninth triennial 1201 rule: https://copyright.gov/newsnet/2024/1057.html ; https://copyright.gov/1201/2024/
- MDY v. Blizzard summary: https://en.wikipedia.org/wiki/MDY_Industries,_LLC_v._Blizzard_Entertainment,_Inc.
- Lume: https://github.com/trycua/lume ; Tart: https://github.com/cirruslabs/tart
- REA checkout (local): `DISCLAIMER.md`, `SECURITY.md`, `src/contracts/toolEffects.ts`, `src/process/WindowsAuthority.ts`, `src/server/mcpClientMetadata.ts`, `docs/tool-design.md`, `docs/frida-instrumentation.md`, `docs/design/limits-and-recovery.md`, `docs/capability-composition.md`

Secondary (search-result summaries; not independently verified)
- Claude 4 system card prompt-injection figures via Simon Willison: https://simonwillison.net/2025/May/25/claude-4-system-card/ ; Anthropic transparency hub: https://www.anthropic.com/transparency
- Gray Swan / Opus 4.5 figures: https://the-decoder.com/claude-opus-4-5-resists-prompt-injections-better-than-rivals-but-still-falls-to-strong-attacks-alarmingly-often/
- UFO2 paper: https://arxiv.org/pdf/2504.14603 ; docs: https://microsoft.github.io/UFO/ufo2/overview
- Windows agent workspace coverage: https://www.tomshardware.com/software/windows/microsofts-vision-of-ai-native-windows-is-becoming-real-update-introduces-agents-that-pilfer-through-your-files-latest-windows-11-insider-build-includes-experimental-ai-agents-toggle-that-can-perform-tasks-for-you-in-the-background
- Codex approval/sandbox modes: https://codex.danielvaughan.com/2026/03/26/codex-cli-approval-modes-sandbox-security
- Pop-up attacks / EVA: https://www.themoonlight.io/review/eva-red-teaming-gui-agents-via-evolving-indirect-prompt-injection
- Dual-LLM / lethal trifecta summaries: https://archestra.ai/docs/platform-lethal-trifecta ; https://jlldavies.github.io/go4-llm-design-patterns/patterns/V4-Dual-LLM.html
- Agent rollback literature: https://arxiv.org/pdf/2603.20625 (ACRFence) ; https://arxiv.org/pdf/2604.13536 ; https://pypi.org/project/ai-agent-vcr/ ; https://scarab-project.github.io/scarab-runtime/cli/replay.html ; https://arize.com/glossary/checkpointing-agents/
- Coding-agent deletion incidents: https://adversa.ai/blog/ai-coding-agent-incidents/ ; https://forum.cursor.com/t/cursor-just-deleted-itself-github-more/150422 ; https://forum.freecodecamp.org/t/the-day-i-asked-ai-to-clean-up-my-project-and-it-deleted-everything-a-wake-up-call-with-google-antigravity/789059
- Hardy, Confused Deputy (1988) and Miller, Robust Composition (2006): https://jscholarship.library.jhu.edu/server/api/core/bitstreams/c2c3ea50-2613-47a7-b33c-9383c706ee63/content ; https://semgrep.dev/blog/2026/security-like-its-1977-capabilities-for-the-modern-agentic-web
- Bowers v. Baystate: https://en.wikipedia.org/wiki/Bowers_v._Baystate_Technologies,_Inc. ; https://law.justia.com/cases/federal/appellate-courts/F3/320/1317/615628/
- Davidson v. Jung (bnetd): https://law.justia.com/cases/federal/appellate-courts/F3/422/630/569453/
- Chamberlain v. Skylink: https://briefs.lsd.law/the-chamberlain-group-inc-v-skylink-technologies-inc-381-f-3d-1178-2004
- Bungie v. AimJunkies: https://www.mbhb.com/intelligence/snippets/destiny-developer-obtains-first-of-its-kind-jury-verdict-against-hackers/ ; https://www.pcgamer.com/games/fps/court-locks-in-bungies-dollar44-million-win-against-destiny-2-cheat-maker-refuses-request-for-a-new-trial
- Van Buren v. United States: https://www.eff.org/cases/van-buren-v-united-states
- Sega v. Accolade / Sony v. Connectix: https://law.resource.org/pub/us/case/reporter/F3/203/203.F3d.596.99-15852.html
- CJEU Top System C-13/20: https://www.scl.org/12394-court-of-justice-rules-that-software-buyers-can-decompile-programs-to-correct-errors/
- Copyright Office 1201(f) interpretation history: https://www.project-disco.org/intellectual-property/070617-interoperability-and-the-copyright-offices-section-1201-report/
- Qubes GUI architecture: https://www.qubes-os.org/news/2020/03/18/gui-domain/ ; https://lwn.net/Articles/704287/
- Xephyr not a sandbox: https://bbs.archlinux.org/viewtopic.php?id=305614
- macOS TCC/PPPC for standard users: https://support.kandji.io/pppc-deny ; https://support.addigy.com/hc/en-us/articles/4403549601043-Privacy-Preferences-Policy-Control-PPPC-for-Standard-Users
- OWASP Agentic Top 10 summaries: https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/ ; https://graylog.org/post/what-is-the-owasp-top-10-agentic-ai
