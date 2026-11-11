#!/usr/bin/env python3
"""Bounded orchestration for a complete Linux syntax sweep.

All parsing and identity proofs are performed by the Rust Excession CLI.
This helper only schedules deterministic partition processes, checkpoints
their JSON receipts, and requests a final native Rust coverage audit.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def call(command, *, output=None, timeout=180):
    proc = subprocess.run(command, stdout=output if output is not None
                          else subprocess.PIPE, stderr=subprocess.PIPE,
                          timeout=timeout)
    if proc.returncode:
        raise RuntimeError(
            "Excession command failed: " + repr(command) + "\n"
            + proc.stderr.decode(errors="replace")[-2000:])
    return proc.stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rev", default="HEAD")
    parser.add_argument("--size", type=int, default=128)
    parser.add_argument("--max-partitions", type=int, default=0,
                        help="0 scans all; a positive value is an incomplete pilot")
    opts = parser.parse_args()
    if opts.size < 1 or opts.size > 128 or opts.max_partitions < 0:
        parser.error("size must be 1..128 and max-partitions cannot be negative")
    root = opts.out.resolve()
    root.mkdir(parents=True, exist_ok=True)
    binary, repo = str(opts.binary.resolve()), str(opts.repo.resolve())
    flags = ["--repo", repo, "--rev", opts.rev, "--size", str(opts.size)]
    inventory = json.loads(call([binary, "kernel", "inventory", *flags]))
    plan_path = root / "inventory.json"
    if plan_path.exists():
        previous = json.loads(plan_path.read_text())
        if previous["report_id"] != inventory["report_id"]:
            raise RuntimeError("existing partition directory belongs to another Git inventory")
    else:
        plan_path.write_text(json.dumps(inventory, sort_keys=True, indent=2) + "\n")
    total = inventory["partition_count"]
    limit = min(opts.max_partitions or total, total)
    started = time.monotonic()
    completed = 0
    for index in range(limit):
        destination = root / f"part-{index:05}.json"
        if destination.is_file():
            record = json.loads(destination.read_text())
            scope = record.get("partition") or {}
            if (record.get("source_commit") != inventory["source_commit"]
                    or scope.get("inventory_sha256") != inventory["inventory_sha256"]
                    or scope.get("index") != index or scope.get("size") != opts.size):
                raise RuntimeError(f"stale partition receipt {index}; refusing to overwrite")
        else:
            temporary = root / f".part-{index:05}.tmp"
            try:
                with temporary.open("wb") as stream:
                    call([binary, "kernel", "partition", *flags,
                          "--index", str(index)], output=stream, timeout=180)
                record = json.loads(temporary.read_text())
                scope = record.get("partition") or {}
                if (scope.get("index") != index or
                        scope.get("inventory_sha256") != inventory["inventory_sha256"]):
                    raise RuntimeError(f"partition {index} identity changed during scan")
                os.replace(temporary, destination)
            finally:
                temporary.unlink(missing_ok=True)
        completed += 1
        if completed % 16 == 0 or completed == limit:
            print(f"scanned/accounted {completed}/{total} partitions; "
                  f"elapsed {time.monotonic() - started:.1f}s", file=sys.stderr, flush=True)
    if completed == total:
        coverage = json.loads(call([binary, "kernel", "audit", *flags,
                                    "--dir", str(root)], timeout=240))
        if coverage["eligible_sources"] != inventory["eligible_source_count"]:
            raise RuntimeError("audit source total differs from initial inventory")
        summary = {
            "schema": "excession.kernel-full-sweep.v1",
            "status": coverage["status"],
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "coverage": coverage,
        }
    else:
        summary = {
            "schema": "excession.kernel-full-sweep.v1",
            "status": "INCOMPLETE_PARTITIONS_UNAUDITED",
            "source_commit": inventory["source_commit"],
            "inventory_sha256": inventory["inventory_sha256"],
            "eligible_sources": inventory["eligible_source_count"],
            "completed_partitions": completed,
            "required_partitions": total,
            "elapsed_seconds": round(time.monotonic() - started, 3),
        }
    summary_path = root / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
