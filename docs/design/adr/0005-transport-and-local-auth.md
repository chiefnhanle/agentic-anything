# ADR 0005: Transport and local authentication

## Context
The engine is local and single-user by default. tonic 0.14.9 exposes `UdsConnectInfo.peer_cred`
(`transport/server/unix.rs:37-42`, verified) and parses `unix:` endpoint URIs natively. The judges found the single
undifferentiated socket in generator-first fatal for approvals. They also found hyper-util/tower connectors unnecessary.

## Decision
- **Two Unix sockets** in a runtime dir created 0700 and checked with `lstat`. The engine refuses to start if the dir is a symlink, has another
  owner, or has group/other bits set:
  - macOS: `$TMPDIR/aa/`, falling back to `/tmp/aa-<uid>/` when the socket path would exceed about 100 bytes (`sun_path` is 104).
  - Linux: `$XDG_RUNTIME_DIR/aa/`.
  - `agent.sock` serves `ROLE_AGENT` and `operator.sock` serves `ROLE_OPERATOR`. Both are 0600 after bind. In Tier 1, `agent.sock` is 0660 with group `aa-agents`.
- Two `serve_with_incoming(UnixListenerStream)` futures. Each router is wrapped in a per-listener layer that inserts
  `Role` into request extensions. A shared interceptor then checks that `peer_cred.uid` is allowed for the listener and that the method's
  `RPC_POLICY` row admits the role and halt state. On failure it returns `PERMISSION_DENIED`/`UNAVAILABLE` with an `Error` in `details`.
- **Single instance:** `flock(engine.lock)` first, then unlink stale sockets, then bind. The engine refuses to run as euid 0.
- **No bearer token on UDS.** A same-uid token file adds nothing (REA SECURITY.md: "not a sandbox ... same operating-system
  user"). **Loopback TCP** (M4) is opt-in and agent-role only. It requires a 32-byte token from a 0600 file compared in constant
  time, a loopback-literal `:authority` check, and a refusal to bind a non-loopback address. It never serves the operator role.
- **Clients** call `Endpoint::from_shared(format!("unix://{}", sock.display()))?.connect().await`. tonic 0.14.9
  parses `unix:` URIs natively (`transport/channel/endpoint.rs:186-215`, verified), so neither hyper-util nor tower is needed. Tests use an in-process `tokio::io::duplex` transport.
- The peer pid → exe realpath → sha256 lookup is recorded in `Principal`. It is *required* only to accept
  `RelayElicitation` (ADR 0002).

## Consequences
- An agent that has only `agent.sock` cannot approve, unhalt or assume. With a same-uid shell it can reach `operator.sock`, so
  Tier 0 is labelled a guardrail. Tier 1 (separate agent uid) makes the split a real boundary, and `aa doctor` explains how to set it up (M4).

## Alternatives
- *One socket with a request-asserted channel*: forgeable.
- *Tokens as the boundary*: readable by the same uid.
- *Named pipes on Windows*: M4.
