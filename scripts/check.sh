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

if cargo nextest --version >/dev/null 2>&1; then
    step "cargo nextest run --workspace"
    cargo nextest run --workspace
else
    step "cargo test --workspace (cargo-nextest not found)"
    cargo test --workspace
fi

printf '\ncheck.sh: all green\n'
