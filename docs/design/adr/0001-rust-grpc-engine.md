# ADR 0001: The core is a Rust gRPC engine

## Context
The owner set a hard constraint: the core is a Rust gRPC engine built on tonic and prost. The `aa` CLI, an MCP server and
generated Agent Skills are all clients of it. REA shows what happens otherwise: front ends are a second implementation of policy. It
registers about 234 MCP tools and about 166 CLI commands through separate paths that need parity tests
(`research/rea-cli-surface-and-agent-packaging.md`). All three proposals agreed on the engine shape. They differed on proto
layout and crate count.

## Decision
- One daemon, `aad`, from crate `aa-engine`, implements every service in one proto package,
  `agentic_anything.v1` (`proto/agentic_anything/v1/*.proto`). That is 8 services and 41 RPCs, with
  `AdapterService` served by adapters and called by the engine.
- Stack, verified by a scratch build on 2026-11-05: tonic 0.14.9, tonic-prost 0.14.9, prost/prost-types 0.14.4,
  tonic-prost-build 0.14.9, and pbjson/pbjson-types/pbjson-build 0.9.0 (these depend on prost 0.14). prost-reflect 0.16.5
  is a **build-dependency only**. `rust-version = "1.88"`, toolchain 1.99.1.
- `aa-proto` has a `grpc` cargo feature. Without it, only messages are generated and tonic is not in the dependency
  tree. This is verified, and it keeps `aa-core` transport-free.
- `build.rs` reads the custom `(agentic_anything.v1.rpc_policy)` MethodOption from the descriptor set and generates a static
  `RPC_POLICY` table. **A method without the option fails the build.**
- All messages are serde-able through pbjson. Lock files, audit lines, `aa --json` and the one-shot adapter framing all use
  proto3-JSON. Digests are SHA-256 over RFC 8785 JCS. The JCS crate is `serde_json_canonicalizer` 0.4.1, tested
  against the RFC 8785 vectors.
- `buf.yaml` (v2) uses lint `STANDARD` minus `ENUM_ZERO_VALUE_SUFFIX` (see ADR 0002) and breaking `FILE`.

## Consequences
- Every policy decision lives in one process. Front ends cannot diverge, because they only render and relay.
- A single package with descriptive file names keeps imports trivial. `buf breaking FILE` protects the on-disk JSON forms
  as well as the wire format.
- The engine is a daemon, so the CLI has to handle autostart and stale sockets. ADR 0005 covers these, and an
  in-process duplex transport is used for tests.

## Alternatives
- *Several proto packages* (`aa.ir.v1`, `aa.engine.v1`, ...), as generator-first and safety-first proposed. Rejected: the
  task requires one package, and cross-package imports give no benefit at this size.
- *Proto as the only IR, with no semantic layer* (mvp-first). Partly adopted: the IR types **are** the generated proto types.
  The semantics (lattice, projections, admission) live in `aa-core` as functions over them.
- *Reading custom options at runtime with prost-reflect* (safety-first). Rejected in favour of build-time codegen, so no
  reflection crate ships in the binary.
