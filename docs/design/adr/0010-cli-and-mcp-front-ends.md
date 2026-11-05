# ADR 0010: CLI and MCP front ends

## Context
REA hand-writes CLI commands separately from MCP tools and keeps parity tests. The default exposure mode is contested: typed MCP,
a curated ≤30 set, search plus invoke, or code mode (CRITIC.md). Host behaviour around annotations and elicitation is not
verified (CRITIC P3 is open).

## Decision
- **`aa` (crate `aa-cli`)** is a gRPC client of `agent.sock`. Operator subcommands (`approve`, `unhalt`, `assume`, `grant`)
  connect to `operator.sock`. In Tier 1 that socket is unreachable to the agent's uid.
  - Static engine-ops commands: `daemon`, `status`, `halt`, `unhalt`, `target add|ls|show|assume`, `import cli-anything`, `harvest`,
    `discover` (harvest + compile), `compile`, `verify`, `distill`, `list`, `describe`, `search`, `invoke` (alias `x`), `run`, `audit [--verify]`,
    `approve`, `mcp`, `doctor`. Slice 1 ships the subset listed in build-plan.md §3.6.
  - **`aa x <capability-id> --input-json <inline|file|->`** is the canonical invoke. **`aa run <target> <cli_path…> [flags]`** builds a clap
    tree at runtime from `GetExposure(CLI)`, cached by catalog digest. Its flags are sugar over the input object.
  - Output is pretty on a TTY and compact JSON when piped. `--full-output` returns the envelope `{ok, data|error, meta:{action_id, catalog_digest}}`.
  - Exit codes: `0` completed, `1` failed, `2` usage/schema, `3` needs a human or more authority (confirmation, policy deny, withheld),
    `4` engine unreachable or halted (restart or unhalt the engine; do not retry the call), `130/143` signal after cleanup, `141` EPIPE.
    One classifier maps `ErrorCode` to an exit code.
- **`aa mcp`** is a cargo feature of `aa-cli` (rmcp 3.8.0, stdio) and lands in slice 2. `list_tools` comes from `GetExposure`. It is
  implemented manually because tools are data.
  - Profiles: `typed`, the servable core set of ≤30 per target with derived annotations and `_meta.aa`; and `search`, which is
    `aa_search`/`aa_describe`/`aa_invoke` and is the default when more than 30 capabilities are servable.
  - Names are ≤64 chars with a 6-hex collision suffix.
  - Descriptions are an engine template plus ≤300 chars of vendor text inside an explicit untrusted block. Withheld capabilities are not tools.
  - No MCP tool can approve anything. Elicitation is relayed only as host-mediated consent (ADR 0002). Without elicitation, a
    confirmation returns `CONFIRMATION_REQUIRED` with "run `aa approve <id>` in your terminal".
  - `aa mcp doctor` spawns the real server as a client.
- **SKILL.md** is rendered per target from the lock, following agentskills.io limits: `name` matches its directory, description ≤1024, body ≤500 lines.
  It contains the core table with levels, `aa x`/`aa run` fallbacks, a withheld section with reasons, the rule "trust the live catalog over this skill",
  and the lock and catalog digests in `metadata`. One hand-written `skills/aa-distill/SKILL.md` teaches the proposal loop and states the fence.
- **Catalog digest:** SHA-256 over JCS of the sorted exposure projection (REA `catalogIdentity.ts`). It is exposed in status, and
  `expected_catalog_digest` lets clients detect a stale catalog.

## Consequences
- One lock yields three surfaces, and set-equality tests replace parity tests: the servable set equals the enabled exposure set, which equals the live `tools/list`.

## Alternatives
- *MCP-first with the CLI as an afterthought*: agents with shells use CLIs well, and the CLI is also the operator tool.
- *Code mode*: deferred (ADR 0004).
