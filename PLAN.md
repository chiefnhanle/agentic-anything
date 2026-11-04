# agentic-anything — Plan & cold-start state (as of 2026-11-04)

## Goal
Distill REA (github.com/morluto/rea, `rea-agents` MCP server) into a universal
observe + drive control plane: one MCP/API through which an agent observes and
drives software the owner owns or is authorized to automate. Not for DRM/licence
defeat, protected-asset extraction, or evading security controls.

## What exists
- Issue #1: roadmap epic (labels, 5 milestones M0–M4 created on GitHub).
- `RESEARCH_PROMPT.md`: the original research brief.
- `docs/research/`: 11 full reports (REA mcp-server-adapter, contracts-and-evidence,
  runtime-observation-surfaces, setup/supply-chain/licensing; surveys of a11y trees,
  OS automation APIs, computer-use/vision, CLI/IPC wrapping, plugin/extension points,
  prior art, safety-rails/authorization).
- `docs/research/salvaged/`: structured summaries only (full reports lost) for REA
  capability-registry-and-composition, engine-bridges-and-process-supervision,
  tool-design-philosophy-and-verification, and the dynamic-instrumentation survey.

## Key findings so far
- REA: dependencies flow inward (domain → contracts → providers → application → CLI/MCP
  adapters). Tools bind to named zod contracts; catalog order never selects handlers.
  `src/composition/capabilityDeclarations.ts` + `capabilityServices.ts` = registry.
  Every result is an Evidence record (observation/derivation/inference/unknown, evidence_id).
  External engines are wrapped by owned processes (`src/process/`): ownership proof,
  deadlines, bounded output, private runtime roots, cleanup retry, fail-closed.
  Bridge pattern: a script inside the host app (Hopper Python, Ghidra Java) exposes a
  token-authenticated local socket.
- REA deliberately removed in-tool permission grants/elicitation (their PR #555) and relies
  on OS permissions. A *driving* layer must revisit this: authority levels
  (observe/launch/attach/inject/mutate) + MCP tool annotations + confirmations.
- Technique ladder (prefer structured first): plugin/scripting API → existing IPC/protocol
  (CDP, LSP, DAP, D-Bus, COM, AppleScript) → OS automation → accessibility tree
  (UIA / AX / AT-SPI2) → dynamic instrumentation (own software only) → vision fallback.

## Next steps (cold start)
1. Read `docs/research/*.md` (start with prior-art and safety-rails).
2. Design phase: 3 independent architecture proposals → judge → synthesis
   (`docs/design/architecture.md`, ADRs for: authority model, adapter contract,
   action-record schema, registry format, confirmation policy).
3. Issue plan: create child issues under #1, grouped by milestone:
   - M0: ADRs above; WSL2→Windows GUI feasibility spike; licence review of bridged tools.
   - M1: MCP server skeleton; capability registry; adapter contract; authority declaration.
   - M2: adapters: a11y (UIA/AT-SPI), AppleScript/COM, CDP/Electron, PTY/CLI wrap, plugin bridge.
   - M3: action records + audit log; confirm-before-irreversible; allow/deny lists; kill switch.
   - M4: dependency pinning/npx trust; setup flow; verification lanes; docs.
4. Edit #1 to list the child issues.

## Workflow resume
Script: ~/.claude/projects/-home-cukin-agentic-anything/65f00ee4-6743-473f-85fc-7ebf788d62a2/workflows/scripts/research2.js
Last run wf_50d88a98-8e3 (stopped before critic/gap-fill; all 10 reports completed).
REA source clone used: scratchpad `rea-src` (re-clone: `git clone --depth 1 https://github.com/morluto/rea.git`).
