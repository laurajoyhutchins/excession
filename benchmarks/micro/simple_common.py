"""Minimal measurement and corpus helpers for independent retrieval probes."""
from __future__ import annotations

import json
from pathlib import Path
import resource
import subprocess
import time
from typing import Any


def command(argv: list[str], timeout: float) -> tuple[subprocess.CompletedProcess[bytes], dict[str, float]]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter_ns()
    try:
        proc = subprocess.run(argv, capture_output=True, check=False, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        proc = subprocess.CompletedProcess(argv, 124, exc.stdout or b"", exc.stderr or b"timed out")
    except FileNotFoundError as exc:
        proc = subprocess.CompletedProcess(argv, 127, b"", str(exc).encode())
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    return proc, {
        "wall_ms": round((time.perf_counter_ns() - start) / 1_000_000, 3),
        "child_user_cpu_ms": round((after.ru_utime - before.ru_utime) * 1000, 3),
        "child_system_cpu_ms": round((after.ru_stime - before.ru_stime) * 1000, 3),
    }


def git_value(repo: Path, *args: str) -> str:
    proc, _ = command(["git", "-C", str(repo), *args], 20)
    if proc.returncode:
        raise RuntimeError(proc.stderr.decode(errors="replace")[:300])
    return proc.stdout.decode("utf-8").strip()


def load_tasks(path: Path) -> list[dict[str, Any]]:
    doc = json.loads(path.read_text(encoding="utf-8"))
    if doc.get("schema") != "excession.benchmark.calibration.v1":
        raise ValueError("unsupported task corpus schema")
    tasks = doc.get("tasks")
    if not isinstance(tasks, list) or not tasks:
        raise ValueError("missing tasks")
    seen = set()
    for task in tasks:
        if not isinstance(task, dict):
            raise ValueError("invalid task")
        for key in ("id", "seed", "kind", "question"):
            if not isinstance(task.get(key), str) or not task[key]:
                raise ValueError(f"invalid task {key}")
        if task["id"] in seen:
            raise ValueError("duplicate task ID")
        seen.add(task["id"])
        expected = task.get("expected_source_paths")
        if expected is not None:
            if not isinstance(expected, list) or not all(
                isinstance(p, str) and p and not p.startswith("/") and ".." not in p.split("/")
                for p in expected
            ) or len(expected) != len(set(expected)):
                raise ValueError("invalid declared paths")
    return tasks
