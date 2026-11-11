#!/usr/bin/env python3
"""Controlled A/B: full Git scan versus checked-index reuse in Excession.

Runs the *same* pinned queries against two native binaries and one index.
Requires byte-identical authenticated outputs and refusal parity before
reporting timings. Not a proof of improved task outcomes or correctness.
"""
from __future__ import annotations

import hashlib
import json
import math
import random
import resource
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from pathlib import Path

# The phase generator remains a microbenchmark dependency, not a second copy.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "micro"))
from projection_phases import prepare

BASELINE = "fa1d15248210ee6ec2a4c744603b271d613fa7b5"
REPETITIONS = 12
KINDS = ("deps", "paths")
ROLES = ("focus", "direct-dependencies", "transitive-dependencies", "indexed-test")
SIZES = (32, 128, 512)


def invoke(argv: list[str], timeout: int = 120) -> tuple[subprocess.CompletedProcess[bytes], float, float]:
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    before = usage.ru_utime + usage.ru_stime
    started = time.perf_counter_ns()
    proc = subprocess.run(argv, capture_output=True, check=False, timeout=timeout)
    wall = (time.perf_counter_ns() - started) / 1_000_000
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = (after.ru_utime + after.ru_stime - before) * 1000
    return proc, round(wall, 3), round(cpu, 3)


def query(binary: Path, task: dict, seed: str, kind: str, role: str) -> list[str]:
    argv = [
        str(binary), "view", kind, "--db", task["db"], "--name", task["name"],
        "--rev", task["commit"], "--seed", seed,
    ]
    if kind == "paths":
        argv += ["--role", role]
    return argv


def checked_call(argv: list[str]) -> bytes:
    proc, _, _ = invoke(argv)
    if proc.returncode:
        raise RuntimeError(
            f"{argv[:4]!r} failed ({proc.returncode}): "
            f"{proc.stderr.decode('utf-8', 'replace')[:1200]}"
        )
    return proc.stdout


def check_view(raw: bytes, task: dict, seed: str, kind: str, role: str) -> None:
    response = json.loads(raw)
    if response["source_commit"] != task["commit"] or response["source_tree"] != task["tree"]:
        raise ValueError("query returned wrong source revision or tree")
    if response["view_id"] != task["view_ids"][seed]:
        raise ValueError("query returned changed canonical view identity")
    if kind == "deps":
        if response["schema"] != "excession.repository-view.v1":
            raise ValueError("wrong full manifest schema")
        if len(response["entries"]) != task["entry_counts"][seed]:
            raise ValueError("full view entry count changed")
    else:
        if response["schema"] != "excession.dependency-paths.v1" or response["role"] != role:
            raise ValueError("wrong compact query schema or role")
        if "entries" in response:
            raise ValueError("compact view unexpectedly emitted full manifest")
        if response["source_paths"] != sorted(set(response["source_paths"])):
            raise ValueError("compact query paths not unique and sorted")


def percentile(values: list[float]) -> dict[str, float]:
    values = sorted(values)
    return {
        "median": round(statistics.median(values), 3),
        "observed_nearest_rank_p95": round(values[math.ceil(len(values) * 0.95) - 1], 3),
    }


