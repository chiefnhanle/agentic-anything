# crates

The slice-1 Rust workspace (design: `docs/design/architecture.md`, plan: `docs/design/build-plan.md`). Dependencies
point downward only: `aa-proto` ← `aa-core` ← (`aa-exec`, `aa-harvest`, `aa-store`) ← `aa-engine` ← `aa-cli` (tests).

| Crate | Binary | What it is |
|---|---|---|
| `aa-proto` | | Generated `agentic_anything.v1` messages (prost + pbjson), optional tonic services, the descriptor set and the generated `RPC_POLICY` table (role, halt and audit rules per RPC). |
| `aa-core` | | Pure semantics, with no I/O: ids, JCS digests, `AaError`, the effect lattice and lowering table, characterize rule tables, overlay parse, deterministic compile to `capabilities.lock.json`, admission, the `aa-policy/1` evaluator and PathScope, and the L0 static lane. |
| `aa-exec` | `torture` (test helper) | Process supervision (own process group, deadline then TERM/KILL, output caps, survivor cleanup), the macOS Seatbelt confiner, private roots and snapshots, target identity pinning, the `cli_argv` binding, and the L1 parser and L3 sandbox-diff lanes. |
| `aa-harvest` | | Harvesters `help-text@1`, `cobra-complete@1`, `click-info@1` (Python shim in `shims/`) and `cli-anything-registry@1`, plus the STATIC CLI-Anything import (which never runs `install_cmd`). |
| `aa-store` | | Workspace layout under `AA_HOME`, the sha256 CAS, the hash-chained JSONL audit log (boot verification, intent recovery) and halt persistence. |
| `aa-engine` | `aad` | The engine: two UDS listeners (`agent.sock`, `operator.sock`), the role layer, the gate (admission, schema, PathScope, identity, policy, intent before dispatch, ActionRecord), and the gRPC services. Out-of-slice RPCs return `UNIMPLEMENTED`. |
| `aa-cli` | `aa` | The client: `daemon`, `status`, `halt`, `unhalt`, `target add/list/show`, `import cli-anything`, `discover`, `verify`, `list`, `describe`, `invoke` (`x`), `audit`. It also holds the cross-crate e2e tests (`tests/e2e_*.rs`). |

## Build and check

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
cargo build --release                 # target/release/{aad,aa}
scripts/check.sh                      # buf lint/format, fmt, clippy -D warnings, nextest --workspace
```

`scripts/check.sh` runs the e2e tests against a real `aad`:
- `e2e_git` always runs.
- `e2e_gh` runs when `gh` is on PATH.
- `e2e_click` needs a Python venv with click and the notes fixture. check.sh picks up `target/integ-venv` or `.venv`
  automatically, or you can set `AA_TEST_PYTHON` yourself:

  ```sh
  uv venv target/integ-venv
  uv pip install --python target/integ-venv/bin/python click crates/aa-harvest/tests/fixtures/notes_cli
  ```

- `e2e_libreoffice` is opt-in: it needs `AA_TEST_CLI_ANYTHING=<CLI-Anything checkout>` and
  `AA_TEST_LO_PYTHON=<venv python with the harness installed>`. Without them it prints `SKIP`.

## Run aad and aa

`aad` refuses to run as root and needs a private runtime dir. Socket paths must fit in about 100 bytes. Keep the
policy file operator-owned and not group- or other-writable. The commands below use a scratch workspace, so nothing
under `~/.config` or `~/.local` is touched:

```sh
export AA_HOME=$PWD/target/demo/home AA_RUNTIME_DIR=$PWD/target/demo/rt PATH="$PWD/target/release:$PATH"
mkdir -p -m 0700 "$AA_HOME"
install -m 0600 examples/policy.toml "$AA_HOME/policy.toml"
aad --workspace "$AA_HOME" --runtime-dir "$AA_RUNTIME_DIR" --policy "$AA_HOME/policy.toml" --log info &
# or: aa daemon --workspace "$AA_HOME" --policy "$AA_HOME/policy.toml"

aa status
aa target add git --exec git --overlay targets/git/target.aa.yaml --authorization "owner workstation"
aa discover git && aa verify git && aa list git --servable
aa x git.status --input-json '{"repo":"/path/to/repo"}'
aa halt --reason test && aa unhalt --epoch "$(aa status --json | jq .halt.haltEpoch)"
aa audit --last 12 --verify
```

Defaults when no flags or env vars are given:
- workspace: `$AA_HOME`, else `~/.local/share/agentic-anything`;
- policy: `~/.config/agentic-anything/policy.toml`, else a built-in default-deny policy;
- runtime dir: `$AA_RUNTIME_DIR`, else `$TMPDIR/aa` on macOS (`/tmp/aa-<uid>` if the socket path would be too long).

Exit codes of `aa`:

| Code | Meaning |
|---|---|
| 0 | ok |
| 1 | failed |
| 2 | usage or schema error |
| 3 | needs a human or more authority (withheld, policy denied) |
| 4 | engine unreachable or halted |
| 130 / 143 | interrupted by a signal |
| 141 | EPIPE |

`unhalt` works only on `operator.sock`.

The slice-1 acceptance transcript is in `docs/design/slice1-acceptance.md`.
