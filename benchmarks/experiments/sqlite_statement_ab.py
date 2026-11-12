#!/usr/bin/env python3
"""Same-run A/B of released Excession indexing versus prepared SQLite statements.

Two production binaries analyze the *same* generated Git revisions, using
separate SQLite files and equal parser/config inputs. Identical source-bound
public reports and the complete persisted semantic rows are mandatory; timing
differences are observations, not an automatic performance-pass threshold.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sqlite3
import statistics
import subprocess
import tempfile

from index_incremental_cli import analyze, evaluate, snapshot


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args], text=True, check=True,
        capture_output=True, timeout=90,
    ).stdout.strip()


def fixture(root: Path, modules: int, functions: int) -> tuple[str, str]:
    root.mkdir()
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.name", "SQLite A/B")
    git(root, "config", "user.email", "sqlite-ab@example.invalid")
    pkg = root / "pkg"
    pkg.mkdir()
    (pkg / "__init__.py").write_text("", encoding="utf-8")
    for n in range(modules):
        content = "VALUE = 1\n" if n == 0 else (
            f"import pkg.m{n - 1:04d}\nVALUE = {n}\n"
        )
        content += "".join(
            f"\ndef function_{i}(x: int) -> int:\n    return x + {n + i}\n"
            for i in range(functions)
        )
        (pkg / f"m{n:04d}.py").write_text(content, encoding="utf-8")
    git(root, "add", ".")
    git(root, "commit", "-qm", "baseline source observations")
    base = git(root, "rev-parse", "HEAD")
    (pkg / "m0000.py").write_text(
        (pkg / "m0000.py").read_text(encoding="utf-8").replace(
            "VALUE = 1\n", "VALUE = 2\n", 1
        ),
        encoding="utf-8",
    )
    git(root, "add", ".")
    git(root, "commit", "-qm", "change one module")
    head = git(root, "rev-parse", "HEAD")
    return base, head


def compare_reports(
    baseline_bin: Path, candidate_bin: Path,
    source: Path, base: str, head: str, workspace: Path,
) -> None:
    # Repeated indexing of a parent revision must preserve original evidence.
    outcomes = {}
    for name, binary in (("baseline", baseline_bin), ("candidate", candidate_bin)):
        for mode in ("fresh", "reused"):
            database = workspace / f"{name}-{mode}.sqlite"
            if mode == "reused":
                analyze(binary, source, base, "pkg", database)
            report, _ = analyze(binary, source, head, "pkg", database)
            receipt, rows = snapshot(database, head)
            with sqlite3.connect(database) as conn:
                stored = {
                    table: conn.execute(
                        f"SELECT COUNT(*) FROM {table} WHERE commit_oid=?",
                        (head,),
                    ).fetchone()[0]
                    for table in ["tree_entry", "git_object", "module", "symbol"]
                    if table != "git_object"
                }
            outcomes[(name, mode)] = (report, rows, stored, receipt[0:2])
    canonical_report, canonical_rows, canonical_counts, _ = outcomes[("baseline", "fresh")]
    for (name, mode), (report, rows, counts, receipt_counts) in outcomes.items():
        if report != canonical_report:
            raise ValueError(f"{name}/{mode} public report differs from unmodified Excession")
        if rows != canonical_rows or counts != canonical_counts:
            raise ValueError(f"{name}/{mode} SQL observation or source identity differs")
        if mode == "reused" and receipt_counts[1] == 0:
            raise ValueError(f"{name}/{mode} did not reuse any source observations")
        if mode == "fresh" and receipt_counts[1] != 0:
            raise ValueError(f"{name}/{mode} unexpectedly reused source observations")
    if git(source, "status", "--porcelain"):
        raise ValueError("A/B experiment modified the input Git checkout")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-bin", type=Path, required=True)
    parser.add_argument("--candidate-bin", type=Path, required=True)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.repetitions <= 20:
        parser.error("repetitions must be 1..20")
    try:
        cases = []
        with tempfile.TemporaryDirectory(prefix="excession-sqlite-ab-") as tmp:
            home = Path(tmp)
            for modules, functions in [(256, 0), (512, 32)]:
                label = f"{modules}x{functions}"
                source = home / f"source-{label}"
                base, head = fixture(source, modules, functions)
                workspace = home / f"comparisons-{label}"
                workspace.mkdir()
                compare_reports(
                    args.baseline_bin.resolve(), args.candidate_bin.resolve(),
                    source, base, head, workspace,
                )
                measurements = {}
                for name, binary in [
                    ("baseline", args.baseline_bin.resolve()),
                    ("candidate", args.candidate_bin.resolve()),
                ]:
                    measurements[name] = evaluate(
                        binary, source, base, head, "pkg", args.repetitions,
                    )
                if any(x["status"] != "MATCH" for x in measurements.values()):
                    raise ValueError(f"{label}: fresh-vs-incremental equality failed")
                medians = {
                    name: measurements[name]["median_ms"] for name in measurements
                }
                ratios = {
                    stage: round(medians["baseline"][stage] /
                                 medians["candidate"][stage], 3)
                    for stage in ("base", "fresh", "reused")
                }
                cases.append({
                    "fixture": label,
                    "module_count": modules + 1,
                    "functions_per_module": functions,
                    "base_commit": base,
                    "head_commit": head,
                    "baseline_median_ms": medians["baseline"],
                    "candidate_median_ms": medians["candidate"],
                    "baseline_over_candidate_ratio": ratios,
                    "samples_ms": {
                        name: measurements[name]["samples_ms"]
                        for name in measurements
                    },
                    "source_and_derived_rows_equal": True,
                })
        result = {
            "schema": "excession.sqlite-statement-ab.v1",
            "status": "MATCH",
            "repetitions": args.repetitions,
            "cases": cases,
            "interpretation": (
                "One runner, same pinned Git commits and source documents, "
                "distinct SQLite files, identical public reports and derived rows. "
                "Cold base, full target, and reused target timings are separate. "
                "No safe-test-elision or semantics claim. Timing is not a CI gate."
            ),
        }
    except Exception as err:
        result = {
            "schema": "excession.sqlite-statement-ab.v1",
            "status": "ERROR",
            "error": f"{type(err).__name__}: {err}",
        }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=2) + "\n")
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0 if result["status"] == "MATCH" else 1


if __name__ == "__main__":
    raise SystemExit(main())
