# ADR 0006: ActionRecord and audit log

## Context
REA's Evidence model requires every result to be a record. A drive plane also needs crash semantics: a killed write may or
may not have happened. The task asks for JSONL hash-chained audit. Safety-first proposed JSONL as the source of truth with a
rebuildable SQLite index. Generator-first proposed SQLite only.

## Decision
- **Source of truth:** append-only JSONL segments `audit/segment-<n>.jsonl`. Each line is one `AuditEntry` (proto3-JSON) with
  `seq`, `prev_sha256` and `entry_sha256 = sha256(JCS(entry with entry_sha256=""))`. The head hash is mirrored atomically
  to `audit/HEAD` so truncation is visible. A single writer task, `aa-store::audit`, holds `flock` on the segment.
- **Exactly one ActionRecord per Invoke attempt after the capability resolves**, including `REJECTED_INPUT`, `DENY`,
  `WITHHELD` and a parked confirmation (all `OUTCOME_NOT_DISPATCHED`). Before resolution (unknown id, malformed envelope) the response is a
  tonic Status and nothing is recorded.
- **Intent before dispatch:** for level ≥ WRITE and for every INTROSPECT pipeline action, an `ActionIntent` is written and
  **fsynced**, and only then does dispatch happen. If the write fails, the result is `AUDIT_UNAVAILABLE` and nothing runs. At boot, an intent without an outcome
  is closed by a synthesized `ActionRecord{outcome: UNKNOWN, error: EXECUTION_STATE_UNKNOWN, recovered: true}`.
- **Boot verification:** the engine verifies the chain tail (by default the last segment fully, plus the HEAD match) at start. On failure it
  **boots halted** with `HALT_SOURCE_AUDIT_VERIFY_FAILED`, and only the operator can unhalt.
- **Privacy:** inputs are stored as `args_commitment = HMAC-SHA256(engine key, JCS(input))` plus `input_redacted_json` (params
  marked `x-aa-secret` are blanked). Full outputs go to the CAS by digest. stderr excerpts are bounded and redacted.
- **Admin events** are audited: halts, target registration (with the authorization statement), each assumed effect field, admission
  changes, decided confirmations, grants and policy loads.
- **SQLite index** (M3): `rusqlite` 0.41.1 with `bundled`. It is rebuildable from the JSONL and has `BEFORE UPDATE/DELETE RAISE(ABORT)`
  triggers. It is a query accelerator only.

## Consequences
- The log is tamper-*evident*, not tamper-proof, because the same uid can rewrite the files. Signed checkpoints (ed25519 anchors) are deferred.
- JSONL is greppable and survives SQLite corruption. Queries in slice 1 are linear scans with an in-memory index by action id.

## Alternatives
- *SQLite as the source of truth* (generator-first): harder to inspect, and corruption loses history.
- *An outcome-only log*: crash semantics are lost.
