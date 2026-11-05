# ADR 0012: Process supervision kernel

## Context
REA's `src/process` ownership kernel is about 5.7k non-test lines, and about 15.5k including tests for the main files. The judges found a full port
unbuildable at slice pace. The needed invariants are owned groups, deadlines, bounded output, private roots, env scrub, poison on
timeout, and never signalling what you do not own.

## Decision
`aa-exec` implements a focused kernel. It uses `tokio::process`, `process-wrap` 10.0.1 for process groups, `rustix`, and `tempfile`:
- spawn in a new process group, `stdin=/dev/null`, `env_clear()` plus allow-list (`PATH`, `LANG`, `LC_*`, binding env), `HOME`/`TMPDIR`/cwd in a
  private 0700 root, `AA_RUN_ID` set last as an ownership marker (not a credential); never a shell;
- an absolute deadline, then SIGTERM, a 250 ms grace, then SIGKILL of the **owned group only**. A timed-out WRITE+ yields `OUTCOME_UNKNOWN` /
  `EXECUTION_STATE_UNKNOWN` (poison);
- stdout capped at `max_output_bytes` (overflow is drained, reported and never retained) and a head+tail stderr excerpt with token redaction;
- a survivor check of the process group after exit, giving `CLEANUP_INCOMPLETE` and cleanup debt that blocks new dispatch to the target;
- a drop guard that kills the group when the RPC future is dropped (client cancel);
- an optional `Confiner` (macOS Seatbelt profile generator; Linux Landlock in M4) wraps the argv;
- identity re-check before dispatch: a stat tuple cache with a rehash on change.

LaunchServices-spawned processes (for example `open -a`) are outside the group. They are recorded in `unowned_process_notes` and never signalled.
PID-reuse-proof ownership verification of foreign processes (REA `ProcessOwnership*`) is deferred to the attach adapters in M2/M3.

## Consequences
- The kernel is about 1-1.5k lines with torture tests: ignores SIGTERM, orphaned grandchild, stdout flood, sleeping past the deadline, out-of-root
  write under confinement (denied and reported).

## Alternatives
- *A full REA port* (safety-first): too large for the slice, and most of it serves attach and foreign processes.
