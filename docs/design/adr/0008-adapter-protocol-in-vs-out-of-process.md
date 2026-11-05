# ADR 0008: Adapter protocol, in-process vs out-of-process

## Context
REA has six bridge shapes (pipe-REPL, pipe-MCP, socket+bearer, socket+HMAC, external MCP, one-shot helper). The reports'
binding taxonomies mix transport, protocol and technique (CRITIC.md).

## Decision
- **Rule 1.** A spec-shaped surface (CLI argv, MCP tools, OpenAPI HTTP, gRPC reflection) gets an in-process generic Rust
  driver inside `aad`. The driver spawns or connects to the target and never loads target code.
- **Rule 2.** A surface that needs a foreign runtime (Python import for Click introspection, Swift for AX/sdef/App Intents, C# for UIA,
  an in-host plugin bridge) gets an **out-of-process adapter** speaking `AdapterService` (`adapter_service.proto`). The adapter has two framings
  of the same messages:
  - **one-shot:** `<exe> --aa-oneshot <Method>`, proto3-JSON on stdin and JSONL on stdout, with reserved exit codes 64/65/69/70/75. Used
    for harvesters (slice 1's `aa_click_info.py`) and stateless helpers.
  - **gRPC over UDS** (M2): long-lived adapters, sessions, in-host bridges. Each run gets a private 0700 directory and a 0600 descriptor
    `{run_id, socket_path, token, max_frame_bytes}`. Only the descriptor *path* goes in the env. `Handshake` must echo run_id and
    frame limit and report the exact operation set. `expected_generation` is checked on every mutating call, and
    `GetRequestStatus` provides receipts.
- **The adapter proposes and the engine decides.** Adapters return raw artifacts (stored verbatim in the CAS) and raw native signals.
  The engine runs `candidates()` and characterization, so a buggy or hostile adapter cannot self-certify.
- REA shape mapping: one-shot helper = one-shot framing; socket+bearer = adapter gRPC; pipe-MCP/external MCP = the `mcp` binding
  (M2); pipe-REPL and socket+HMAC are deferred.
- Bridge kits (`adapters/python/aa_adapter_kit`, later Swift) provide the descriptor reader, framing, closed method table and limits.
  `aa adapter conformance -- <cmd>` checks handshake echo, limit refusal, unknown-method errors, oversize frames, poison-on-deadline, and no
  orphans.

## Consequences
- Foreign code never runs inside `aad`. Crash isolation and adapters in any language come with that.
- Two framings means two code paths, kept honest by one conformance suite.

## Alternatives
- *Everything out-of-process* (safety-first): extra hops for generic drivers with no added safety.
- *dylib plugins*: rejected.
