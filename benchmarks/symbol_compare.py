#!/usr/bin/env python3
"""Observable symbol definition query: clangd LSP vs rg and Git grep.

A tiny tracked C fixture has one definition and one call. Report candidate
locations and actual costs, not subjective agent task-completion scores.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import statistics

from lsp_probe import probe
from simple_common import command, git_value

FILE = "benchmarks/fixtures/c-symbol/example.c"
SYMBOL = "compute"
CALL_LINE = 1
CALL_COLUMN = 24
DEFINITION_LINE = 0
DEFINITION_COLUMN = 4


def text_locations(stdout: bytes, arm: str, revision: str) -> list[tuple[int, int]]:
    candidates = []
    for line in stdout.decode("utf-8").splitlines():
        if arm == "git_grep":
            object_id, path, no, source = line.split(":", 3)
            if object_id != revision or path != FILE:
                raise ValueError("unpinned Git grep output")
        else:
            no, source = line.split(":", 1)
        start = 0
        while True:
            position = source.find(SYMBOL, start)
            if position < 0:
                break
            candidates.append((int(no) - 1, position))
            start = position + len(SYMBOL)
    return candidates


def execute(repo: Path, server: list[str], repetitions: int, timeout: float) -> dict:
    revision = git_value(repo, "rev-parse", "HEAD")
    if git_value(repo, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("symbol benchmark requires clean repository")
    if not (repo / FILE).is_file():
        raise ValueError("symbol fixture missing")
    rows = []
    for repetition in range(repetitions):
        for arm in ("rg", "git_grep", "lsp"):
            if arm == "lsp":
                if not server or shutil.which(server[0]) is None:
                    rows.append({
                        "arm": arm, "repetition": repetition,
                        "status": "unsupported",
                    })
                    continue
                row = probe(
                    server, repo, FILE, CALL_LINE, CALL_COLUMN, "c", timeout
                )
                candidates = []
                for location in row.get("locations", []):
                    if location.get("uri") == (repo / FILE).as_uri():
                        point = location.get("range", {}).get("start", {})
                        candidates.append(
                            (point.get("line"), point.get("character"))
                        )
                row.update({
                    "arm": arm, "repetition": repetition,
                    "candidate_locations": candidates,
                })
            else:
                argv = (
                    ["rg", "-n", "-F", "--no-heading", SYMBOL, str(repo / FILE)]
                    if arm == "rg" else [
                        "git", "-C", str(repo), "grep", "-n", "-F",
                        "-e", SYMBOL, revision, "--", FILE,
                    ]
                )
                proc, timings = command(argv, timeout)
                row = {
                    "arm": arm, "repetition": repetition, **timings,
                    "status": "ok" if proc.returncode == 0 else "error",
                    "returncode": proc.returncode,
                    "raw_stdout_bytes": len(proc.stdout),
                    "raw_stdout_sha256": hashlib.sha256(proc.stdout).hexdigest(),
                }
                if row["status"] == "ok":
                    row["candidate_locations"] = text_locations(
                        proc.stdout, arm, revision
                    )
                else:
                    row["error"] = proc.stderr.decode(errors="replace")[:500]
            if row["status"] == "ok":
                points = row["candidate_locations"]
                row["candidate_count"] = len(points)
                row["contains_declared_definition"] = (
                    DEFINITION_LINE, DEFINITION_COLUMN
                ) in points
            rows.append(row)
    arms = ("rg", "git_grep", "lsp")
    summary = {
        "schema": "excession.benchmark.symbol.v1",
        "source_commit": revision,
        "source_file": FILE,
        "repetitions": repetitions,
        "status_counts": {
            arm: {
                state: sum(
                    row["arm"] == arm and row["status"] == state for row in rows
                )
                for state in ("ok", "error", "unsupported")
            }
            for arm in arms
        },
        "median_candidates": {
            arm: statistics.median(
                row["candidate_count"] for row in rows
                if row["arm"] == arm and row["status"] == "ok"
            ) if any(
                row["arm"] == arm and row["status"] == "ok" for row in rows
            ) else None
            for arm in arms
        },
        "definition_retrieved_trials": {
            arm: sum(
                row.get("contains_declared_definition") is True
                for row in rows if row["arm"] == arm
            )
            for arm in arms
        },
        "limitations": "One controlled C fixture. LSP returns definition locations; rg and Git grep return textual occurrences. Excession's module views do not support C symbols. No agent productivity inference.",
    }
    return {"summary": summary, "trials": rows}


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--repo", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--repetitions", type=int, default=3)
    p.add_argument("--timeout", type=float, default=15)
    p.add_argument(
        "--server", nargs="*", default=["clangd", "--background-index=false"]
    )
    a = p.parse_args()
    if a.repetitions <= 0 or a.timeout <= 0:
        p.error("repetitions and timeout must be positive")
    out = Path(a.out).resolve()
    if out.exists():
        p.error("output already exists")
    repo = Path(a.repo).resolve(strict=True)
    result = execute(repo, a.server, a.repetitions, a.timeout)
    out.mkdir()
    (out / "summary.json").write_text(
        json.dumps(result["summary"], indent=2, sort_keys=True) + "\n"
    )
    with (out / "trials.jsonl").open("w") as f:
        for row in result["trials"]:
            f.write(json.dumps(row, sort_keys=True) + "\n")
    print(json.dumps(result["summary"], indent=2, sort_keys=True))
    if any(
        row["status"] == "error" or (
            row["status"] == "ok" and not row["contains_declared_definition"]
        ) for row in result["trials"]
    ):
        raise SystemExit(2)


if __name__ == "__main__":
    main()
