#!/usr/bin/env python3
"""Measure authenticated view phases on pinned Git fixtures of varying sizes.

All profiles use the production dependency-plan path and must match an
independently requested full view_id. Timings are wall-clock observations, not
algorithmic bounds or proof of parser completeness.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import random
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "core/fixtures/h11-crossrepo.bundle"
SCALES = (32, 128, 512)
REPETITIONS = 12
PHASES = (
    "resolve_revision", "check_git_tree", "reconstruct_index_tree",
    "load_modules", "load_dependencies", "traverse_and_build_entries",
    "validate_index_and_plan", "validate_plan", "reuse_indexed_tree",
    "validate_source_bindings", "canonical_view_id",
)


def invoke(argv: list[str], *, env: dict[str, str] | None = None, limit: int = 120) -> bytes:
    proc = subprocess.run(argv, capture_output=True, check=False, timeout=limit, env=env)
    if proc.returncode:
        raise RuntimeError(
            f"{argv[:5]!r} exited {proc.returncode}: "
            + proc.stderr.decode("utf-8", "replace")[:1800]
        )
    return proc.stdout


def commit_env() -> dict[str, str]:
    env = dict(os.environ)
    env.update({
        "GIT_AUTHOR_NAME": "Excession benchmark",
        "GIT_AUTHOR_EMAIL": "benchmark@example.invalid",
        "GIT_COMMITTER_NAME": "Excession benchmark",
        "GIT_COMMITTER_EMAIL": "benchmark@example.invalid",
        "GIT_AUTHOR_DATE": "2020-01-01T00:00:00+0000",
        "GIT_COMMITTER_DATE": "2020-01-01T00:00:00+0000",
    })
    return env


def synthetic_repo(folder: Path, size: int) -> tuple[str, list[str]]:
    folder.mkdir()
    invoke(["git", "init", "-q", str(folder)])
    package = folder / "bench"
    package.mkdir()
    (package / "__init__.py").write_text('"""Deterministic benchmark modules."""\n')
    for i in range(size):
        imports = f"import bench.m{i + 1:04d}\n" if i + 1 < size else ""
        source = imports + f"\ndef f{i:04d}():\n    return {i}\n"
        (package / f"m{i:04d}.py").write_text(source)
    invoke(["git", "-C", str(folder), "add", "bench"])
    invoke(["git", "-C", str(folder), "commit", "-q", "-m", "Pinned synthetic phase workload"], env=commit_env())
    commit = invoke(["git", "-C", str(folder), "rev-parse", "HEAD"]).decode().strip()
    return commit, ["bench.m0000", f"bench.m{size - 1:04d}"]


def prepare(binary: Path, workdir: Path, kind: str, size: int | None) -> dict:
    repo = workdir / kind
    if size is None:
        invoke(["git", "clone", "-q", str(BUNDLE), str(repo)])
        commit = invoke(["git", "-C", str(repo), "rev-parse", "HEAD"]).decode().strip()
        seeds = ["h11._connection", "h11._headers", "h11._events", "h11._util"]
        root = "h11,tests"
    else:
        commit, seeds = synthetic_repo(repo, size)
        root = "bench"

    tree = invoke(["git", "-C", str(repo), "rev-parse", "HEAD^{tree}"]).decode().strip()
    db = workdir / (kind + ".sqlite")
    t0 = time.perf_counter_ns()
    invoke([
        str(binary), "index", "--repo", str(repo), "--db", str(db),
        "--name", kind, "--rev", commit, "--roots", root,
    ], limit=240)
    setup_ms = (time.perf_counter_ns() - t0) / 1_000_000
    with sqlite3.connect(db) as conn:
        modules = conn.execute("SELECT count(*) FROM module").fetchone()[0]
        edges = conn.execute("SELECT count(*) FROM dependency").fetchone()[0]
        tree_entries = conn.execute("SELECT count(*) FROM tree_entry").fetchone()[0]
    if size is not None and (modules != size + 1 or edges != size - 1):
        raise ValueError(
            f"synthetic fixture parser coverage mismatch: "
            f"modules={modules}, edges={edges}, expected={size + 1}/{size - 1}"
        )
    view_ids = {}
    entry_counts = {}
    for seed in seeds:
        raw = invoke([
            str(binary), "view", "deps", "--db", str(db), "--name", kind,
            "--rev", commit, "--seed", seed,
        ])
        manifest = json.loads(raw)
        if manifest["source_commit"] != commit or manifest["source_tree"] != tree:
            raise ValueError("full manifest source identity mismatch")
        view_ids[seed] = manifest["view_id"]
        entry_counts[seed] = len(manifest["entries"])

    return {
        "name": kind, "repo": str(repo), "db": str(db), "commit": commit, "tree": tree,
        "size": size, "seeds": seeds, "view_ids": view_ids, "entry_counts": entry_counts,
        "module_rows": modules, "dependency_rows": edges, "tree_entries": tree_entries,
        "index_ms": round(setup_ms, 3), "index_bytes": db.stat().st_size,
    }


def sample(binary: Path, task: dict, seed: str, repetition: int) -> dict:
    command = [
        str(binary), "view", "profile-deps", "--db", task["db"],
        "--name", task["name"], "--rev", task["commit"], "--seed", seed,
    ]
    start = time.perf_counter_ns()
    raw = invoke(command)
    cli_ms = (time.perf_counter_ns() - start) / 1_000_000
    profile = json.loads(raw)
    if (profile["view_id"] != task["view_ids"][seed]
            or profile["source_commit"] != task["commit"]
            or profile["source_tree"] != task["tree"]
            or profile["entry_count"] != task["entry_counts"][seed]):
        raise ValueError(f"profile changed authenticated view identity for {task['name']}/{seed}")
    phases = profile["phases"]
    if [item["phase"] for item in phases] != list(PHASES):
        raise ValueError("unexpected or missing phase profile labels")
    phase_times = {}
    for item in phases:
        value = item["elapsed_us"]
        if not isinstance(value, int) or value < 0:
            raise ValueError("invalid phase timing")
        phase_times[item["phase"]] = value
    if sum(phase_times.values()) > profile["total_us"]:
        raise ValueError("profile phases exceed measured total")
    return {
        "workload": task["name"], "seed": seed, "repetition": repetition,
        "entry_count": profile["entry_count"],
        "cli_wall_ms": round(cli_ms, 3),
        "profile_total_us": profile["total_us"],
        "phase_us": phase_times,
        "stdout_sha256": hashlib.sha256(raw).hexdigest(),
    }


def percentile(vals: list[int | float]) -> dict:
    ordered = sorted(vals)
    return {
        "median": round(statistics.median(ordered), 3),
        "observed_nearest_rank_p95": round(ordered[math.ceil(0.95 * len(ordered)) - 1], 3),
    }


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: projection_phases.py RELEASE_BIN OUTPUT_JSON")
    binary = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()

    with tempfile.TemporaryDirectory(prefix="excession-phases-") as tmp:
        directory = Path(tmp)
        workloads = [prepare(binary, directory, "h11", None)]
        workloads.extend(
            prepare(binary, directory, f"synthetic_{size}", size) for size in SCALES
        )

        tasks = [
            (workload, seed, repetition)
            for workload in workloads
            for seed in workload["seeds"]
            for repetition in range(REPETITIONS)
        ]
        random.Random(20261111).shuffle(tasks)
        samples = [sample(binary, workload, seed, repetition)
                   for workload, seed, repetition in tasks]

        metadata = [
            {key: value for key, value in task.items()
             if key not in ("repo", "db", "view_ids")}
            for task in workloads
        ]

    grouped: dict[str, list[dict]] = defaultdict(list)
    for item in samples:
        grouped[item["workload"]].append(item)

    summary = {}
    for name, records in grouped.items():
        phases = {
            phase: percentile([record["phase_us"][phase] for record in records])
            for phase in PHASES
        }
        summary[name] = {
            "sample_count": len(records),
            "profile_total_us": percentile([record["profile_total_us"] for record in records]),
            "cli_wall_ms": percentile([record["cli_wall_ms"] for record in records]),
            "phases": phases,
            "largest_median_phase": max(PHASES, key=lambda p: phases[p]["median"]),
        }

    receipt = {
        "schema": "excession.projection-phase-benchmark.v1",
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "h11_bundle_sha256": hashlib.sha256(BUNDLE.read_bytes()).hexdigest(),
        "repetitions": REPETITIONS,
        "workloads": metadata,
        "summary": summary,
        "samples": samples,
        "limitations": (
            "Phase timings use monotonic process-local wall time and include measurement overhead. "
            "The synthetic inputs are deterministic Git/Python module graphs, not independent "
            "real-world programs. This is observation of one release build on one GitHub runner, "
            "not an asymptotic bound, causal attribution, or a measure of security guarantees."
        ),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(json.dumps({
        "schema": receipt["schema"],
        "workloads": metadata,
        "summary": summary,
        "total_samples": len(samples),
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
