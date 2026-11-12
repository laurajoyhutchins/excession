#!/usr/bin/env python3
"""End-to-end Rust incremental benchmark on a 256-module Git history."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from index_incremental_cli import evaluate


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        text=True, capture_output=True, check=True, timeout=60,
    ).stdout.strip()


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--bin", type=Path, required=True)
    p.add_argument("--modules", type=int, default=256)
    p.add_argument("--repetitions", type=int, default=5)
    p.add_argument("--functions-per-module", type=int, default=0)
    p.add_argument("--out", type=Path, required=True)
    args = p.parse_args()
    if (not 8 <= args.modules <= 2048 or not 1 <= args.repetitions <= 20
            or not 0 <= args.functions_per_module <= 128):
        p.error("modules 8..2048; repetitions 1..20; functions-per-module 0..128")
    with tempfile.TemporaryDirectory(prefix="excession-native-scale-") as tmp:
        repo = Path(tmp) / "repo"
        pkg = repo / "pkg"
        pkg.mkdir(parents=True)
        git(repo, "init", "-q", "-b", "main")
        git(repo, "config", "user.name", "Native incremental scale")
        git(repo, "config", "user.email", "incremental@example.invalid")
        (pkg / "__init__.py").write_text("", encoding="utf-8")
        for n in range(args.modules):
            content = "VALUE = 1\n" if n == 0 else (
                f"import pkg.m{n - 1:04d}\nVALUE = {n}\n"
            )
            # Nontrivial ASTs and symbol metadata expose whether the existing
            # SQLite/producer path can avoid work that matters in real modules.
            content += "".join(
                f"\ndef compute_{i}(x: int) -> int:\n    return x + {n + i}\n"
                for i in range(args.functions_per_module)
            )
            (pkg / f"m{n:04d}.py").write_text(content, encoding="utf-8")
        git(repo, "add", ".")
        git(repo, "commit", "-qm", "baseline")
        base = git(repo, "rev-parse", "HEAD")
        (pkg / "m0000.py").write_text("VALUE = 2\n", encoding="utf-8")
        git(repo, "add", ".")
        git(repo, "commit", "-qm", "change one module")
        head = git(repo, "rev-parse", "HEAD")
        try:
            result = evaluate(
                args.bin.resolve(), repo, base, head,
                "pkg", args.repetitions,
            )
            result["generated_modules"] = args.modules
            result["functions_per_module"] = args.functions_per_module
        except Exception as exc:
            result = {
                "schema": "excession.native-incremental-cli-benchmark.v1",
                "status": "ERROR", "generated_modules": args.modules,
                "error": f"{type(exc).__name__}: {exc}",
            }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({
        key: result.get(key) for key in [
            "status", "generated_modules", "functions_per_module", "median_ms",
            "speedup_warm_vs_fresh", "head_parsed_modules",
            "head_reused_modules", "error",
        ]
    }, sort_keys=True))
    return 0 if result["status"] == "MATCH" else 1


if __name__ == "__main__":
    raise SystemExit(main())
