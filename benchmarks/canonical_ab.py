#!/usr/bin/env python3
"""Compare canonical manifest bytes, hash cost and CLI latency before/after.

Two pinned native builds, one Git/SQLite dataset per workload, identical
full and compact JSON outputs. Historical byte equality is required.
"""
from __future__ import annotations

import hashlib
import json
import random
import statistics
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

# The benchmark program now keeps its reusable controls under experiments/
# and synthetic fixture generator under micro/. Use those originals directly.
ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / "experiments"))
sys.path.insert(0, str(ROOT / "micro"))
from checked_tree_ab import check_view, invoke, percentile, query
from projection_phases import prepare

BASELINE_SHA = "2a7080f3404b42a9f90357e91d1a40af1ead7e3e"
SCALES = (32, 128, 512, 1024)
REPETITIONS = 10
ROLES = ("focus", "direct-dependencies", "indexed-test")


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: canonical_ab.py BASELINE_BIN OPTIMIZED_BIN JSON_RECEIPT")
    before, after, destination = [Path(x).resolve() for x in sys.argv[1:]]
    if before == after or not before.is_file() or not after.is_file():
        raise ValueError("two distinct compiled native binaries required")

    with tempfile.TemporaryDirectory(prefix="excession-canonical-ab-") as tmp:
        directory = Path(tmp)
        workloads = [prepare(before, directory, "h11", None)]
        workloads.extend(
            prepare(before, directory, f"synthetic_{size}", size) for size in SCALES
        )

        variants = [
            (task, seed, kind, role)
            for task in workloads for seed in task["seeds"]
            for kind in ("deps", "paths")
            for role in (("all",) if kind == "deps" else ROLES)
        ]
        expected = {}
        for task, seed, kind, role in variants:
            argv_a = query(before, task, seed, kind, role)
            argv_b = query(after, task, seed, kind, role)
            proc_a, _, _ = invoke(argv_a)
            proc_b, _, _ = invoke(argv_b)
            if proc_a.returncode or proc_b.returncode:
                raise RuntimeError(f"query refused for {task['name']}/{seed}/{kind}/{role}")
            check_view(proc_a.stdout, task, seed, kind, role)
            check_view(proc_b.stdout, task, seed, kind, role)
            if proc_a.stdout != proc_b.stdout:
                raise AssertionError(f"canonical JSON bytes changed: {task['name']}/{seed}/{kind}/{role}")
            expected[(task["name"], seed, kind, role)] = proc_a.stdout

        randomizer = random.Random(20261111)
        order = [
            (task, seed, kind, role, iteration)
            for task, seed, kind, role in variants
            for iteration in range(REPETITIONS)
        ]
        randomizer.shuffle(order)
        results = []
        for position, (task, seed, kind, role, iteration) in enumerate(order):
            arms = ("before", "after") if position % 2 == 0 else ("after", "before")
            for arm in arms:
                binary = before if arm == "before" else after
                proc, wall, cpu = invoke(query(binary, task, seed, kind, role))
                if proc.returncode or proc.stdout != expected[(task["name"], seed, kind, role)]:
                    raise AssertionError(f"{arm} changed authenticated JSON output on repeated query")
                results.append({
                    "workload": task["name"], "kind": kind, "seed": seed, "role": role,
                    "iteration": iteration, "arm": arm, "wall_ms": wall, "cpu_ms": cpu,
                    "output_bytes": len(proc.stdout),
                })

        # Separately time the existing per-command canonical hash phase.
        # These are process-internal observations with profiler overhead.
        profile_results = []
        for task in workloads:
            for seed in task["seeds"]:
                for repetition in range(REPETITIONS):
                    arms = ("before", "after") if repetition % 2 == 0 else ("after", "before")
                    for arm in arms:
                        binary = before if arm == "before" else after
                        argv = [
                            str(binary), "view", "profile-deps", "--db", task["db"],
                            "--name", task["name"], "--rev", task["commit"], "--seed", seed,
                        ]
                        proc, _, _ = invoke(argv)
                        if proc.returncode:
                            raise RuntimeError(f"{arm} profile refused: {proc.stderr[:300]!r}")
                        response = json.loads(proc.stdout)
                        if response["view_id"] != task["view_ids"][seed]:
                            raise AssertionError("canonical view identity changed in profile")
                        phases = {p["phase"]: p["elapsed_us"] for p in response["phases"]}
                        if "canonical_view_id" not in phases:
                            raise AssertionError("missing canonical hash phase")
                        profile_results.append({
                            "workload": task["name"], "seed": seed, "arm": arm,
                            "repetition": repetition, "canonical_us": phases["canonical_view_id"],
                            "total_us": response["total_us"], "entries": response["entry_count"],
                        })

        metadata = [
            {key: value for key, value in task.items()
             if key not in ("repo", "db", "view_ids")}
            for task in workloads
        ]

    grouped: dict[tuple[str, str, str], list[float]] = defaultdict(list)
    for sample in results:
        grouped[(sample["workload"], sample["kind"], sample["arm"])].append(sample["wall_ms"])
    grouped_hash: dict[tuple[str, str], list[int]] = defaultdict(list)
    for sample in profile_results:
        grouped_hash[(sample["workload"], sample["arm"])].append(sample["canonical_us"])
    summary = {}
    for task in workloads:
        label = task["name"]
        summary[label] = {}
        for kind in ("deps", "paths"):
            old = percentile(grouped[(label, kind, "before")])
            new = percentile(grouped[(label, kind, "after")])
            summary[label][kind] = {
                "before_wall_ms": old, "after_wall_ms": new,
                "median_speedup_ratio": round(old["median"] / new["median"], 3),
            }
        old_hash = percentile(grouped_hash[(label, "before")])
        new_hash = percentile(grouped_hash[(label, "after")])
        summary[label]["canonical_view_id"] = {
            "before_us": old_hash, "after_us": new_hash,
            "median_speedup_ratio": round(old_hash["median"] / new_hash["median"], 3),
        }

    result = {
        "schema": "excession.canonical-json-ab.v1",
        "baseline_commit": BASELINE_SHA,
        "baseline_binary_sha256": hashlib.sha256(before.read_bytes()).hexdigest(),
        "optimized_binary_sha256": hashlib.sha256(after.read_bytes()).hexdigest(),
        "query_variants_checked": len(variants),
        "timed_query_pairs": len(order),
        "profile_pairs": len(profile_results) // 2,
        "repetitions": REPETITIONS,
        "workloads": metadata,
        "summary": summary,
        "samples": results,
        "profile_samples": profile_results,
        "limitations": (
            "Identical Git, index and pinned Rust release toolchain, but process startup remains "
            "in wall time. This compares two commits, not isolated single-instruction costs. "
            "The canonical hash phase measures both JSON creation and SHA-256, not just hashing. "
            "Synthetic Python chains are not representative real repositories; no allocation "
            "counts, memory peaks, external correctness proof or asymptotic bound are claimed."
        ),
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({key: result[key] for key in (
        "schema", "query_variants_checked", "timed_query_pairs", "profile_pairs", "summary"
    )}, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
