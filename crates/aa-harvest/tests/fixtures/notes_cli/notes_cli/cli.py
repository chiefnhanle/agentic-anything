"""notes: a hermetic Click CLI shaped like a CLI-Anything harness.

Shape (what the harvesters and lanes are tested against):
- a root group with ``invoke_without_command=True`` and globals ``--json/--project/--dry-run`` that precede the
  subcommand; bare invocation starts a REPL;
- a ``main()`` wrapper, so the console script is not the group (root selection must find the group);
- ``{"error", "type"}`` JSON errors on stdout with exit 1 in ``--json`` mode;
- an auto-save ``result_callback`` (the CLI-Anything autosave rule);
- ``notes stats`` claims to be a read but writes ``~/.cache/notes/stats`` (L3 must flag SIDE_EFFECT);
- ``notes lenient`` accepts unknown options (L1 must flag PARSER_MISMATCH).
"""

import json
import os
import sys

import click


class State:
    def __init__(self, project_path, as_json, dry_run):
        self.project_path = project_path
        self.as_json = as_json
        self.dry_run = dry_run
        self.modified = False
        self.notes = []
        if project_path and os.path.exists(project_path):
            with open(project_path, encoding="utf-8") as fh:
                self.notes = json.load(fh).get("notes", [])

    def save(self):
        if self.project_path and self.modified and not self.dry_run:
            with open(self.project_path, "w", encoding="utf-8") as fh:
                json.dump({"notes": self.notes}, fh, indent=2)


def emit(state, data, text):
    click.echo(json.dumps(data, sort_keys=True) if state.as_json else text)


@click.group(invoke_without_command=True)
@click.option("--json", "as_json", is_flag=True, help="Output as JSON")
@click.option("--project", "project_path", type=str, default=None, help="Path to the notes project file")
@click.option("--dry-run", "dry_run", is_flag=True, help="Run without saving changes to disk")
@click.pass_context
def cli(ctx, as_json, project_path, dry_run):
    """Notes CLI -- stateful note keeping. Run without a subcommand for a REPL."""
    ctx.obj = State(project_path, as_json, dry_run)
    if ctx.invoked_subcommand is None:
        ctx.invoke(repl)


@cli.result_callback()
@click.pass_obj
def auto_save(state, result, **kwargs):
    """Auto-save the project after one-shot commands that modified it."""
    state.save()


@cli.group()
def notes():
    """Note management commands."""


@notes.command("list")
@click.option("--limit", type=click.IntRange(1, 1000), default=None, help="Maximum notes to show")
@click.option("--tag", type=str, default=None, help="Only notes with this tag")
@click.pass_obj
def list_notes(state, limit, tag):
    """List notes."""
    items = [n for n in state.notes if tag is None or tag in n.get("tags", [])]
    items = items[:limit] if limit else items
    emit(state, {"notes": items, "count": len(items)}, "\n".join(n["text"] for n in items))


@notes.command("add")
@click.option("--text", required=True, help="Note text")
@click.option("--tag", "tags", multiple=True, help="Tag (repeatable)")
@click.pass_obj
def add_note(state, text, tags):
    """Add a note."""
    state.notes.append({"text": text, "tags": list(tags)})
    state.modified = True
    emit(state, {"added": len(state.notes) - 1, "text": text}, f"added #{len(state.notes) - 1}")


@notes.command("remove")
@click.argument("index", type=int)
@click.pass_obj
def remove_note(state, index):
    """Remove a note by index."""
    if index < 0 or index >= len(state.notes):
        raise IndexError(f"Index {index} out of range (0-{max(len(state.notes) - 1, 0)})")
    removed = state.notes.pop(index)
    state.modified = True
    emit(state, {"removed": index, "text": removed["text"]}, f"removed #{index}")


@notes.command("stats")
@click.pass_obj
def stats(state):
    """Show note statistics (read-only)."""
    data = {"count": len(state.notes), "tags": sorted({t for n in state.notes for t in n.get("tags", [])})}
    # The lie: a "read" that writes outside the project (L3 must observe it).
    cache = os.path.join(os.path.expanduser("~"), ".cache", "notes")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, "stats"), "w", encoding="utf-8") as fh:
        json.dump(data, fh)
    emit(state, data, f"{data['count']} notes")


@notes.command("lenient", context_settings={"ignore_unknown_options": True, "allow_extra_args": True})
@click.argument("words", nargs=-1)
@click.pass_obj
def lenient(state, words):
    """Echo words; silently accepts unknown options."""
    emit(state, {"words": list(words)}, " ".join(words))


@cli.command("export")
@click.option("--output", "output_path", type=click.Path(dir_okay=False, writable=True), required=True,
              help="Destination file")
@click.option("--format", "fmt", type=click.Choice(["txt", "json"]), default="txt", help="Output format")
@click.pass_obj
def export(state, output_path, fmt):
    """Export notes to a file."""
    with open(output_path, "w", encoding="utf-8") as fh:
        if fmt == "json":
            json.dump({"notes": state.notes}, fh)
        else:
            fh.write("".join(n["text"] + "\n" for n in state.notes))
    emit(state, {"output": output_path, "format": fmt, "count": len(state.notes)}, output_path)


@cli.command("repl")
@click.pass_obj
def repl(state):
    """Interactive REPL."""
    for line in sys.stdin:
        if line.strip() in ("quit", "exit"):
            break
        click.echo(f"notes> {line.strip()}")


def main(argv=None):
    """Console-script wrapper (CLI-Anything style): the entry point is not the group."""
    as_json = "--json" in (argv if argv is not None else sys.argv[1:])
    try:
        cli.main(args=argv, prog_name="notes", standalone_mode=False)
    except click.exceptions.Exit as exc:
        sys.exit(exc.exit_code)
    except click.ClickException as exc:
        exc.show()
        sys.exit(exc.exit_code)
    except click.exceptions.Abort:
        sys.exit(1)
    except (ValueError, IndexError, RuntimeError, OSError) as exc:
        if as_json:
            click.echo(json.dumps({"error": str(exc), "type": type(exc).__name__}))
        else:
            click.echo(f"Error: {exc}", err=True)
        sys.exit(1)


if __name__ == "__main__":
    main()
