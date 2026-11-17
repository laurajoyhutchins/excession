#!/usr/bin/env python3
"""Trusted-local CPython source-line/value observer, not a sandbox or attestation.

Executed by Excession with -I against a detached Git worktree. Target code
can disable tracing or escape the process; observations are deliberately
scoped to this one run, not evidence of complete coverage or equivalence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import sys


def scalar(value: object) -> dict[str, object]:
    """Never invoke user-defined __repr__ or traverse arbitrary containers."""
    typ = type(value)
    if value is None:
        return {"type": "none", "value": None}
    if typ is bool:
        return {"type": "bool", "value": value}
    if typ is int:
        rendered = str(value) if value.bit_length() <= 256 else None
        return {"type": "int", "value": rendered} if rendered is not None else {
            "type": "int", "value": "UNAVAILABLE_TOO_LARGE"
        }
    if typ is float:
        return {"type": "float", "value": repr(value)}
    if typ is str:
        encoded = value.encode("utf-8", errors="surrogatepass")
        return {"type": "str", "sha256": hashlib.sha256(encoded).hexdigest(), "bytes": len(encoded)}
    if typ is bytes:
        return {"type": "bytes", "sha256": hashlib.sha256(value).hexdigest(), "bytes": len(value)}
    return {"type": "opaque", "value": "UNOBSERVED_MUTABLE_OR_OBJECT"}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--max-events", required=True, type=int)
    parser.add_argument("--max-bytes", required=True, type=int)
    parser.add_argument("script")
    parser.add_argument("args", nargs=argparse.REMAINDER)
    opts = parser.parse_args()
    root = Path(opts.root).resolve(strict=True)
    if Path.cwd().resolve() != root:
        parser.error("instrumented Git worktree is not the process working directory")
    source = Path(opts.script)
    if source.is_absolute() or ".." in source.parts or not source.is_file():
        parser.error("script must be an existing relative worktree file")
    resolved = source.resolve(strict=True)
    if not resolved.is_relative_to(root):
        parser.error("script escapes the detached Git worktree")
    out = Path(opts.out)
    if not out.is_absolute() or out.parent.resolve() == root or out.resolve().is_relative_to(root):
        parser.error("trace output must be outside the source worktree")
    if opts.max_events < 1 or opts.max_bytes < 1024:
        parser.error("positive trace budget and at least 1024 trace bytes required")

    source_prefix = str(root) + os.sep
    snapshots: dict[int, tuple[dict[str, object], int]] = {}
    emitted = 0
    used = 0
    truncated = False
    partial_values = False

    with out.open("xb") as stream:
        def write(record: dict[str, object]) -> bool:
            nonlocal used
            payload = (json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n").encode()
            # Reserve room for a final END record even at the byte boundary.
            if used + len(payload) + 200 > opts.max_bytes:
                return False
            stream.write(payload)
            used += len(payload)
            return True

        def trace(frame: object, event: str, arg: object) -> object:
            nonlocal emitted, truncated, partial_values
            if event not in ("call", "line", "return", "exception"):
                return trace
            code = frame.f_code
            # Frozen/internal CPython code objects use symbolic names such
            # as "<frozen runpy>". realpath() would incorrectly turn those
            # into fake paths inside our current working tree.
            if not os.path.isabs(code.co_filename):
                return None
            filename = os.path.realpath(code.co_filename)
            if not filename.startswith(source_prefix) or not filename.endswith(".py"):
                return None
            path = os.path.relpath(filename, str(root)).replace(os.sep, "/")
            fid = id(frame)
            previous, prev_line = snapshots.get(fid, ({}, 0))
            now: dict[str, object] = {}
            keys = sorted(k for k in frame.f_locals if isinstance(k, str) and not k.startswith("__"))
            if len(keys) > 128:
                partial_values = True
            for name in keys[:128]:
                if len(name) <= 128:
                    now[name] = scalar(frame.f_locals[name])
                else:
                    partial_values = True
            changes = [
                {"name": name, "before": previous.get(name, {"type": "absent"}),
                 "after": now.get(name, {"type": "absent"}), "after_line": prev_line}
                for name in sorted(set(previous) | set(now))
                if previous.get(name) != now.get(name)
            ]
            record = {
                "kind": event.upper(),
                "path": path,
                "line": frame.f_lineno,
                "function": code.co_name[:256],
                "values": now,
                "changes": changes,
                "return_value": scalar(arg) if event == "return" else None,
            }
            if not truncated:
                if emitted >= opts.max_events or not write(record):
                    truncated = True
                else:
                    emitted += 1
            if event == "return":
                snapshots.pop(fid, None)
            else:
                snapshots[fid] = (now, frame.f_lineno)
            return trace

        sys.dont_write_bytecode = True
        sys.path.insert(0, str(root))
        sys.argv = [opts.script, *opts.args]
        try:
            sys.settrace(trace)
            runpy.run_path(str(resolved), run_name="__main__")
        finally:
            sys.settrace(None)
            marker = {
                "kind": "END",
                "complete": not truncated,
                "truncated": truncated,
                "events": emitted,
                "values_partial": partial_values,
            }
            # END is never silently omitted by an internal buffer limit.
            encoded = (json.dumps(marker, sort_keys=True, separators=(",", ":")) + "\n").encode()
            stream.write(encoded)
            stream.flush()


if __name__ == "__main__":
    main()
