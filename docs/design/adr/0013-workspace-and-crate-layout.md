# ADR 0013: Workspace and crate layout

## Context
The proposals had 4 crates (mvp-first), 17 (generator-first) and 18 (safety-first). The judges asked for about 5 crates in the slice, with a split
only when a dependency-direction lint or build time requires it, and a transport-free core.

## Decision
Slice-1 workspace, 7 crates:
| Crate | Role | Depends on |
|---|---|---|
| `aa-proto` | generated messages (+ `grpc` feature: tonic services), descriptor set, `RPC_POLICY` table | prost, pbjson |
| `aa-core` | **pure**: ids, JCS digests, errors, effects lattice, projections, branches, admission, overlay/lock compile, characterize rule tables, policy evaluator, L0 lane, renderers (slice 2), proposal fence (slice 2), and the shared traits (`ProcessRunner`, `Binding`, `Harvester`, `AuditSink`) | aa-proto (no grpc), serde, jsonschema |
| `aa-exec` | supervision kernel, `Confiner`, `cli_argv` binding, L1/L3 lanes (L4 in slice 2) | aa-core, tokio, process-wrap |
| `aa-harvest` | harvesters: cli-anything-registry, click-info (one-shot Python shim), cobra-complete, help-text | aa-core |
| `aa-store` | workspace layout, CAS, JSONL hash-chained audit, intent recovery, halt persistence | aa-core |
| `aa-engine` | `aad`: tonic services, two listeners, role layer, gate, composition root | all above, aa-proto[grpc] |
| `aa-cli` | `aa` binary (client over UDS), `mcp` feature (slice 2), e2e tests | aa-proto[grpc], tokio, clap |

Rules:
- `aa-core` has no tokio, tonic or I/O beyond `std::fs` reads in the overlay/lock loaders. A `cargo metadata` test enforces the
  dependency direction and a ban list.
- `aa-cli` depends on `aa-engine` only as a dev-dependency (e2e). The CLI never links engine internals.
- `aa-adapter-host` (M2) and `aa-mcp` split out only when a second consumer appears.

## Consequences
- Seven parallel work packages with exclusive crate ownership (build-plan.md).
