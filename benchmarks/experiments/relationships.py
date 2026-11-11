#!/usr/bin/env python3
"""Independently specified graph relationships across immutable Git revisions.

A deliberately bounded experiment: retrieval exactness, query cost, indexing,
and repeated-query cost. It does not measure agent productivity or graph
completeness outside the generated fixture.
"""
from __future__ import annotations

import argparse
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
import tempfile
import time

KINDS = {
    "focus": "focus",
    "dependencies_direct": "direct-dependencies",
    "dependents_direct": "direct-dependents",
    "tests": "indexed-test",
}
SCHEMA = "excession.relational-retrieval.v1"


def invoke(argv: list[str], timeout: float = 90, env: dict[str, str] | None = None) -> tuple[subprocess.CompletedProcess[bytes], dict]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    t0 = time.perf_counter_ns()
    try:
        result = subprocess.run(argv, check=False, capture_output=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired as exc:
        result = subprocess.CompletedProcess(argv, 124, exc.stdout or b"", exc.stderr or b"timeout")
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    return result, {
        "wall_ms": round((time.perf_counter_ns() - t0) / 1e6, 3),
        "child_cpu_ms": round(1000 * (after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime), 3),
        "stdout_bytes": len(result.stdout),
        "stdout_sha256": hashlib.sha256(result.stdout).hexdigest(),
    }


def checked(argv: list[str], **kwargs) -> tuple[bytes, dict]:
    proc, metrics = invoke(argv, **kwargs)
    if proc.returncode:
        raise RuntimeError(f"command failed ({proc.returncode}): {argv[:5]!r}: {proc.stderr.decode(errors='replace')[:500]}")
    return proc.stdout, metrics


def graph_spec(n: int, fanout: int, revision: int) -> dict[str, set[str]]:
    """Define the oracle *before* parsing or indexing any source."""
    if n < 4 or not (1 <= fanout < n) or revision < 0:
        raise ValueError("invalid graph dimensions")
    names = [f"bench.m{i:04d}" for i in range(n)]
    graph: dict[str, set[str]] = {"bench": set(), "tests": set()}
    for i, name in enumerate(names):
        graph[name] = set(names[i + j] for j in range(1, fanout + 1) if i + j < n)
    if revision:
        pivot = 1 + (revision - 1) % (n - 1)
        graph[names[pivot]].add(names[0])
    graph["tests.test_links"] = {names[0], names[n // 2]}
    return graph


def oracle(graph: dict[str, set[str]], seed: str, kind: str) -> list[str]:
    if seed not in graph or kind not in KINDS:
        raise ValueError("unknown seed or query kind")
    def visit(reverse: bool) -> dict[str, int]:
        relation = ({m: {from_ for from_, targets in graph.items() if m in targets}
                     for m in graph} if reverse else graph)
        found = {seed: 0}
        todo = deque([seed])
        while todo:
            module = todo.popleft()
            for neighbor in sorted(relation[module]):
                if neighbor not in found:
                    found[neighbor] = found[module] + 1
                    todo.append(neighbor)
        return found

    if kind == "focus":
        selected = {seed}
    elif kind == "tests":
        selected = (set(visit(False)) | set(visit(True))) - {seed}
        selected = {m for m in selected if m.startswith("tests.")}
    else:
        d = visit(kind == "dependents_direct")
        selected = {m for m, depth in d.items() if depth == 1}
    return sorted(module_path(name) for name in selected)


def module_path(name: str) -> str:
    if name in ("bench", "tests"):
        return name + "/__init__.py"
    return name.replace(".", "/") + ".py"


def source_for(graph: dict[str, set[str]], name: str) -> str:
    imports = "".join(f"import {target}\n" for target in sorted(graph[name]))
    return imports + f"\nMODULE = {name!r}\n"


def fixture(source: Path, n: int, fanout: int, revisions: int) -> list[dict]:
    source.mkdir()
    checked(["git", "init", "-q", str(source)])
    history = []
    for rev in range(revisions):
        graph = graph_spec(n, fanout, rev)
        for name in sorted(graph):
            path = source / module_path(name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source_for(graph, name), encoding="utf-8")
        when = f"2020-01-{rev + 1:02d}T00:00:00+0000"
        env = dict(os.environ, GIT_AUTHOR_NAME="Benchmark", GIT_COMMITTER_NAME="Benchmark",
                   GIT_AUTHOR_EMAIL="benchmark@example.invalid",
                   GIT_COMMITTER_EMAIL="benchmark@example.invalid",
                   GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
        checked(["git", "-C", str(source), "add", "bench", "tests"])
        checked(["git", "-C", str(source), "commit", "-q", "-m", f"Generated workload revision {rev}"], env=env)
        commit = checked(["git", "-C", str(source), "rev-parse", "HEAD"])[0].decode().strip()
        tree = checked(["git", "-C", str(source), "rev-parse", "HEAD^{tree}"])[0].decode().strip()
        history.append({"commit": commit, "tree": tree, "graph": graph, "revision": rev})
    if checked(["git", "-C", str(source), "status", "--porcelain"])[0]:
        raise RuntimeError("synthetic fixture is dirty")
    return history


def indexed_edges(db: Path, name: str, commit: str) -> set[tuple[str, str]]:
    with sqlite3.connect(db) as conn:
        return set(conn.execute(
            "SELECT d.source_name,d.target_name FROM dependency d "
            "JOIN repository r ON r.id=d.repo_id "
            "WHERE r.name=? AND d.commit_oid=?", (name, commit)
        ))


def run_query(binary: Path, db: Path, name: str, sample: dict, seed: str, kind: str, arm: str) -> dict:
    base = [str(binary), "view"]
    opts = ["--db", str(db), "--name", name, "--rev", sample["commit"], "--seed", seed]
    cmd = base + (["graph", *opts, "--kind", kind] if arm == "graph"
                  else ["paths", *opts, "--role", KINDS[kind]])
    proc, metrics = invoke(cmd)
    record = {
        "workload": name, "revision": sample["revision"], "commit": sample["commit"],
        "seed": seed, "kind": kind, "arm": arm, **metrics,
        "status": "ok" if proc.returncode == 0 else ("timeout" if proc.returncode == 124 else "error"),
        "exit_code": proc.returncode,
    }
    if record["status"] != "ok":
        record["error"] = proc.stderr.decode(errors="replace")[:400]
        return record
    try:
        result = json.loads(proc.stdout)
        paths = result["source_paths"]
        if (result["source_commit"] != sample["commit"]
                or result["source_tree"] != sample["tree"]
                or not isinstance(paths, list) or not all(isinstance(x, str) for x in paths)):
            raise ValueError("invalid source identity or file set")
        normalized = sorted(set(paths))
        expected = oracle(sample["graph"], seed, kind)
        record.update({
            "source_paths": normalized,
            "source_count": len(normalized),
            "exact_oracle_match": normalized == expected,
            "declared_oracle_paths": expected,
            "duplicate_paths": len(paths) - len(normalized),
        })
        if normalized != expected:
            record["status"] = "oracle-mismatch"
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
        record["status"] = "invalid-output"
        record["error"] = str(exc)
    return record


def report(binary: Path, out: Path, sizes: list[int], fanouts: list[int],
           revisions: int, repetitions: int) -> dict:
    if out.exists():
        raise ValueError("output directory must not exist")
    if not sizes or not fanouts or any(n < 4 for n in sizes) or any(f < 1 for f in fanouts):
        raise ValueError("invalid sizes or fanouts")
    if not (1 <= revisions <= 20) or repetitions < 1:
        raise ValueError("invalid revisions or repetitions")
    out.mkdir(parents=True)
    rng = random.Random(20261111)
    rows = []
    workloads = []
    with tempfile.TemporaryDirectory(prefix="excession-relational-") as temp:
        tmp = Path(temp)
        for n in sizes:
            for fanout in fanouts:
                if fanout >= n:
                    raise ValueError("fanout must be less than every size")
                name = f"modules{n}_fanout{fanout}"
                source = tmp / name
                db = tmp / (name + ".sqlite")
                history = fixture(source, n, fanout, revisions)
                bundle = out / (name + ".bundle")
                checked(["git", "-C", str(source), "bundle", "create", str(bundle), "--all"])
                item = {
                    "name": name, "size": n, "fanout": fanout,
                    "source_bundle": bundle.name,
                    "bundle_sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
                    "revisions": [],
                }
                for snapshot in history:
                    _, idx = checked([
                        str(binary), "index", "--repo", str(source), "--db", str(db),
                        "--name", name, "--rev", snapshot["commit"], "--roots", "bench,tests"
                    ])
                    expected_edges = {(src, dst) for src, targets in snapshot["graph"].items() for dst in targets}
                    actual_edges = indexed_edges(db, name, snapshot["commit"])
                    indexed_exact = actual_edges == expected_edges
                    item["revisions"].append({
                        "commit": snapshot["commit"], "tree": snapshot["tree"],
                        "index_ms": idx["wall_ms"], "index_cpu_ms": idx["child_cpu_ms"],
                        "index_bytes_cumulative": db.stat().st_size,
                        "expected_edge_count": len(expected_edges),
                        "indexed_edge_count": len(actual_edges),
                        "independently_specified_edges_equal": indexed_exact,
                        "missing_edges": sorted(expected_edges - actual_edges),
                        "unexpected_edges": sorted(actual_edges - expected_edges),
                    })
                    if not indexed_exact:
                        # Write a diagnostic receipt instead of silently benchmarking a
                        # graph that contradicts the independent workload specification.
                        (out / "coverage_failure.json").write_text(json.dumps(item, indent=2) + "\n")
                        raise ValueError(f"indexed relations disagree with fixture specification: {name} rev {snapshot['revision']}")
                workloads.append(item)
                jobs = [(snap, seed, kind, rep)
                        for snap in history
                        for seed in (f"bench.m0000", f"bench.m{n // 2:04d}", f"bench.m{n - 1:04d}")
                        for kind in KINDS
                        for rep in range(repetitions)]
                rng.shuffle(jobs)
                for snap, seed, kind, rep in jobs:
                    pair = []
                    for arm in (("graph", "view") if (len(rows) // 2) % 2 == 0 else ("view", "graph")):
                        row = run_query(binary, db, name, snap, seed, kind, arm)
                        row["repetition"] = rep
                        rows.append(row)
                        pair.append(row)
                    if all(row["status"] == "ok" for row in pair):
                        if pair[0]["source_paths"] != pair[1]["source_paths"]:
                            raise ValueError("different source paths despite independent oracle")
    with (out / "trials.jsonl").open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, sort_keys=True) + "\n")
    arms = ("graph", "view")
    result = {
        "schema": SCHEMA,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "sizes": sizes, "fanouts": fanouts, "revision_count": revisions,
        "repetitions": repetitions, "workloads": workloads,
        "query_count_per_arm": len(rows) // 2,
        "status_counts": {arm: {state: sum(row["arm"] == arm and row["status"] == state for row in rows)
                               for state in ("ok", "timeout", "error", "invalid-output", "oracle-mismatch")}
                          for arm in arms},
        "median_wall_ms": {arm: round(statistics.median(
            row["wall_ms"] for row in rows if row["arm"] == arm), 3) for arm in arms},
        "median_raw_stdout_bytes": {arm: statistics.median(
            row["stdout_bytes"] for row in rows if row["arm"] == arm) for arm in arms},
        "total_query_wall_ms": {arm: round(sum(row["wall_ms"] for row in rows if row["arm"] == arm), 3) for arm in arms},
        "limitations": ("Specified synthetic Python import graphs only; no task-completion metric. "
                        "Both arms execute one native Rust CLI call per identical query. "
                        "Per-query costs exclude indexing, which is recorded for every revision; "
                        "the graph arm verifies the Git tree but does not authenticate a derived view."),
    }
    (out / "summary.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    # Publish a validated report even on nonpassing observations; do not drop
    # failures from the denominator or replace raw source-bound receipts.
    # The benchmark orchestrates observations; the Rust CLI owns reporting.
    checked([str(binary), "benchmark", "report", "--kind", "relational", "--directory", str(out)])
    print(json.dumps({k: v for k, v in result.items() if k != "workloads"}, indent=2, sort_keys=True))
    if any(row["status"] != "ok" for row in rows):
        raise RuntimeError("relational benchmark had query failures")
    return result


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--bin", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--sizes", default="8,24")
    p.add_argument("--fanouts", default="1,3")
    p.add_argument("--revisions", type=int, default=3)
    p.add_argument("--repetitions", type=int, default=2)
    args = p.parse_args()
    report(Path(args.bin).resolve(strict=True), Path(args.out).resolve(),
           [int(x) for x in args.sizes.split(",")],
           [int(x) for x in args.fanouts.split(",")],
           args.revisions, args.repetitions)


if __name__ == "__main__":
    main()
