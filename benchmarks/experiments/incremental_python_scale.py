#!/usr/bin/env python3
"""Scale the *same* Python AST incremental experiment on a generated Git history.

No new parser is introduced. A single source change retains the module catalog,
and the independent oracle checks the complete resulting static import graph.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from incremental_python import evaluate


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True, text=True, check=True, timeout=120,
    )
    return result.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--modules", type=int, default=256)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if not 4 <= args.modules <= 2048 or not 1 <= args.repetitions <= 50:
        parser.error("modules must be 4..2048, repetitions 1..50")
    with tempfile.TemporaryDirectory(prefix="excession-incremental-scale-") as tmp:
        repo = Path(tmp) / "repo"
        repo.mkdir()
        git(repo, "init", "-q", "-b", "main")
        git(repo, "config", "user.name", "Incremental scaling")
        git(repo, "config", "user.email", "incremental@example.invalid")
        pkg = repo / "pkg"
        pkg.mkdir()
        (pkg / "__init__.py").write_text("", encoding="utf-8")
        for n in range(args.modules):
            code = ("VALUE = 0\n" if n == 0 else
                    f"import pkg.m{n - 1:04d}\nVALUE = {n}\n")
            (pkg / f"m{n:04d}.py").write_text(code, encoding="utf-8")
        git(repo, "add", ".")
        git(repo, "commit", "-qm", "chain baseline")
        base = git(repo, "rev-parse", "HEAD")
        (pkg / "m0000.py").write_text("VALUE = 1\n", encoding="utf-8")
        git(repo, "add", ".")
        git(repo, "commit", "-qm", "change first module only")
        head = git(repo, "rev-parse", "HEAD")
        result = evaluate(repo, base, head, ("pkg",), args.repetitions)
    result["fixture"] = "generated-static-import-chain"
    result["requested_modules"] = args.modules
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({
        "status": result["status"],
        "modules": result["head_sources"],
        "reparsed": result["reparsed_modules"],
        "reused": result["reused_modules"],
        "median_full_head_ms": result["median_full_head_ms"],
        "median_incremental_head_ms": result["median_incremental_head_ms"],
        "missing_edges": len(result["missing_edges"]),
        "unexpected_edges": len(result["unexpected_edges"]),
    }, sort_keys=True))
    return 0 if result["status"] == "MATCH" else 1


if __name__ == "__main__":
    raise SystemExit(main())
