# Handoff: continuing agentic-anything on another computer

Written 2026-11-07, at commit `ff2fb3b` plus this file. `PLAN.md` was last updated 2026-11-04 and is out of date.
Use this file and `docs/design/` to see where the project stands.

## 1. Where things stand

| Phase | State | Where |
|---|---|---|
| Research | done: 20+ reports and a completeness critic | `docs/research/` (start with `CRITIC.md`) |
| Design | done: final architecture, 13 ADRs, protos (buf lint clean) | `docs/design/architecture.md`, `docs/design/adr/`, `proto/` |
| **Slice 1** (M1, first half) | **done on macOS.** 7 Rust crates; `aad` engine + `aa` CLI over gRPC/UDS; `scripts/check.sh` green with 309/309 tests | `crates/`, `docs/design/build-plan.md`, `docs/design/slice1-acceptance.md` |
| Slice 1 leftover | CLI-Anything LibreOffice half of the demo (steps 11-12) never ran, because the harness venv was not installed | `docs/design/slice1-acceptance.md` |
| **Slice 2** | **not started**: this is the next piece of work | `docs/design/build-plan.md` §5, `docs/design/architecture.md` §15 |

What the project is: a framework for distilling the capabilities of any software you own or are authorized to automate
into a CLI and/or API (MCP) interface. It borrows its patterns from REA. The core is a Rust gRPC engine (`aad`). The
`aa` CLI, the MCP server and the skills are clients of that engine.

## 2. Set up the new machine

### 2.1 Clone this repo and the two forks

```sh
git clone https://github.com/chiefnhanle/agentic-anything.git
git clone https://github.com/chiefnhanle/rea.git            # REA fork (upstream morluto/rea): reference source for patterns
git clone https://github.com/chiefnhanle/CLI-Anything.git   # CLI-Anything fork (Apache-2.0): harnesses ingested as a binding source
```

The docs and some tests use the old machine's absolute paths (`/Users/nhanclie/rea`, `/Users/nhanclie/CLI-Anything`).
Replace them with your own clone paths. Nothing in the build depends on them. Only the opt-in LibreOffice e2e and the
demo commands use them.

### 2.2 Toolchain

- **Rust**: `rust-toolchain.toml` pins 1.99.1 with rustfmt and clippy. rustup installs it on the first `cargo` run.
  - On the old Mac, rustup came from Homebrew and was not on PATH:
    `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"`.
- **buf** (proto lint/format, used by `scripts/check.sh`): `brew install bufbuild/buf/buf`, or see buf.build.
- **cargo-nextest** (optional, check.sh falls back to `cargo test`): `cargo install cargo-nextest --locked`.
- **protoc is not needed**: `aa-proto/build.rs` handles codegen (see build-plan §3.0.2).
- For the e2e tests: `git` (always required), `gh` (the test runs when it is on PATH), `uv` and Python with click (the
  Click fixture), `jq` (the demo transcript).

### 2.3 Build and check

```sh
cd agentic-anything
cargo build --release                       # target/release/{aad,aa}
uv venv target/integ-venv
uv pip install --python target/integ-venv/bin/python click crates/aa-harvest/tests/fixtures/notes_cli
scripts/check.sh                            # buf lint/format, fmt, clippy -D warnings, tests: expect "all green"
```

`crates/README.md` covers running `aad` and `aa` by hand: a scratch `AA_HOME` and runtime dir, `examples/policy.toml`
installed with mode 0600, and the exit codes.

### 2.4 Platform caveat

Slice 1 was built and tested **only on macOS (arm64)**. Confinement uses Seatbelt (`sandbox-exec`). Other platforms
get `NoConfiner`, so `confinement_available=false`. READ capabilities require `ISOLATION_CONFINED`, which means no
capability becomes servable there, and the e2e tests that expect `git.status` to be servable will probably fail. The
Linux Landlock confiner is on the M4 roadmap. If the new computer runs Linux or WSL, you have two options: pull the
Landlock confiner forward, or continue on a Mac.

## 3. What to do next

1. **Optional: finish the slice-1 leftover.** Install the LibreOffice harness into a venv the operator owns, then run
   steps 11-12 and the opt-in `e2e_libreoffice` test. The exact commands are in `docs/design/slice1-acceptance.md`
   under "To finish steps 11-12". The engine never installs harnesses itself, by design (ADR 0009).
2. **Slice 2** (build-plan §5, architecture §15 M1):
   - L2 dry-run and L4 read-back/negative lanes in `aa-exec::lanes`. These make WRITE servable for `git.add`,
     `notes.add` and `cli-anything.libreoffice.writer.add-paragraph`.
   - Renderers in `aa-core::render`: mcp-typed, mcp-search, skill-md, cli-tree, plus the catalog digest.
     `CatalogService.GetExposure` and `DistillService.Package`.
   - `aa mcp` (an rmcp stdio server, behind the `mcp` feature of `aa-cli`) and `aa run` (a dynamic clap tree).
   - The `aa-core::proposal` ManifestProposal fence (hostile-proposal table, cargo-mutants floor), with
     `GetProposalContext` and `ApplyProposal`, and `skills/aa-distill`.
   - `compile --check` and `package --check` in CI. No CI exists yet.
   - Exit criterion: git, gh (read core), a Click harness and `cli-anything.libreoffice` each reach READ-servable in
     under 10 minutes; `writer.add-paragraph` is admitted by L4; SKILL.md and `tools/list` are set-equal with the lock.
3. Before you start, write a slice-2 build plan in the same style as `docs/design/build-plan.md` (work packages,
   definition of done, acceptance demo), then implement against it. Update `PLAN.md` once you have, so it no longer
   lags.
4. GitHub housekeeping from `PLAN.md`: child issues under epic #1 per milestone (M0-M4) have not been created yet.

## 4. Kickoff prompt for Claude Code on the new machine

Paste this in a Claude Code session opened at the repo root:

> Read `HANDOFF.md`, `crates/README.md`, `docs/design/build-plan.md` §1 and §5, and `docs/design/architecture.md`
> §0 and §15. The REA fork is at `<path>/rea` and the CLI-Anything fork is at `<path>/CLI-Anything`. Read them locally
> and do not re-clone. First make `scripts/check.sh` green on this machine. Then draft
> `docs/design/build-plan-slice2.md` for slice 2 and stop for my review before you implement.

Claude Code memory from the old machine does not carry over. The parts that matter are in this file: the project goal,
where the forks live, the Rust PATH note, and the fact that `PLAN.md` holds repo state.
