#!/usr/bin/env python3
"""Pinned module-file lookup: ripgrep, Git, native graph, and derived views.

This compares exact source filenames, NOT keyword-based transitive traversal.
Both filesystem tools require a clean checkout at the requested commit.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import random
import statistics

from simple_common import command, git_value, load_tasks

ARMS = ("rg", "git", "rust_graph", "view")


def normalize(data: bytes, arm: str, repo: Path, basename: str,
              revision: str, seed: str) -> list[str]:
    if arm in ("rg", "git"):
        result = set()
        for line in data.decode("utf-8").splitlines():
            if not line:
                continue
            path = Path(line)
            if path.is_absolute():
                path = path.relative_to(repo)
            if path.name == basename and not path.is_absolute() and ".." not in path.parts:
                result.add(path.as_posix())
        return sorted(result)
    output = json.loads(data)
    if arm not in ("rust_graph", "view"):
        raise ValueError("unknown retrieval arm")
    if output.get("source_commit") != revision:
        raise ValueError("retrieval output uses a different source revision")
    return sorted(output["source_paths"])


def run(args: argparse.Namespace) -> dict:
    repo = Path(args.repo).resolve(strict=True)
    db = Path(args.db).resolve(strict=True)
    binary = Path(args.bin).resolve(strict=True)
    out = Path(args.out).resolve()
    if out.exists():
        raise ValueError("output already exists")
    if repo == out or repo in out.parents:
        raise ValueError("output must remain outside source repository")
    tasks = [t for t in load_tasks(Path(args.tasks)) if t["kind"] == "focus"]
    if not tasks:
        raise ValueError("no exact module-location tasks")
    revision = git_value(repo, "rev-parse", "--verify", f"{args.rev}^{{commit}}")
    if git_value(repo, "rev-parse", "HEAD") != revision:
        raise ValueError("rg reads working tree; requested commit must be HEAD")
    if git_value(repo, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("rg requires a clean working tree")
    out.mkdir(parents=True)
    order = [(t, repeat) for repeat in range(args.repetitions) for t in tasks]
    random.Random(args.random_seed).shuffle(order)
    records = []
    for i, (task, repeat) in enumerate(order):
        basename = task["seed"].rsplit(".", 1)[-1] + ".py"
        arms = ARMS[i % len(ARMS):] + ARMS[:i % len(ARMS)]
        for arm in arms:
            if arm == "rg":
                argv = ["rg", "--files", "--hidden", "--no-ignore",
                        "-g", basename, "-g", "!.git", str(repo)]
            elif arm == "git":
                argv = ["git", "-C", str(repo), "ls-tree", "-r", "--name-only",
                        revision, "--", "h11"]
            elif arm == "rust_graph":
                argv = [str(binary), "view", "graph", "--db", str(db),
                        "--name", args.name, "--rev", revision,
                        "--seed", task["seed"], "--kind", "focus"]
            else:
                argv = [str(binary), "view", "paths", "--db", str(db),
                        "--name", args.name, "--rev", revision,
                        "--seed", task["seed"], "--role", "focus"]
            proc, timings = command(argv, args.timeout)
            row = {
                "task_id": task["id"], "repeat": repeat, "arm": arm,
                "source_commit": revision,
                "status": "ok" if proc.returncode == 0 else ("unsupported" if proc.returncode == 127 else "error"),
                "returncode": proc.returncode, "raw_stdout_bytes": len(proc.stdout),
                "raw_stdout_sha256": hashlib.sha256(proc.stdout).hexdigest(),
                **timings,
            }
            if proc.returncode == 0:
                try:
                    paths = normalize(proc.stdout, arm, repo, basename, revision, task["seed"])
                    row["source_paths"] = paths
                    row["declared_paths_equal"] = paths == sorted(task["expected_source_paths"])
                    row["selected_output_bytes"] = len(
                        json.dumps({"source_paths": paths}, separators=(",", ":")).encode()
                    )
                except (ValueError, KeyError, TypeError, UnicodeError) as exc:
                    row["status"] = "invalid-output"
                    row["error"] = str(exc)
            else:
                row["error"] = proc.stderr.decode(errors="replace")[:500]
            records.append(row)
    with (out / "trials.jsonl").open("w") as f:
        for record in records:
            f.write(json.dumps(record, sort_keys=True) + "\n")
    by_arm = {arm: [r for r in records if r["arm"] == arm] for arm in ARMS}
    summary = {
        "schema": "excession.benchmark.locate.v1", "revision": revision,
        "task_count": len(tasks), "repetitions": args.repetitions,
        "status_counts": {
            arm: {state: sum(r["status"] == state for r in rows)
                  for state in ("ok", "error", "unsupported", "invalid-output")}
            for arm, rows in by_arm.items()
        },
        "declared_path_matches": {
            arm: sum(r.get("declared_paths_equal") is True for r in rows)
            for arm, rows in by_arm.items()
        },
        "median_wall_ms": {
            arm: round(statistics.median(r["wall_ms"] for r in rows), 3)
            for arm, rows in by_arm.items()
        },
        "median_raw_stdout_bytes": {
            arm: statistics.median(r["raw_stdout_bytes"] for r in rows)
            for arm, rows in by_arm.items()
        },
        "limitations": "Four exact file lookups only. Git enumerates all h11 files and is filtered after timing; rg reads clean HEAD. Indexing excluded from per-query time.",
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    if any(r["status"] != "ok" or r.get("declared_paths_equal") is not True for r in records):
        raise SystemExit(2)
    return summary


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    for field in ("repo", "db", "bin", "tasks", "out", "name"):
        p.add_argument("--" + field, required=True)
    p.add_argument("--rev", default="HEAD")
    p.add_argument("--repetitions", type=int, default=3)
    p.add_argument("--random-seed", type=int, default=20261110)
    p.add_argument("--timeout", type=float, default=60)
    args = p.parse_args()
    if args.repetitions <= 0 or args.timeout <= 0:
        p.error("repetitions and timeout must be positive")
    run(args)


if __name__ == "__main__":
    main()
