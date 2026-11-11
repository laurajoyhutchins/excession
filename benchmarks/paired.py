#!/usr/bin/env python3
"""Measure graph-query and derived-view output and resource costs on pinned Git.

The two arms consume exactly the same module/dependency index. No agent outcome
or human-assigned task-success score is claimed by this calibration.
"""
from __future__ import annotations

import argparse
import math
from collections import deque
import hashlib
import json
import os
from pathlib import Path
import random
import resource
import sqlite3
import statistics
import subprocess
import sys
import time
from typing import Any

KINDS = (
    "focus", "dependencies_direct", "dependencies_all",
    "dependents_direct", "dependents_all", "tests",
)
ROLES = {
    "focus": {"focus"},
    "dependencies_direct": {"direct-dependencies"},
    "dependencies_all": {"direct-dependencies", "transitive-dependencies"},
    "dependents_direct": {"direct-dependents"},
    "dependents_all": {"direct-dependents", "transitive-dependents"},
    "tests": {"indexed-test"},
}
SCHEMA = "excession.benchmark.calibration.v1"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(argv: list[str], timeout: float) -> tuple[subprocess.CompletedProcess[bytes], dict[str, float]]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter_ns()
    try:
        proc = subprocess.run(argv, capture_output=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired as exc:
        proc = subprocess.CompletedProcess(argv, 124, exc.stdout or b"", exc.stderr or b"timed out")
    wall = (time.perf_counter_ns() - start) / 1_000_000
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    return proc, {
        "wall_ms": round(wall, 3),
        "child_user_cpu_ms": round((after.ru_utime - before.ru_utime) * 1000, 3),
        "child_system_cpu_ms": round((after.ru_stime - before.ru_stime) * 1000, 3),
    }


def checked(argv: list[str], timeout: float) -> tuple[bytes, dict[str, float]]:
    proc, measure = command(argv, timeout)
    if proc.returncode:
        raise RuntimeError(
            f"command exited {proc.returncode}: {argv[:5]!r}: "
            f"{proc.stderr.decode('utf-8', errors='replace')[:1500]}"
        )
    return proc.stdout, measure


def load_tasks(path: Path) -> list[dict[str, Any]]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if raw.get("schema") != SCHEMA:
        raise ValueError("unknown calibration task schema")
    tasks = raw.get("tasks")
    if not isinstance(tasks, list) or not tasks:
        raise ValueError("tasks must be a nonempty list")
    ids: set[str] = set()
    for task in tasks:
        if not isinstance(task, dict) or not all(isinstance(task.get(k), str) and task[k] for k in ("id", "seed", "kind", "question")):
            raise ValueError("task requires id, seed, kind, question")
        if task["id"] in ids or task["kind"] not in KINDS:
            raise ValueError("duplicate task id or unsupported kind")
        ids.add(task["id"])
        expected = task.get("expected_source_paths")
        if expected is not None and (not isinstance(expected, list) or not all(isinstance(p, str) and p and not p.startswith("/") and ".." not in p.split("/") for p in expected) or len(expected) != len(set(expected))):
            raise ValueError("invalid expected paths")
    return tasks


def graph_paths(db_path: Path, name: str, revision: str, seed: str, kind: str) -> list[str]:
    if kind not in KINDS:
        raise ValueError("unknown task kind")
    with sqlite3.connect(f"file:{db_path}?mode=ro", uri=True) as db:
        repo = db.execute("SELECT id FROM repository WHERE name=?", (name,)).fetchone()
        if repo is None:
            raise ValueError("unknown repository name")
        rid = repo[0]
        modules = {
            module: (path, bool(is_test))
            for module, path, is_test in db.execute(
                "SELECT name,path,is_test FROM module WHERE repo_id=? AND commit_oid=?",
                (rid, revision),
            )
        }
        if seed not in modules:
            raise ValueError("unknown indexed seed module")
        forward: dict[str, set[str]] = {m: set() for m in modules}
        backward: dict[str, set[str]] = {m: set() for m in modules}
        for source, target in db.execute(
            "SELECT source_name,target_name FROM dependency WHERE repo_id=? AND commit_oid=?",
            (rid, revision),
        ):
            if source not in modules or target not in modules:
                raise ValueError("edge references unknown module")
            forward[source].add(target)
            backward[target].add(source)

    def distances(edges: dict[str, set[str]]) -> dict[str, int]:
        found = {seed: 0}
        queue = deque([seed])
        while queue:
            node = queue.popleft()
            for neighbor in sorted(edges[node]):
                if neighbor not in found:
                    found[neighbor] = found[node] + 1
                    queue.append(neighbor)
        return found

    if kind == "focus":
        return [modules[seed][0]]
    if kind == "tests":
        scope = set(distances(forward)) | set(distances(backward))
        scope.discard(seed)
        return sorted({modules[m][0] for m in scope if modules[m][1]})
    forward_dir = kind.startswith("dependencies")
    scope = distances(forward if forward_dir else backward)
    direct = kind.endswith("_direct")
    return sorted({modules[m][0] for m, depth in scope.items() if depth > 0 and (not direct or depth == 1)})


def view_paths(manifest: dict[str, Any], revision: str, seed: str, kind: str) -> list[str]:
    if manifest.get("schema") != "excession.repository-view.v1" or manifest.get("kind") != "dependencies":
        raise ValueError("not a dependency-view manifest")
    if manifest.get("source_commit") != revision or manifest.get("focus") != seed:
        raise ValueError("view belongs to a different revision or focus")
    entries = manifest.get("entries")
    if not isinstance(entries, list):
        raise ValueError("manifest entries missing")
    allowed = ROLES[kind]
    paths: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError("invalid entry")
        if entry.get("role") in allowed:
            path = entry.get("source_path")
            if not isinstance(path, str) or not path:
                raise ValueError("invalid source path")
            paths.add(path)
    return sorted(paths)


def output_counts(data: dict[str, Any], arm: str, kind: str) -> dict[str, int]:
    """Count emitted references before postprocessing or deduplication.

    A view emits its entire multi-role manifest even for a focused question.
    The number of selected files is not its raw response size.
    """
    if arm == "graph":
        emitted = data["source_paths"]
        if not isinstance(emitted, list) or not all(isinstance(p, str) for p in emitted):
            raise ValueError("invalid graph output")
        selected = emitted
    elif arm == "view":
        entries = data["entries"]
        if not isinstance(entries, list):
            raise ValueError("invalid view entries")
        emitted = [item["source_path"] for item in entries]
        selected = [item["source_path"] for item in entries if item["role"] in ROLES[kind]]
        if not all(isinstance(p, str) for p in emitted):
            raise ValueError("invalid view source path")
    else:
        raise ValueError("unknown arm")
    return {
        "emitted_references": len(emitted),
        "emitted_unique_files": len(set(emitted)),
        "emitted_duplicate_references": len(emitted) - len(set(emitted)),
        "selected_references": len(selected),
        "selected_duplicate_references": len(selected) - len(set(selected)),
    }


def nearest_rank_p95(data: list[float]) -> float:
    """Observed p95; no interpolation or invented sampling precision."""
    ordered = sorted(data)
    return ordered[math.ceil(len(ordered) * 0.95) - 1]


def git_value(repo: Path, *args: str) -> str:
    out, _ = checked(["git", "-C", str(repo), *args], 20)
    return out.decode("utf-8").strip()


def source_bytes(repo: Path, revision: str, paths: list[str], cache: dict[str, int]) -> int:
    total = 0
    for path in paths:
        if path not in cache:
            # A path comes from a source-bound manifest or indexed module table.
            out, _ = checked(["git", "-C", str(repo), "cat-file", "-s", f"{revision}:{path}"], 20)
            cache[path] = int(out)
        total += cache[path]
    return total


def run(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve(strict=True)
    binary = Path(args.bin).resolve(strict=True)
    tasks_file = Path(args.tasks).resolve(strict=True)
    tasks = load_tasks(tasks_file)
    output = Path(args.out).resolve()
    if repo == output or repo in output.parents:
        raise ValueError("output must be outside the source checkout")
    output.mkdir(parents=True, exist_ok=False)
    revision = git_value(repo, "rev-parse", "--verify", f"{args.rev}^{{commit}}")
    tree = git_value(repo, "rev-parse", "--verify", f"{revision}^{{tree}}")
    db_path = output / "index.sqlite"
    _, index_cost = checked([
        str(binary), "index", "--repo", str(repo), "--db", str(db_path),
        "--name", args.name, "--rev", revision, "--roots", args.roots,
    ], args.timeout)
    # Pin source, index and executable identities before any measured query.
    meta = {
        "schema": SCHEMA, "repository": str(repo), "name": args.name,
        "source_commit": revision, "source_tree": tree, "roots": args.roots,
        "binary_sha256": digest(binary), "index_sha256": digest(db_path),
        "tasks_sha256": digest(tasks_file), "index_cost": index_cost,
        "index_bytes": db_path.stat().st_size, "repetitions": args.repetitions,
        "random_seed": args.random_seed,
        "measurement_notes": "Wall, child CPU, raw stdout bytes and emitted references measured. Selected candidate source bytes are Git blob sizes, not files actually read by an agent. Peak RSS, model tokens and agent outcomes are not measured. Graph worker includes Python process startup.",
    }
    (output / "metadata.json").write_text(json.dumps(meta, indent=2, sort_keys=True) + "\n")
    rng = random.Random(args.random_seed)
    order = [(task, repeat) for repeat in range(args.repetitions) for task in tasks]
    rng.shuffle(order)
    rows: list[dict[str, Any]] = []
    cache: dict[str, int] = {}
    for pair_index, (task, repeat) in enumerate(order):
        pair: dict[str, dict[str, Any]] = {}
        for arm in (("graph", "view") if (repeat + pair_index) % 2 == 0 else ("view", "graph")):
            command_line = ([
                sys.executable, str(Path(__file__).resolve()), "graph-query",
                "--db", str(db_path), "--name", args.name,
                "--revision", revision, "--seed", task["seed"], "--kind", task["kind"],
            ] if arm == "graph" else [
                str(binary), "view", "deps", "--db", str(db_path),
                "--name", args.name, "--rev", revision, "--seed", task["seed"],
            ])
            proc, timing = command(command_line, args.timeout)
            stdout = proc.stdout
            record: dict[str, Any] = {
                "task_id": task["id"], "kind": task["kind"], "seed": task["seed"],
                "repeat": repeat, "arm": arm, "returncode": proc.returncode,
                "raw_stdout_bytes": len(stdout),
                "raw_stdout_sha256": hashlib.sha256(stdout).hexdigest(),
                **timing,
            }
            if proc.returncode:
                record["status"] = "timeout" if proc.returncode == 124 else "refused"
                record["error"] = proc.stderr.decode("utf-8", errors="replace")[:1500]
            else:
                try:
                    data = json.loads(stdout)
                    paths = data["source_paths"] if arm == "graph" else view_paths(
                        data, revision, task["seed"], task["kind"]
                    )
                    if not isinstance(paths, list) or not all(isinstance(p, str) for p in paths):
                        raise ValueError("invalid query output")
                    paths = sorted(set(paths))
                    expected = task.get("expected_source_paths")
                    canonical = json.dumps({"source_paths": paths}, sort_keys=True, separators=(",", ":")).encode("utf-8")
                    counts = output_counts(data, arm, task["kind"])
                    record.update({
                        "status": "ok", "source_paths": paths,
                        "selected_unique_files": len(paths),
                        "empty_selected": len(paths) == 0,
                        "candidate_source_bytes": source_bytes(repo, revision, paths, cache),
                        "declared_paths_equal": None if expected is None else paths == sorted(expected),
                        "canonical_selected_output_bytes": len(canonical),
                        **counts,
                    })
                except (ValueError, KeyError, TypeError, RuntimeError) as exc:
                    record["status"] = "invalid-output"
                    record["error"] = str(exc)[:1500]
            rows.append(record)
            pair[arm] = record
        assert set(pair) == {"graph", "view"}
        parity = (
            pair["graph"]["source_paths"] == pair["view"]["source_paths"]
            if all(pair[arm]["status"] == "ok" for arm in ("graph", "view"))
            else None
        )
        for arm in ("graph", "view"):
            pair[arm]["paired_source_parity"] = parity
    with (output / "trials.jsonl").open("w", encoding="utf-8") as fp:
        for record in rows:
            fp.write(json.dumps(record, sort_keys=True) + "\n")
    grouped = {arm: [r for r in rows if r["arm"] == arm] for arm in ("graph", "view")}
    successful = {arm: [r for r in records if r["status"] == "ok"] for arm, records in grouped.items()}

    def median(arm: str, field: str) -> float | None:
        records = successful[arm]
        return round(statistics.median(r[field] for r in records), 3) if records else None

    def p95(arm: str, field: str) -> float | None:
        records = successful[arm]
        return round(nearest_rank_p95([r[field] for r in records]), 3) if records else None

    summary = {
        "schema": SCHEMA, "tasks": len(tasks), "repetitions": args.repetitions,
        "source_parity_pairs": sum(r["paired_source_parity"] is True for r in grouped["graph"]),
        "parity_eligible_pairs": sum(r["paired_source_parity"] is not None for r in grouped["graph"]),
        "total_pairs": len(grouped["graph"]),
        "counts_by_status": {
            arm: {state: sum(r["status"] == state for r in records)
                  for state in ("ok", "refused", "timeout", "invalid-output")}
            for arm, records in grouped.items()
        },
        "empty_selected_trials": {
            arm: sum(r.get("empty_selected", False) for r in records)
            for arm, records in grouped.items()
        },
        "declared_path_tasks": sum(t.get("expected_source_paths") is not None for t in tasks),
        "declared_path_matches": {
            arm: sum(r.get("declared_paths_equal") is True for r in records)
            for arm, records in grouped.items()
        },
        "median_wall_ms": {arm: median(arm, "wall_ms") for arm in grouped},
        "p95_wall_ms": {arm: p95(arm, "wall_ms") for arm in grouped},
        "median_child_cpu_ms": {
            arm: round(statistics.median(r["child_user_cpu_ms"] + r["child_system_cpu_ms"] for r in records), 3)
            if records else None for arm, records in successful.items()
        },
        "median_raw_stdout_bytes": {arm: median(arm, "raw_stdout_bytes") for arm in grouped},
        "median_canonical_selected_output_bytes": {
            arm: median(arm, "canonical_selected_output_bytes") for arm in grouped
        },
        "median_emitted_references": {arm: median(arm, "emitted_references") for arm in grouped},
        "median_duplicate_references": {arm: median(arm, "emitted_duplicate_references") for arm in grouped},
        "median_candidate_source_bytes": {arm: median(arm, "candidate_source_bytes") for arm in grouped},
        "amortized_wall_ms_per_query": {
            arm: {str(n): round(index_cost["wall_ms"] / n + statistics.mean(r["wall_ms"] for r in records), 3)
                  for n in (1, 5, 20, 100)} if records else None
            for arm, records in successful.items()
        },
        "conclusion": "Calibration only: path parity, emitted output, query CPU/wall cost and shared indexing cost are measured. No task-completion or agent-efficiency inference.",
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest="action", required=True)
    q = subs.add_parser("graph-query", help="Direct graph-only baseline worker")
    for key in ("db", "name", "revision", "seed", "kind"):
        q.add_argument("--" + key, required=True)
    r = subs.add_parser("run", help="Index once, execute paired queries and persist receipts")
    for key in ("repo", "bin", "tasks", "out", "name", "roots"):
        r.add_argument("--" + key, required=True)
    r.add_argument("--rev", default="HEAD")
    r.add_argument("--repetitions", type=int, default=3)
    r.add_argument("--random-seed", type=int, default=20261110)
    r.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    if args.action == "graph-query":
        paths = graph_paths(Path(args.db), args.name, args.revision, args.seed, args.kind)
        print(json.dumps({"source_paths": paths}, sort_keys=True))
    else:
        if args.repetitions < 1 or args.timeout <= 0:
            parser.error("repetitions and timeout must be positive")
        run(args)


if __name__ == "__main__":
    main()
