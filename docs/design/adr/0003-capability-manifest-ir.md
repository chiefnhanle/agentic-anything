# ADR 0003: Capability manifest IR (overlay + lock)

## Context
The reports proposed five IR shapes (CRITIC.md). The pilot (`research/pilot-pipeline-on-real-targets.md`) found that 21 of 38
candidate fields were used. It also found four essentials that were missing or weak in every shape: per-bit tri-state effects with provenance, effect
branches, oracles with a kind, and preconditions as data. Generator-first's overlay+lock split was the judges' top graft for
making distillation easy.

## Decision
- **Lock** = `CapabilityLock` (`manifest.proto`), stored as `targets/<id>/capabilities.lock.json`. It is canonical proto3-JSON, committed, and
  carries `lock_digest = sha256(JCS(lock with lock_digest=""))`. `compile(harvest artifacts, overlay, rule_versions)` is pure and
  deterministic, and `aa compile --check` fails CI on drift.
- **Overlay** = `targets/<id>/target.aa.yaml`. It is small and human-written, and holds only what a machine cannot decide: exclusions,
  state-handle convention, raises, oracle pairings, fixtures, examples, the core set, and `scope` exposure. Effect assumptions are
  **not** overlay keys an agent can write. They are recorded through `TargetService.AssumeEffects` on `operator.sock`, one
  audited AdminEvent per field, and mirrored into an `assumed:` block that compile reads only if each entry's audit seq verifies.
- **Capability** fields follow the pilot's minimal IR plus derived projections: id `<target>.<noun-path>.<verb>`,
  closed input schema, effects + signals + branches, derived level/tier/hints, scopes, binding, oracles (`read_back | file
  | negative`), dry run, lifetime, preconditions, provenance, admission, projections, examples, and `capability_digest`.
- Every target-originated string is `TargetText` (from mvp-first), so a front end cannot drop the untrusted marker.
- **LLM fence**: `DistillService.ApplyProposal(ManifestProposal)` is the only entry for LLM output. Ops may rename, describe,
  set family, set the core set, raise an effect, add a branch (raise-only), pair an oracle (still unverified), exclude, narrow the schema, or add an example.
  Hard rejects: stale `base_lock_digest`, a capability id outside the harvested set, a lowering op, any widening patch, any
  admission, grant or assumption key. A table of hostile proposals runs in CI, and cargo-mutants has a blocking score floor on the fence.
- Admission and operator reviews carry over across recompiles only while `capability_digest` is unchanged.

## Consequences
- Every renderer and policy check reads one artifact, and a git diff of the lock is the review surface.
- JSON Schema strings sit inside proto messages, so there are two schema languages. JSON Schema is validated with
  `jsonschema` 0.60.2, and schema narrowing is checked only over a supported subset. Anything else is rejected.

## Alternatives
- *A single hand-edited manifest* (mvp-first/safety-first): there is no line between machine output and human intent.
- *usage-spec KDL as the source format*: it lacks effects vectors, oracles and bindings. It may be exported later.