def tamper_refusals(binary_a: Path, binary_b: Path, task: dict, work: Path) -> list[dict]:
    """Replace claims with other valid indexed objects where possible.

    The dependency-target case tests a missing reference, not authenticity
    of plausible semantic edges, which Git tree identity cannot establish.
    """
    commit = task["commit"]
    source = Path(task["db"])
    cases = [
        ("module_blob_claim",
         """UPDATE module SET blob_oid=(
                SELECT blob_oid FROM module WHERE name='h11._headers' AND commit_oid=?
            ) WHERE name='h11._connection' AND commit_oid=?""",
         (commit, commit)),
        ("tree_entry_oid",
         """UPDATE tree_entry SET oid=(
                SELECT oid FROM tree_entry
                WHERE path='h11/_headers.py' AND commit_oid=?
            ) WHERE path='h11/_connection.py' AND commit_oid=?""",
         (commit, commit)),
        ("revision_root_tree",
         """UPDATE revision SET tree_oid=(
                SELECT oid FROM tree_entry
                WHERE kind='tree' AND commit_oid=? LIMIT 1
            ) WHERE commit_oid=?""",
         (commit, commit)),
        ("missing_dependency_target",
         "UPDATE dependency SET target_name=? WHERE source_name='h11._connection' AND commit_oid=?",
         ("missing.module", commit)),
    ]
    outcomes = []
    for index, (name, sql, params) in enumerate(cases):
        destination = work / f"corrupt_{index}.sqlite"
        with sqlite3.connect(source) as original, sqlite3.connect(destination) as clone:
            original.backup(clone)
        with sqlite3.connect(destination) as connection:
            connection.execute("PRAGMA foreign_keys=OFF")
            affected = connection.execute(sql, params)
            if affected.rowcount <= 0:
                raise ValueError(f"tamper fixture had no matching rows: {name}")
            connection.commit()
        observed: dict[str, int | str] = {"kind": name}
        for label, binary in (("baseline", binary_a), ("optimized", binary_b)):
            for query_kind in KINDS:
                args = query(binary, {**task, "db": str(destination)},
                             "h11._connection", query_kind, "focus")
                proc, _, _ = invoke(args)
                if proc.returncode == 0:
                    raise AssertionError(f"{label} accepted corrupted source data: {name}/{query_kind}")
                observed[f"{label}_{query_kind}_exit"] = proc.returncode
        outcomes.append(observed)
    return outcomes


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: checked_tree_ab.py BASELINE_BIN OPTIMIZED_BIN RESULT_JSON")
    baseline = Path(sys.argv[1]).resolve()
    optimized = Path(sys.argv[2]).resolve()
    destination = Path(sys.argv[3]).resolve()
    if baseline == optimized or not baseline.is_file() or not optimized.is_file():
        raise ValueError("two distinct, existing native binaries required")

    with tempfile.TemporaryDirectory(prefix="excession-checked-tree-ab-") as tmp:
        work = Path(tmp)
        # Generate the catalog once, using the baseline program. Both query
        # implementations read exactly the same Git and SQLite objects.
        workloads = [prepare(baseline, work, "h11", None)]
        workloads.extend(
            prepare(baseline, work, f"synthetic_{size}", size) for size in SIZES
        )

        queries = [
            (task, seed, kind, role)
            for task in workloads
            for seed in task["seeds"]
            for kind in KINDS
            for role in (("all",) if kind == "deps" else ROLES)
        ]
        # Prove byte-for-byte identity before collecting performance numbers.
        reference: dict[tuple[str, str, str, str], bytes] = {}
        for task, seed, kind, role in queries:
            args_a = query(baseline, task, seed, kind, role)
            args_b = query(optimized, task, seed, kind, role)
            raw_a = checked_call(args_a)
            raw_b = checked_call(args_b)
            check_view(raw_a, task, seed, kind, role)
            check_view(raw_b, task, seed, kind, role)
            if raw_a != raw_b:
                raise AssertionError(f"byte-different authenticated outputs: {task['name']}/{seed}/{kind}/{role}")
            reference[(task["name"], seed, kind, role)] = raw_a

        refusals = tamper_refusals(baseline, optimized, workloads[0], work)
        order = [
            (task, seed, kind, role, rep)
            for task, seed, kind, role in queries
            for rep in range(REPETITIONS)
        ]
        rng = random.Random(20261111)
        rng.shuffle(order)
        samples = []
        for pair_index, (task, seed, kind, role, repetition) in enumerate(order):
            arms = ("baseline", "optimized") if pair_index % 2 == 0 else ("optimized", "baseline")
            for arm in arms:
                binpath = baseline if arm == "baseline" else optimized
                stdout, wall, cpu = invoke(query(binpath, task, seed, kind, role))
                if stdout.returncode:
                    raise RuntimeError(
                        f"{arm} query refused during timing: "
                        + stdout.stderr.decode("utf-8", "replace")[:1200]
                    )
                key = (task["name"], seed, kind, role)
                if stdout.stdout != reference[key]:
                    raise AssertionError(f"{arm} changed authenticated output during timing: {key}")
                samples.append({
                    "workload": task["name"], "seed": seed, "kind": kind,
                    "role": role, "repetition": repetition, "arm": arm,
                    "wall_ms": wall, "child_cpu_ms": cpu,
                    "stdout_bytes": len(stdout.stdout),
                })

        metadata = [
            {key: value for key, value in task.items()
             if key not in ("repo", "db", "view_ids")}
            for task in workloads
        ]

    grouped: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for row in samples:
        grouped[(row["workload"], row["kind"])].append(row)
    summary = {}
    for (name, kind), group in sorted(grouped.items()):
        by_arm = {arm: [v for v in group if v["arm"] == arm]
                  for arm in ("baseline", "optimized")}
        if len(by_arm["baseline"]) != len(by_arm["optimized"]):
            raise AssertionError("unpaired A/B sample counts")
        summary[f"{name}/{kind}"] = {
            "pairs": len(by_arm["baseline"]),
            "baseline": {
                "wall_ms": percentile([s["wall_ms"] for s in by_arm["baseline"]]),
                "cpu_ms": percentile([s["child_cpu_ms"] for s in by_arm["baseline"]]),
            },
            "optimized": {
                "wall_ms": percentile([s["wall_ms"] for s in by_arm["optimized"]]),
                "cpu_ms": percentile([s["child_cpu_ms"] for s in by_arm["optimized"]]),
            },
            "wall_speedup_median_ratio": round(
                statistics.median(s["wall_ms"] for s in by_arm["baseline"])
                / statistics.median(s["wall_ms"] for s in by_arm["optimized"]), 3
            ),
        }
    payload = {
        "schema": "excession.checked-tree-ab.v1",
        "baseline_source_commit": BASELINE,
        "baseline_binary_sha256": hashlib.sha256(baseline.read_bytes()).hexdigest(),
        "optimized_binary_sha256": hashlib.sha256(optimized.read_bytes()).hexdigest(),
        "query_variants": len(queries),
        "repetitions": REPETITIONS,
        "output_parity_checks": len(queries),
        "tamper_refusals": refusals,
        "workloads": metadata,
        "summary": summary,
        "samples": samples,
        "limitations": (
            "Two native release binaries running interleaved on one CI host, "
            "using a shared index created by the baseline implementation. "
            "The builds differ by commits besides checked-tree reuse, and the "
            "synthetic chain is not representative of real repositories. "
            "The timings are descriptive observations, not security proofs, "
            "complexity bounds or a substitute for adversarial review."
        ),
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    print(json.dumps({
        k: value for k, value in payload.items() if k not in ("samples",)
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
