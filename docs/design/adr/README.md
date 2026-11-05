# Architecture Decision Records

Status of all ADRs below: **Accepted (2026-11-05)**. They come from the synthesis in
`../architecture.md`, which merged three proposals (`../proposals/`) using the judges' verdicts.
Each ADR states its context, the decision, its consequences and the alternatives considered.

| ADR | Decision |
|---|---|
| [0001](0001-rust-grpc-engine.md) | The core is a Rust gRPC engine (`aad`, tonic 0.14.9 / prost 0.14.4). Every front end is a client of it. |
| [0002](0002-authority-model.md) | Tri-state effects, a derived tier ladder, fail-safe zero values, one gate, and approval authority taken from the listener. |
| [0003](0003-capability-manifest-ir.md) | A human overlay plus a compiled lock (proto3-JSON, JCS digest). LLM output enters only through a typed proposal fence. |
| [0004](0004-typed-codegen-vs-manifest-dispatcher.md) | Data-driven capabilities run over a closed set of typed Rust bindings. There is no generic code dispatcher and no per-capability codegen. |
| [0005](0005-transport-and-local-auth.md) | Two UDS listeners (`agent.sock`, `operator.sock`) with peer-uid checks. Loopback TCP is opt-in and agent-only. |
| [0006](0006-action-record-and-audit-log.md) | One ActionRecord per attempt. The audit log is hash-chained JSONL with intent written before dispatch, and the engine boots halted if the chain fails to verify. |
| [0007](0007-confirmation-policy.md) | Admission is set by effect level. Confirmations are bound to an action digest. A grant lifecycle ships as one unit. |
| [0008](0008-adapter-protocol-in-vs-out-of-process.md) | Generic bindings run in-process. Foreign runtimes run out-of-process over `AdapterService`, which has two framings. |
| [0009](0009-cli-anything-ingestion.md) | CLI-Anything harnesses are imported from a local checkout and never trusted. `install_cmd` is never run. |
| [0010](0010-cli-and-mcp-front-ends.md) | `aa` CLI (static and dynamic trees), `aa mcp` (typed or search profile) and SKILL.md are all rendered from the lock. |
| [0011](0011-verification-ladder-and-confinement.md) | Lanes L0-L8 are code. Effects are lowered only by observation at a sufficient isolation, and capabilities are served under the confinement they were verified in. |
| [0012](0012-process-supervision.md) | `aa-exec` provides a focused supervision kernel, not a full port of REA's `src/process`. |
| [0013](0013-workspace-and-crate-layout.md) | 7 crates in slice 1. A crate is split only when the dependency lint or build times require it. |
