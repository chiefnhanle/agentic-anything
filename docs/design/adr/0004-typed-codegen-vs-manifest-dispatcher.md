# ADR 0004: Typed code generation vs. a manifest dispatcher

## Context
The reports conflict (CRITIC.md):
- REA's registry rule says to keep hand-wired typed tables and **no generic JSON dispatcher**
  (`rea docs/capability-composition.md:26-27`).
- The walkthrough wants a new target to touch zero shared files.
- Discovery proposes a policy-checked `invoke(capability_id, args)`.
- Generators propose a code-mode renderer.

REA's real cost is about 12 files per tool and 7-10 shared seams per target. The "EASY" goal cannot afford per-capability code.

## Decision
- The reason REA forbids a dispatcher is that **arbitrary code is selected by untyped data**. We keep that prohibition and allow
  **data-driven capabilities over a closed set of typed drivers**:
  - `Binding` is a closed proto `oneof` (`cli`, `mcp`, `http`, `grpc`, `adapter`). Each variant has exactly one Rust driver
    implementation, matched exhaustively. A set-equality test asserts that every variant has a driver.
  - A driver builds its request only from schema-validated input. `cli_argv` builds an argv **vector** and never a shell
    string. `--opt=value` is used for options, and positionals follow `--`.
  - Capability data never selects code. It selects an address (argv path, method, operation id) inside an already-typed
    driver.
- We do **not** generate Rust code per capability. The lock is loaded at runtime. Adding a target that uses an existing binding
  kind touches zero Rust files.
- `aa x <capability-id> --input-json …`, the MCP search profile's `aa_invoke`, and `RuntimeService.Invoke` are one path.
  Each call is checked against the canonical schema and the policy **for the underlying capability**.
- Code-mode exposure (one `execute` running many operations) is deferred. Per-call authorization would need host callbacks.

## Consequences
- A new spec-shaped target is a manifest only. A new *kind* of surface is one new driver plus one new oneof arm, reviewed once.
- Type safety moves from compile time to a load-time check plus tests. Every lock is validated at load: closed schema, a
  binding that resolves against the harvested address, and effects that are decided or withheld.

## Alternatives
- *REA-style per-capability codegen into Rust*: safe, but every target becomes a code change plus a rebuild.
- *Dynamic plugins (dylib/WASM) per target*: they would load foreign code into the engine. WASM stays an option for pure-data
  harvesters later.
