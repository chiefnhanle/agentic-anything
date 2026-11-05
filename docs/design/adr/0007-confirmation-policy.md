# ADR 0007: Admission and confirmation policy

## Context
The reports disagree on what to do with unknown or unverified writes: "serve with confirmation" vs "withhold" (CRITIC.md).
REA's #545 showed what happens with an unissuable grant. REA's elicitation never ran against a real client.

## Decision
- **Admission is a matter of evidence, and runtime policy is a matter of authorization.** They are separate:
  | Level | Admission (PASSED lanes bound to current lock + identity) | Runtime default |
  |---|---|---|
  | UNKNOWN | never servable (`EFFECTS_UNKNOWN` + hint) | — |
  | READ | L0 + L1 (CLI) + L3, with lowered fields observed at the isolation they need | allow if the target is allow-listed |
  | UI | L0 + L3 + a11y oracle (M3) | ask per policy |
  | WRITE | READ set + **L4 read-back** (+ L2 if dry-run is supported) | allow within the policy's `write_roots`; idempotency key required |
  | DESTRUCTIVE | WRITE set (+ negative oracle for deletes) | **always ask**; grants can never satisfy it |
- Unknown or unverified writes are **withheld**, not "served with confirmation". A human approving an unobserved effect is
  the REA failure mode.
- **Policy file** `~/.config/agentic-anything/policy.toml` is operator-owned and refused if group- or other-writable. It is evaluated over structured
  descriptors (target, capability glob, level, tier, effect bits, canonical path scopes, session taint, assurance tier), never
  over command text. **The strictest matching rule wins regardless of order**, which a shuffled-order proptest checks. The default is deny. The digest is recorded in every
  decision.
- **Confirmations** are bound to `action_digest = sha256(JCS(ActionDigestInput))`, which covers the capability digest, lock digest, identity
  digest, generation, args commitment, policy digest and dry-run flag. They are single-use with a 120 s TTL. `Resume` re-runs halt, identity and
  policy checks. A changed digest yields `TARGET_STATE_CHANGED` and a fresh ask. The prompt is engine-authored, and the agent's stated reason and
  any dry-run preview are shown labelled.
- **Grant lifecycle as one unit** (M3): `aa grant`, `aa grants list`, `aa policy explain`, `aa grants revoke`. Grants are issued
  only over `operator.sock`. Each grant is session × capability-glob × `max_tier < DESTRUCTIVE` × TTL ≤ 1 h × max uses. A test asserts
  that every lifetime or reason has a production issuer and a CLI command that exists (REA #545/#552).
- **Session taint**: after target content is returned in a session, actions with `external_side_effect`,
  `executes_foreign_code`, `reads_credentials` or `writes_clipboard` escalate to ask, even under a grant.
- **Circuit breaker** (M3): a session is suspended after 3 consecutive or 20 total denials or declines.
- **Elicitation** is claimed only once a CI lane proves a real pinned MCP client round trip. Until then it is documented as fail-closed.

## Consequences
- "EASY" depends on evidence lanes being cheap (ADR 0011), not on prompting.
- Slice 1 ships the evaluator and allow/deny. Confirmations land in M3, and destructive capabilities return
  `CONFIRMATION_REQUIRED` (exit 3) with nothing dispatched until then.

## Alternatives
- *Ceilings auto-promoted to grants* (REA): inert and confusing.
- *Session grants as the default*: binds scope, not the action.
