# Research Prompt: Agentic-Anything (extending/distilling REA)

> Paste the block below to Fable as the task. It's framed as research + strategy,
> scoped to authorized automation of software you own. No implementation requested.

---

Research task (educational / personal-automation, no implementation needed yet).

Context: I own a repo scaffolded to extend or distill the open-source project
"REA — Reverse Engineer Anything" (github.com/morluto/rea, the `rea-agents` MCP
server). My long-term goal is an "agentic-anything" layer: a general-purpose
control plane that lets an AI agent observe and drive software on my own machine
through a single API/MCP, including desktop apps and binaries that expose no
public API or scripting interface. This is for interoperability and automation of
software I own or am authorized to use. It is not for defeating DRM/licensing,
copying protected assets, or evading security controls.

What I want from you:

1. Explain how REA is architected: how its MCP server exposes tools to an agent,
   how it bridges to analysis engines, and which design choices make it extensible.
2. Survey the established, legitimate techniques for giving an agent a control
   interface over existing software when no API exists — e.g. accessibility/UI
   automation trees, OS automation APIs, computer-use/vision agents, CLI/IPC
   wrapping, documented plugin/extension points, and dynamic instrumentation
   (Frida-style) for software I control. Note the trade-offs and failure modes of
   each.
3. Propose a strategy for distilling REA's patterns into a reusable "universal
   agentic API" layer: architecture, how a capability/tool registry would work,
   how to add a new target app, and where the hard problems are.
4. Flag the real constraints up front: authorization/scope, supply-chain trust of
   npx-installed tooling, licensing of bridged commercial tools, and safety rails
   for an agent that can drive arbitrary software.

Keep it research and strategy. I'll handle implementation myself later.

---

## Framing notes (keep these in your own words whenever you reuse the prompt)

- State that the targets are software you **own or are authorized to automate**.
- Name what it is **not** for: DRM/license defeat, protected-asset extraction,
  evading security controls. That sentence is what distinguishes this project from
  misuse — it's accurate scoping, not classifier-gaming.
