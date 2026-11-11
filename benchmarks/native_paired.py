#!/usr/bin/env python3
"""Paired native-source retrieval calibration, not an agent-quality benchmark.

Both arms use the same native Rust binary and pinned, already-indexed Git tree.
The direct graph arm does not create a view; the focused arm authenticates one.
"""
from __future__ import annotations

import hashlib
import json
import math
import random
import resource
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / "core/fixtures/h11-crossrepo.bundle"
SEEDS = ("h11._connection", "h11._headers", "h11._events", "h11._util")
KINDS = {
    "focus": "focus",
    "dependencies_direct": "direct-dependencies",
    "dependents_direct": "direct-dependents",
    "tests": "indexed-test",
}
REPETITIONS = 5


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(argv: list[str], timeout: int = 60) -> tuple[bytes, float, float]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter_ns()
    proc = subprocess.run(argv, capture_output=True, check=False, timeout=timeout)
    elapsed = (time.perf_counter_ns() - start) / 1_000_000
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = 1000 * (
        after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime
    )
    if proc.returncode:
        raise RuntimeError(
            f"command exited {proc.returncode}: {argv[:4]!r}: "
            f"{proc.stderr.decode('utf-8', 'replace')[:2000]}"
        )
    return proc.stdout, round(elapsed, 3), round(cpu, 3)


def summary(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    return {
        "median": round(statistics.median(ordered), 3),
        "nearest_rank_p95": round(ordered[math.ceil(len(ordered) * .95) - 1], 3),
    }


def measure(binary: Path, root: Path, db: Path, commit: str, tree: str) -> dict:
    prefix = [str(binary), "view"]
    fixed = ["--db", str(db), "--name", "h11", "--rev", commit]
    expected: dict[tuple[str, str], list[str]] = {}
    full_bytes: dict[str, int] = {}
    full_ids: dict[str, str] = {}
    for seed in SEEDS:
        view_stdout, _, _ = run(prefix + ["deps", *fixed, "--seed", seed])
        manifest = json.loads(view_stdout)
        if manifest["source_commit"] != commit or manifest["source_tree"] != tree:
            raise ValueError("full view source identity mismatch")
        full_bytes[seed] = len(view_stdout)
        full_ids[seed] = manifest["view_id"]
        for kind, role in KINDS.items():
            expected[(seed, kind)] = sorted({
                item["source_path"] for item in manifest["entries"]
                if item["role"] == role
            })

    order = [(seed, kind, repeat) for seed in SEEDS for kind in KINDS for repeat in range(REPETITIONS)]
    random.Random(20261111).shuffle(order)
    samples: list[dict] = []
    for pair_index, (seed, kind, repeat) in enumerate(order):
        arms = ["graph", "paths"] if pair_index % 2 == 0 else ["paths", "graph"]
        for arm in arms:
            tail = (["graph", *fixed, "--seed", seed, "--kind", kind]
                    if arm == "graph" else
                    ["paths", *fixed, "--seed", seed, "--role", KINDS[kind]])
            command = prefix + tail
            output, wall, cpu = run(command)
            data = json.loads(output)
            if data["source_commit"] != commit or data["source_tree"] != tree:
                raise ValueError(f"{arm} used wrong source identity")
            paths = data["source_paths"]
            if paths != expected[(seed, kind)]:
                raise ValueError(f"{arm} source mismatch at {seed}/{kind}: {paths!r}")
            if arm == "paths" and data["view_id"] != full_ids[seed]:
                raise ValueError("focused result has wrong full-view commitment")
            samples.append({
                "seed": seed, "kind": kind, "repeat": repeat,
                "arm": arm, "wall_ms": wall, "child_cpu_ms": cpu,
                "stdout_bytes": len(output),
                "stdout_sha256": hashlib.sha256(output).hexdigest(),
                "source_paths": paths,
            })

    result = {
        "schema": "excession.native-paired-calibration.v1",
        "source_commit": commit, "source_tree": tree,
        "bundle_sha256": sha256(BUNDLE),
        "binary_sha256": sha256(binary),
        "index_sha256": sha256(db),
        "seed_count": len(SEEDS), "query_kinds": list(KINDS),
        "repetitions": REPETITIONS,
        "paired_queries": len(order), "source_set_matches": len(order),
        "full_view_bytes": full_bytes,
        "summary": {
            arm: {
                "wall_ms": summary([x["wall_ms"] for x in samples if x["arm"] == arm]),
                "child_cpu_ms": summary([x["child_cpu_ms"] for x in samples if x["arm"] == arm]),
                "stdout_bytes": summary([x["stdout_bytes"] for x in samples if x["arm"] == arm]),
            }
            for arm in ("graph", "paths")
        },
        "samples": samples,
        "limitations": (
            "Native CLI latency includes process launch and Git/index validation; "
            "this is a 4-seed fixture with 4 role queries, not representative task "
            "performance, runtime memory usage, or a proof of dependency completeness."
        ),
    }
    return result


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: native_paired.py /abs/path/to/excession /abs/output.json")
    binary = Path(sys.argv[1]).resolve()
    dest = Path(sys.argv[2]).resolve()
    with tempfile.TemporaryDirectory(prefix="excession-native-calibration-") as tmp:
        source = Path(tmp) / "h11"
        db = Path(tmp) / "index.sqlite"
        run(["git", "clone", "-q", str(BUNDLE), str(source)])
        commit = run(["git", "-C", str(source), "rev-parse", "HEAD"])[0].decode().strip()
        tree = run(["git", "-C", str(source), "rev-parse", "HEAD^{tree}"])[0].decode().strip()
        run([str(binary), "index", "--repo", str(source), "--db", str(db),
             "--name", "h11", "--rev", commit, "--roots", "h11,tests"])
        receipt = measure(binary, source, db, commit, tree)
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in receipt.items() if k != "samples"}, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
