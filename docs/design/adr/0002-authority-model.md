# ADR 0002: Authority model

## Context
PLAN.md and the reports use four authority vocabularies that were never reconciled (CRITIC.md). REA removed its
grant/ceiling/approval system in PR #555. The reasons were inert settings, ceiling-vs-grant confusion (#545/#552), approvals the agent
could type itself (`*_approved: z.literal(true)`), and scope dimensions that were declared but never enforced
(`research/rea-removed-permission-system-archaeology.md`). A *drive* plane still needs authority, because it acts on live apps
that render untrusted content. The judges also found two concrete holes: generator-first trusted a client-asserted approval `channel`, and
safety-first let any `agent.sock` client self-approve through an elicitation relay.

## Decision
1. **Effects** are REA's ToolEffects made tri-state and extended to 18 fields (`manifest.proto` `Effects`). They are split into
   target-facing fields, `mutates_session` (control plane), and `idempotent` (safe direction). Missing means `TRI_UNKNOWN`, which policy treats as
   TRUE. Signals combine by max. A FALSE counts only from admissible sources (ADR 0011). Flag-conditioned
   `EffectBranch`es raise effects when they match.
2. **Binding-intrinsic execution is not an effect.** This covers the identity-pinned executable or interpreter of a `cli_argv`
   binding running in an owned group (`KILL_SCOPE_OWNED_GROUP`), and nothing else (REA `toolEffects.ts:12-18`).
3. **Derived projections**, never authored:
   - `EffectLevel`: `UNKNOWN < READ < UI < WRITE < DESTRUCTIVE`.
   - `AuthorityTier`: `OBSERVE < LAUNCH < DRIVE < WRITE < DESTRUCTIVE < ATTACH < INJECT < MUTATE_SYSTEM`. This reconciles PLAN.md's
     observe/launch/attach/inject/mutate: mutate is split into WRITE and DESTRUCTIVE, and attach/inject sit above both, as safety-rails §8 recommends.
   - MCP hints, computed by REA's formula over target-facing fields.
4. **Fail-safe zero values** on every safety enum: `TRI_UNKNOWN`, `ADMISSION_STATE_WITHHELD`, `AUTHORITY_TIER_UNKNOWN` (evaluated as
   highest), `EFFECT_LEVEL_UNKNOWN`, `VERDICT_DENY`, `ROLE_NONE`, `CONTENT_ORIGIN_TARGET`, `HALT_SCOPE_ENGINE` (widest),
   `ISOLATION_NOT_PROVEN`, `CONFIDENCE_PROPOSED` (weakest), `OUTCOME_UNKNOWN`, `LANE_STATUS_NOT_RUN`,
   `APPROVAL_SOURCE_NONE`, `ASSURANCE_TIER_SAME_PRINCIPAL`, `REQUEST_STATE_UNKNOWN_ID`. buf's `ENUM_ZERO_VALUE_SUFFIX` is
   disabled for this. A proptest in `aa-core` asserts that every zero evaluates to the most restrictive behaviour, which
   stops anyone "fixing" the lint by renaming.
5. **One gate** in `aa-engine::gate`, a required dependency of every dispatch, with a fixed order (architecture §5.3). The engine's own
   pipeline steps (INTROSPECT harvest, lane runs) pass through it as `aa.pipeline.*` actions.
6. **Approval authority comes from the listener, never from a request field.** Only `operator.sock` can call
   `DecideConfirmation`, `IssueGrant`, `Unhalt` or `AssumeEffects`. A role extension set per listener enforces this, checked
   against the generated `RPC_POLICY` table. MCP elicitation is *host-mediated*. `RelayElicitation` is accepted only when the
   peer executable is verified as the pinned `aa` binary *and* policy lists `HOST_ELICITATION` for that level (default
   WRITE only, never DESTRUCTIVE, and off in Tier ≥ 1).
7. **Honest assurance tiers** are computed from proofs and recorded in every decision (Tier 0 same principal, Tier 1
   separated agent uid, Tier 2 isolated target). In Tier 0 approvals are guardrails against model error and injection
   steering the normal tool path. They are not a boundary against a same-uid agent that has a shell.
8. **Hard exclusions** (licence, activation, DRM, anti-cheat, security-product surfaces) are compiled in, evaluated first, and
   cannot be re-enabled by overlay or policy.

## Consequences
- There are no permission booleans in any input schema. An agent cannot approve by typing.
- Each denial carries a typed `DenialReason`, and each reason maps to exactly one existing remediation command, enforced by a test.
- Observe stays cheap. Reads of caller-selected files are not gated beyond path scope (REA's lesson).

## Alternatives
- *REA's current stance* (no server-side approvals; the MCP client's UI suffices). Rejected for drive and write: headless clients have
  no UI.
- *A client-asserted approval channel on one socket* (generator-first). Rejected: it is self-approval through the API.
- *A three-socket design with a relay socket*. Rejected: a same-uid agent can reach it anyway. Peer-exe verification plus policy
  is the honest mechanism.
