#!/usr/bin/env bash
# Slice "done" gate (build-plan §1, §3.0): buf lint + format, rustfmt, clippy -D warnings, tests.
set -euo pipefail

export PATH="/opt/homebrew/opt/rustup/bin:${HOME}/.cargo/bin:${PATH}"
cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n==> %s\n' "$*"; }

step "buf lint"
buf lint

step "buf format -d --exit-code proto"
buf format -d --exit-code proto

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

# The Click e2e test (crates/aa-cli/tests/e2e_click.rs) needs a venv python with click and the notes_cli fixture;
# without one it prints a SKIP. Pick up a conventional local venv when the caller did not set one:
#   uv venv target/integ-venv && uv pip install --python target/integ-venv/bin/python click \
#     crates/aa-harvest/tests/fixtures/notes_cli
if [[ -z "${AA_TEST_PYTHON:-}" ]]; then
    for py in target/integ-venv/bin/python .venv/bin/python; do
        if [[ -x "$py" ]]; then
            export AA_TEST_PYTHON="$PWD/$py"
            break
        fi
    done
fi
printf '\nAA_TEST_PYTHON=%s\n' "${AA_TEST_PYTHON:-<unset: e2e_click will SKIP>}"

if cargo nextest --version >/dev/null 2>&1; then
    step "cargo nextest run --workspace"
    cargo nextest run --workspace
else
    step "cargo test --workspace (cargo-nextest not found)"
    cargo test --workspace
fi

printf '\ncheck.sh: all green\n'
