# ADR 0011: Verification ladder and confinement

## Context
For "EASY", reads must be admitted by *observed evidence*, not by human assertion (judges, against mvp-first). All three
judges flagged that an L3 sandbox diff at `ISOLATION_PROCESS` on macOS cannot see writes outside the private root, network
traffic, or child processes. Lowering `writes_filesystem` there is a false-READ risk.

Verified on this Mac (2026-11-05, `/usr/bin/sandbox-exec`): a Seatbelt profile with `(deny network-outbound …)`,
`(deny file-write* (require-not (subpath <root>)))` and an exec allow-list does deny curl, outside writes and unlisted execs, while
`git --no-optional-locks status` runs normally. The pilot also found sandbox-exec plus a scratch HOME workable for CLIs.

## Decision
- **Lanes as code.** L0 static, L1 parser, L2 dry-run, L3 sandbox-diff, L4 read-back, L5 golden, L6 differential, L7 property, L8 agent.
  Only `PASSED` counts. `SKIPPED` never counts, `--require-tools` turns a missing tool into FAILED, and every record carries
  `non_claims`.
- **Lowering rules** (UNKNOWN→FALSE) by field:
  | Field | May be lowered by OBSERVED only at |
  |---|---|
  | `mutates_target`, `writes_filesystem` (within cwd/home/state handle) | ≥ PROCESS, by tree diff of the private root + state-handle hash |
  | `writes_filesystem` (anywhere) | ≥ CONFINED (writes outside root/scopes are denied, so a passing run proves none were needed) |
  | `accesses_network` | ≥ CONFINED (network denied) |
  | `launches_process` | ≥ CONFINED (exec allow-list = pinned binding executables) |
  | UI / screen / clipboard / credentials fields | never by L3. Needs OPERATOR or DECLARED-trusted, or an adapter oracle (M3) |
  - TRUE→FALSE is never done by a lane ("absence of evidence"). It needs OPERATOR on `operator.sock` with a reason.
  - A capability with a path-typed or output-like parameter (`format: path` with WRITE scope, `-o/--output`) is **never** lowered on
    `writes_filesystem`. It is WRITE with a `WRITE_PATH` scope.
- **Served as verified.** A capability admitted with confined-lowered fields records `required_runtime_isolation = CONFINED`
  and **runs under the same confinement profile at runtime**, generated per capability: network denied, exec allow-list,
  writes limited to the private root plus canonicalized write scopes. If confinement is unavailable at runtime, the capability is
  `PRECONDITION_UNMET: confinement`. A wrong READ admission therefore still cannot write outside its scope or reach the network.
- L3 runs at least 2 distinct examples. Fixture HOME and cwd changes count as `writes_filesystem` TRUE, and `tmp/` is ignored. Observed effects outside
  the declared set produce `SIDE_EFFECT` and withhold the capability. Process-group survivors are reported.
- L4 read-back: the oracle predicates must **fail on the seed and pass after the write**. Deletes also need a negative oracle with a
  bogus-id control.
- On Linux, confinement is Landlock (M4). Until then Linux lanes run at `ISOLATION_PROCESS`, and those fields need operator assumptions.

## Consequences
- macOS gets real enforcement for CLI targets now. `sandbox-exec` is deprecated by Apple, which is a recorded risk. The fallback is
  `ISOLATION_PROCESS` with per-field operator assumptions, and the profile generator is isolated behind a `Confiner` trait.
- GUI and Electron apps cannot be wrapped in Seatbelt (nested-sandbox failure, as the pilot found). Their lanes need VMs or users (CRITIC P4, open).

## Alternatives
- *Source-grep floors* (safety-first): withholds everything.
- *Operator assertion per capability* (mvp-first): no evidence, and too much manual work.
- *Lowering at PROCESS isolation with a non-claim* (generator-first): a false-READ risk.
