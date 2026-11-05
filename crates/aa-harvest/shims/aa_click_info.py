"""aa-click-info/1: one-shot Click self-description (agentic-anything, build-plan §3.3, ADR 0009).

Run as:  <python> -I <private-root>/aa_click_info.py <entry_point>

- stdlib + click only;
- resolves the console-script entry point by name via importlib.metadata and loads it (this imports the
  harness, which is why the engine runs it confined: network denied, writes only in the private root);
- if the loaded object is not a click.Command, the root is the unique click.Group in the entry module that
  is not a child of another group (CLI-Anything `main()` wrappers); otherwise {"error":"ambiguous_root"}, exit 65;
- prints {"protocol","entry_point","distribution":{name,version},"click_version","root": root.to_info_dict(ctx)}
  (plus "entry_value" and "main_module" so the binding can run `-I -m <main_module>`);
- never invokes the group or any command callback.

Exit codes: 0 ok, 64 usage, 65 ambiguous_root, 66 entry point not found, 69 click missing, 70 internal error.
"""

import contextlib
import enum
import importlib.metadata
import importlib.util
import io
import json
import sys

PROTOCOL = "aa-click-info/1"

sys.dont_write_bytecode = True  # never write __pycache__ next to the harness


def fail(error, code, **extra):
    payload = {"protocol": PROTOCOL, "error": error}
    payload.update(extra)
    sys.stdout.write(json.dumps(payload, sort_keys=True) + "\n")
    sys.stdout.flush()
    sys.exit(code)


def sanitize(value):
    """JSON-safe, deterministic: no reprs with addresses, no callables."""
    if value is None or isinstance(value, (bool, int, str)):
        return value
    if isinstance(value, float):
        return value if value == value and value not in (float("inf"), float("-inf")) else None
    if isinstance(value, dict):
        return {str(k): sanitize(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, set, frozenset)):
        items = [sanitize(v) for v in value]
        return sorted(items, key=lambda v: json.dumps(v, sort_keys=True)) if isinstance(value, (set, frozenset)) else items
    if isinstance(value, enum.Enum):
        return sanitize(value.value) if isinstance(value.value, (str, int, float, bool)) else value.name
    if callable(value):
        return {"$callable": True}
    if hasattr(value, "__fspath__"):
        return str(value)
    return {"$unrepresentable": type(value).__name__}


def find_entry_point(name):
    for ep in importlib.metadata.entry_points(group="console_scripts"):
        if ep.name == name:
            return ep
    return None


def main_module_for(module_name):
    """Most specific package (module itself, then parents) that has a __main__ submodule."""
    parts = module_name.split(".")
    for i in range(len(parts), 0, -1):
        candidate = ".".join(parts[:i])
        try:
            if importlib.util.find_spec(candidate + ".__main__") is not None:
                return candidate
        except (ImportError, ValueError, AttributeError):
            continue
    return None


def select_root(click, obj, module):
    if isinstance(obj, click.Command):
        return obj, "entry_point"
    seen = {}
    for value in vars(module).values():
        if isinstance(value, click.Group):
            seen[id(value)] = value
    groups = list(seen.values())
    children = {id(c) for g in groups for c in g.commands.values()}
    roots = [g for g in groups if id(g) not in children]
    if len(roots) == 1:
        return roots[0], "unique_root_group"
    return None, "groups=%d roots=%d" % (len(groups), len(roots))


def main(argv):
    if len(argv) != 2 or not argv[1]:
        fail("usage", 64, detail="aa_click_info.py <entry_point>")
    name = argv[1]
    try:
        import click
    except ImportError:
        fail("click_missing", 69)
    ep = find_entry_point(name)
    if ep is None:
        fail("entry_point_not_found", 66, entry_point=name)
    captured = io.StringIO()
    try:
        # Harness modules may print at import time; keep stdout clean for the protocol line.
        with contextlib.redirect_stdout(captured):
            obj = ep.load()
            module = sys.modules[ep.module]
    except Exception as exc:  # noqa: BLE001 - report, never crash with a traceback on stdout
        fail("entry_point_load_failed", 70, entry_point=name, type=type(exc).__name__)
    root, how = select_root(click, obj, module)
    if root is None:
        fail("ambiguous_root", 65, entry_point=name, detail=how)
    with contextlib.redirect_stdout(captured):
        ctx = click.Context(root, info_name=name)
        with ctx.scope(cleanup=False):
            info = root.to_info_dict(ctx)
    dist = getattr(ep, "dist", None)
    try:
        click_version = importlib.metadata.version("click")
    except importlib.metadata.PackageNotFoundError:
        click_version = ""
    payload = {
        "protocol": PROTOCOL,
        "entry_point": name,
        "entry_value": ep.value,
        "main_module": main_module_for(ep.module),
        "root_selection": how,
        "distribution": {
            "name": dist.metadata["Name"] if dist is not None else "",
            "version": dist.version if dist is not None else "",
        },
        "click_version": click_version,
        "root": sanitize(info),
    }
    sys.stdout.write(json.dumps(payload, sort_keys=True, indent=1) + "\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
