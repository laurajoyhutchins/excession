#!/usr/bin/env python3
"""Read-only terminal explorer for Excession's Git-backed virtual layouts.

Every operation passes through the Rust projection authority. This program
never extracts source files or interprets the code in the target repository.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys


def resolve(current: str, requested: str) -> str:
    """Resolve a virtual navigation path while refusing to leave the view root."""
    parts = [] if requested.startswith("/") else current.split("/") if current else []
    for component in requested.split("/"):
        if component in ("", "."):
            continue
        if component == "..":
            if not parts:
                raise ValueError("cannot navigate above virtual root")
            parts.pop()
        else:
            parts.append(component)
    return "/".join(parts)


def visible_bytes(data: bytes, max_bytes: int) -> str:
    """Escape terminal control characters; this is a viewer, not an executor."""
    sample = data[:max_bytes]
    out = []
    for char in sample.decode("utf-8", errors="replace"):
        if char in "\n\t" or char.isprintable():
            out.append(char)
        else:
            out.append(char.encode("unicode_escape").decode("ascii"))
    if len(data) > max_bytes:
        out.append(f"\n[display truncated at {max_bytes} bytes; blob not changed]")
    return "".join(out)


class Browser:
    def __init__(self, executable: Path, repository: Path, manifest: Path, max_bytes: int):
        self.executable = executable
        self.repository = repository
        self.manifest = manifest
        self.max_bytes = max_bytes
        self.cwd = ""

    def query(self, verb: str, path: str = "", *, raw: bool = False):
        operation = {'view-ls': 'ls', 'view-stat': 'stat', 'view-open': 'cat'}[verb]
        argv = [str(self.executable), 'view', operation, '--repo', str(self.repository),
                '--manifest', str(self.manifest)]
        if path:
            argv.extend(["--path", path])
        result = subprocess.run(argv, capture_output=True, check=False)
        if result.returncode:
            msg = result.stderr.decode("utf-8", errors="replace").strip()
            raise ValueError(msg or f"Excession command failed: {verb}")
        return result.stdout if raw else json.loads(result.stdout)

    def execute(self, command: str) -> bool:
        args = shlex.split(command)
        if not args:
            return True
        verb, *rest = args
        if verb in ("quit", "exit"):
            return False
        if verb == "help":
            print("Commands: ls [directory], cd DIRECTORY, pwd, stat FILE, show FILE, quit")
            return True
        if verb == "pwd" and not rest:
            print("/" + self.cwd)
            return True
        if verb in ("ls", "cd", "stat", "show"):
            if (verb == "ls" and len(rest) > 1) or (verb != "ls" and len(rest) != 1):
                raise ValueError(f"usage: {verb} " + ("PATH" if verb != "ls" else "[DIRECTORY]"))
            path = resolve(self.cwd, rest[0] if rest else ".")
            if verb == "ls":
                items = self.query("view-ls", path)["children"]
                for item in items:
                    suffix = "/" if item["type"] == "directory" else ""
                    print(f"{item['name']!r}{suffix}  ({item['references']} refs)")
            elif verb == "cd":
                self.query("view-ls", path)
                self.cwd = path
            elif verb == "stat":
                print(json.dumps(self.query("view-stat", path), indent=2, sort_keys=True))
            else:
                print(visible_bytes(self.query("view-open", path, raw=True), self.max_bytes))
            return True
        raise ValueError("unknown command; type help")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repository", type=Path, help="Git repository containing the pinned blobs")
    parser.add_argument("manifest", type=Path, help="Existing Excession view manifest")
    parser.add_argument("--rust", type=Path, default=Path(__file__).resolve().parents[1] / "core/target/debug/excession")
    parser.add_argument("--max-bytes", type=int, default=8192)
    args = parser.parse_args()
    if not 1 <= args.max_bytes <= 65536:
        parser.error("--max-bytes must be in 1..65536")
    browser = Browser(args.rust, args.repository, args.manifest, args.max_bytes)
    try:
        browser.query("view-ls")
    except (ValueError, OSError) as e:
        parser.error(f"unable to authenticate view: {e}")
    interactive = sys.stdin.isatty()
    if interactive:
        print("Excession virtual browser. Type help for commands.")
    while True:
        try:
            if interactive:
                print(f"excession:/{browser.cwd}> ", end="", flush=True)
            line = sys.stdin.readline()
            if not line:
                break
            try:
                if not browser.execute(line):
                    break
            except (ValueError, OSError) as e:
                print(f"error: {e}", file=sys.stderr)
                if not interactive:
                    return 1
        except KeyboardInterrupt:
            break
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
