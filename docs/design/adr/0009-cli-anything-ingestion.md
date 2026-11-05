# ADR 0009: CLI-Anything ingestion

## Context
HKUDS CLI-Anything (owner's fork at `/Users/nhanclie/CLI-Anything`, HEAD `34f5195`, Apache-2.0) ships 69 `*/agent-harness`
Click CLIs. The facts below were checked in this session:
- `registry.json` has `{meta, clis[79]}` with keys name, display_name, version, description, requires, homepage, source_url,
  install_cmd, entry_point, skill_md, category, contributors.
- In the LibreOffice harness, `libreoffice_cli.py` declares `@click.group(invoke_without_command=True)` at :117 and `def cli` at :124, and starts a REPL
  when invoked bare. A `result_callback` at :142 auto-saves the project. `def main()` at :831 wraps `cli()`, so the console script is not the group.
  `sys.path.insert` at :25 runs at import. The package has `__main__.py`.
- Errors are `{"error","type"}` JSON on stdout with exit 1. Global `--json/--project/--dry-run` must precede the subcommand.
  Generator-first observed this, and also 40 leaf commands via `to_info_dict()`.
- `install_cmd` is an unpinned `pip install git+https://…` (and `cli-hub` runs such strings with `shell=True`).
- CLI-Anything carries no effect or authority metadata. Its `validate` is an LLM prompt.

## Decision
- **Import is a STATIC step from a local checkout only.** `aa import cli-anything <repo> --harness <name> --python
  <venv-python> --authorization "…"`:
  1. Parse `registry.json` and the harness directory. Hash the harness tree into `source_tree_sha`. Record the repo `LICENSE` in `License`.
  2. Store `install_cmd` verbatim as a CAS artifact and **never execute it**. Flag shell metacharacters.
  3. Store the harness `SKILL.md` as an untrusted reference (`TargetText`, a description fallback only). It is never served.
  4. The interpreter is **operator-installed** (`uv venv` + `uv pip install <local harness dir>`). Engine-managed installation
     runs build-backend code, so in M2 it becomes a gated pipeline action needing an operator confirmation (`aa.pipeline.install`,
     DESTRUCTIVE: `executes_foreign_code`, `writes_filesystem`).
- **Introspection** (`click-info@1`) is an INTROSPECT pipeline action. It is audited with intent-before-dispatch and refused unless the target has an
  authorization statement. It runs `<python> -I aa_click_info.py <entry_point>` one-shot in a private root under confinement
  (network denied), with a scrubbed env, a 20 s deadline and a 4 MiB stdout cap. Root selection is **the unique `click.Group` in the entry module that
  is not a child of another group**. Anything else is an `ambiguous_root` error. The raw JSON is stored verbatim, and `candidates()` is
  pure over those bytes, so CI replays them without Python.
- **Mapping:**
  - Click types become JSON Schema: String, Int/IntRange with bounds, Float, flag→boolean, Choice→enum, Path/File→`format: path` + scope,
    multiple→array, nargs=-1→positional array.
  - `--json` goes into `global_args` and is never exposed.
  - `--project` becomes a **state handle**, engine-supplied from the session's 0700 workspace by default. The overlay may expose it
    as a path param, which is then always PathScope-checked.
  - `--dry-run` becomes `DryRun{NO_PERSIST}`.
  - Errors use `ERROR_CONVENTION_CLI_ANYTHING_JSON`.
  - `repl` and the bare group are `INTERACTIVE` and never emitted as servable.
  - **Autosave rule:** any leaf that raises `mutates_target` and accepts the state handle also raises `writes_filesystem` (scope STATE_HANDLE).
- **Backend identity.** `requires` becomes a `binary:<x>` precondition, resolved with which + realpath + sha256 + version banner and pinned in
  `TargetIdentity.files` as `backend:<x>`. A missing or changed backend gives `PRECONDITION_UNMET` or `TARGET_IDENTITY_CHANGED`
  instead of a silent fallback (GIMP→Pillow). Exec allow-lists under confinement contain only the pinned interpreter and
  backends, so an unpinned fallback exec is denied and recorded.
- `public_registry.json` and `matrix_registry.json` are target **suggestions** only.

## Consequences
- 79 registry entries become importable targets with no per-harness code. Quality varies, so we verify behaviour, not
  doctrine. The non-claim "L4 does not establish that the real backend rendered this" stays unless a backend precondition binds it.
- Any Click CLI, not just CLI-Anything, works through the same `click-info` harvester (`aa target add --python-script`).

## Alternatives
- *Running `install_cmd` or `cli-hub`*: remote shell strings and unpinned installs.
- *A static AST parse of harness sources*: a fallback only, because it misses dynamic commands.
- *Grepping harness sources for effect floors* (safety-first): it puts `launches_process` on every command and withholds everything. Rejected in favour of
  observation under confinement.
