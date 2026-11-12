#!/usr/bin/env python3
"""Full-revision vs parent-reuse measurements of the genuine Rust Excession CLI.

Only compare identical target revisions, roots, adapters, report bytes and SQL
derived tables. Measure base indexing separately. Not a semantic-runtime proof.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sqlite3
import statistics
import subprocess
import tempfile
import time


TABLES = [
    ("module", "name,path,blob_oid,is_test", "name,path"),
    ("dependency", "source_name,target_name,line,syntax",
     "source_name,target_name,line,syntax"),
    ("symbol", "module_name,qualname,kind,signature,body_fingerprint,line",
     "module_name,qualname"),
    ("api_binding", "module_name,public_name,target,explicitly_exported",
     "module_name,public_name"),
    ("hazard", "module_name,line,reason", "module_name,line,reason"),
]


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        text=True, capture_output=True, check=True, timeout=30,
    ).stdout.strip()


def analyze(binary: Path, repo: Path, rev: str, roots: str, db: Path):
    args = [
        str(binary), "analyze", str(repo), "--rev", rev, "--roots", roots,
        "--db", str(db), "--json",
    ]
    started = time.perf_counter_ns()
    proc = subprocess.run(args, capture_output=True, text=True, timeout=90)
    elapsed = (time.perf_counter_ns() - started) / 1_000_000
    if proc.returncode:
        raise RuntimeError(f"native analyze failed: {proc.stderr[-1400:]}")
    return json.loads(proc.stdout), elapsed


def snapshot(db: Path, commit: str):
    with sqlite3.connect(db) as con:
        result = con.execute(
            "SELECT parsed_modules,reused_modules,rows_sha256 "
            "FROM python_index_snapshot WHERE commit_oid=?",
            (commit,),
        ).fetchone()
        if result is None:
            raise ValueError(f"no completed Python snapshot for {commit}")
        if con.execute("PRAGMA foreign_key_check").fetchall():
            raise ValueError("indexed SQL violates a foreign key")
        result_rows = {}
        for table, fields, order in TABLES:
            result_rows[table] = con.execute(
                f"SELECT {fields} FROM {table} WHERE commit_oid=? ORDER BY {order}",
                (commit,),
            ).fetchall()
    return result, result_rows


def evaluate(binary: Path, repo: Path, base: str, head: str,
             roots: str, repetitions: int) -> dict:
    if git(repo, "rev-parse", "HEAD") != head:
        raise ValueError("fixture must be checked out at the pinned head revision")
    actual_parent = git(repo, "rev-list", "--parents", "-n", "1", head).split()
    if actual_parent != [head, base]:
        raise ValueError("the pinned base must be the head commit's sole parent")
    recorded = {"fresh": [], "reused": [], "base": []}
    parsed, reused = [], []
    with tempfile.TemporaryDirectory(prefix="excession-native-incremental-") as tmp:
        work = Path(tmp)
        for n in range(repetitions):
            fresh_db = work / f"{n}-fresh.sqlite"
            incremental_db = work / f"{n}-incremental.sqlite"
            _, base_ms = analyze(binary, repo, base, roots, incremental_db)
            recorded["base"].append(base_ms)
            outcomes = {}
            # Alternate order so one approach is not always first in a warm FS.
            order = ["fresh", "reused"] if n % 2 == 0 else ["reused", "fresh"]
            for method in order:
                database = fresh_db if method == "fresh" else incremental_db
                response, elapsed = analyze(binary, repo, head, roots, database)
                recorded[method].append(elapsed)
                receipt, tables = snapshot(database, head)
                outcomes[method] = (response, receipt, tables)
            fresh_report, fresh_receipt, fresh_rows = outcomes["fresh"]
            reused_report, reused_receipt, reused_rows = outcomes["reused"]
            if fresh_report != reused_report:
                raise ValueError("native incremental analysis differs from fresh public report")
            if fresh_rows != reused_rows or fresh_receipt[2] != reused_receipt[2]:
                raise ValueError("native incremental persisted relationships differ from fresh")
            if fresh_receipt[1] != 0:
                raise ValueError("fresh head index unexpectedly reused parent observations")
            if fresh_receipt[0] != reused_receipt[0] + reused_receipt[1]:
                raise ValueError("incremental module conservation differs from full indexing")
            parsed.append(reused_receipt[0])
            reused.append(reused_receipt[1])
        if not reused or min(reused) == 0:
            raise ValueError("no source observations reused in any incremental trial")
    medians = {key: round(statistics.median(values), 3)
               for key, values in recorded.items()}
    return {
        "schema": "excession.native-incremental-cli-benchmark.v1",
        "status": "MATCH",
        "source": "source-bound Rust CLI and SQLite, CPython native adapter",
        "base_commit": base,
        "head_commit": head,
        "roots": roots,
        "repetitions": repetitions,
        "median_ms": medians,
        "speedup_warm_vs_fresh": round(medians["fresh"] / medians["reused"], 3),
        "median_base_index_ms": medians["base"],
        "samples_ms": {key: [round(x, 3) for x in vals]
                       for key, vals in recorded.items()},
        "head_parsed_modules": parsed,
        "head_reused_modules": reused,
        "exact_reports_and_persisted_rows_equal": True,
        "interpretation": (
            "End-to-end target-revision indexing with and without existing parent "
            "snapshot; base indexing is an additional initial cost. Does not "
            "establish dynamic import closure or safe test skipping."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--roots", default="h11,tests")
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.repetitions <= 30:
        parser.error("repetitions must be 1..30")
    try:
        if any(len(x) != 40 or any(c not in "0123456789abcdef" for c in x)
               for x in (args.base, args.head)):
            raise ValueError("pin both revisions with full lowercase Git SHA-1")
        report = evaluate(args.bin.resolve(), args.repo.resolve(),
                          args.base, args.head, args.roots, args.repetitions)
    except Exception as exc:
        report = {
            "schema": "excession.native-incremental-cli-benchmark.v1",
            "status": "ERROR", "error": f"{type(exc).__name__}: {exc}",
        }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0 if report["status"] == "MATCH" else 1


if __name__ == "__main__":
    raise SystemExit(main())
