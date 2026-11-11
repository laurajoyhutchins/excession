#!/usr/bin/env python3
"""Observe Python import *witnesses* during one explicitly trusted local run.

This is intentionally not a runtime-completeness tracer or an isolation boundary.
It records successful import calls and newly retained sys.modules entries, with
their Git binding where possible. It cannot mediate arbitrary native loaders,
processes, import hooks, module cache mutation, or hostile Python code.
"""
from __future__ import annotations

import argparse
import builtins
from collections import Counter
import hashlib
import importlib
import importlib.util
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import time
from typing import Any


SCHEMA = "excession.python-runtime-witness.v1"
MAX_WITNESSES = 10000
MAX_ATTEMPTS = 500
MAX_AUDIT_EVENTS = 100000


def git(repo: Path, *args: str) -> bytes:
    proc = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True, check=False, timeout=30,
    )
    if proc.returncode:
        raise ValueError(
            "Git command rejected source: "
            + proc.stderr.decode("utf-8", "replace")[:250]
        )
    return proc.stdout


def git_identity(repo: Path, revision: str) -> tuple[str, str, dict[str, tuple[str, str]]]:
    """Read tracked file identities from Git, not the working directory."""
    commit = git(repo, "rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip()
    head = git(repo, "rev-parse", "HEAD").decode().strip()
    if commit != head or len(commit) != 40:
        raise ValueError("observation requires the checked-out SHA-1 HEAD")
    if git(repo, "status", "--porcelain", "--untracked-files=no").strip():
        raise ValueError("tracked checkout contains modifications; refuse source binding")
    tree = git(repo, "rev-parse", f"{commit}^{{tree}}").decode().strip()
    rows = git(repo, "ls-tree", "-r", "-z", "--full-tree", commit)
    result: dict[str, tuple[str, str]] = {}
    for line in rows.split(b"\0"):
        if not line:
            continue
        header, filename = line.split(b"\t", 1)
        mode, kind, blob = header.decode("ascii").split()
        if kind == "blob":
            path = filename.decode("utf-8")
            if path in result:
                raise ValueError("duplicate tracked Git path")
            result[path] = mode, blob
    return commit, tree, result


def sha256_file(path: Path) -> str:
    size = path.stat().st_size
    if not path.is_file() or size > 128 * 1024 * 1024:
        raise ValueError("interpreter binary absent or too large to hash")
    return hashlib.sha256(path.read_bytes()).hexdigest()


class WitnessCollector:
    def __init__(self, repo: Path, tracked: dict[str, tuple[str, str]]) -> None:
        self.repo = repo
        self.tracked = tracked
        self.calls: Counter[tuple[str, str, str, str]] = Counter()
        self.failed: Counter[tuple[str, str, str]] = Counter()
        self.audit: Counter[str] = Counter()
        self.overflow = Counter()
        self.initial_modules = set(sys.modules)
        self.original_import = builtins.__import__
        self.original_import_module = importlib.import_module

    def module_binding(self, module_name: str) -> dict[str, Any]:
        module = sys.modules.get(module_name)
        spec = getattr(module, "__spec__", None)
        origin = getattr(spec, "origin", None) or getattr(module, "__file__", None)
        if not isinstance(origin, str) or origin in {"built-in", "frozen", "namespace"}:
            return {"status": "NO_SOURCE_FILE", "origin": str(origin or "")}
        try:
            resolved = Path(origin).resolve(strict=True)
        except (OSError, RuntimeError):
            return {"status": "UNRESOLVED_SOURCE_PATH", "origin": origin}
        try:
            relative = resolved.relative_to(self.repo).as_posix()
        except ValueError:
            return {"status": "EXTERNAL_TO_PINNED_REPO", "origin": str(resolved)}
        identity = self.tracked.get(relative)
        if identity is None:
            return {"status": "UNTRACKED_REPO_PATH", "path": relative}
        mode, declared_oid = identity
        if mode not in {"100644", "100755"} or not resolved.is_file():
            return {"status": "UNSUPPORTED_GIT_MODE", "path": relative, "mode": mode}
        try:
            content = resolved.read_bytes()
        except OSError:
            return {"status": "SOURCE_UNREADABLE", "path": relative}
        actual_oid = hashlib.sha1(
            b"blob " + str(len(content)).encode() + b"\0" + content
        ).hexdigest()
        if actual_oid != declared_oid:
            return {"status": "GIT_BLOB_MISMATCH", "path": relative,
                    "expected_oid": declared_oid, "observed_oid": actual_oid}
        return {"status": "GIT_BOUND", "path": relative,
                "mode": mode, "blob_oid": declared_oid}

    def record(self, caller: str, name: str, mechanism: str) -> None:
        if name not in sys.modules or not isinstance(name, str):
            return
        # Keep one row per caller/target/mechanism, with bounded counts.
        record = (caller, name, mechanism, "MODULE_PRESENT")
        if record not in self.calls and len(self.calls) >= MAX_WITNESSES:
            self.overflow["witnesses"] += 1
        else:
            self.calls[record] += 1

    def fail(self, caller: str, name: str, mechanism: str) -> None:
        record = (caller, name, mechanism)
        if record not in self.failed and len(self.failed) >= MAX_ATTEMPTS:
            self.overflow["failed_attempts"] += 1
        else:
            self.failed[record] += 1

    def import_call(self, name: str, globals_: dict | None = None,
                    locals_: dict | None = None, fromlist=(), level: int = 0):
        caller = globals_.get("__name__", "<unknown>") if isinstance(globals_, dict) else "<unknown>"
        package = globals_.get("__package__", "") if isinstance(globals_, dict) else ""
        try:
            result = self.original_import(name, globals_, locals_, fromlist, level)
        except BaseException:
            self.fail(str(caller), str(name), "builtins.__import__")
            raise
        normalized = name
        if level:
            try:
                normalized = importlib.util.resolve_name("." * level + name, package or "")
            except (ValueError, ImportError, TypeError):
                normalized = name
        self.record(str(caller), normalized, "builtins.__import__")
        if fromlist and normalized:
            for member in fromlist:
                if isinstance(member, str):
                    self.record(str(caller), f"{normalized}.{member}", "builtins.__import__")
        return result

    def import_module_call(self, name: str, package: str | None = None):
        try:
            result = self.original_import_module(name, package)
        except BaseException:
            self.fail("<importlib-caller>", str(name), "importlib.import_module")
            raise
        self.record("<importlib-caller>", str(getattr(result, "__name__", name)),
                    "importlib.import_module")
        return result

    def audit_event(self, event: str, _args: tuple) -> None:
        if event not in {"import", "subprocess.Popen", "os.system", "os.exec",
                         "ctypes.dlopen"}:
            return
        if sum(self.audit.values()) >= MAX_AUDIT_EVENTS:
            self.overflow["audit_events"] += 1
        else:
            self.audit[event] += 1

    def install(self) -> None:
        # Audit hooks cannot be removed; the observer runs as a one-shot process.
        sys.addaudithook(self.audit_event)
        builtins.__import__ = self.import_call
        importlib.import_module = self.import_module_call

    def uninstall(self) -> None:
        builtins.__import__ = self.original_import
        importlib.import_module = self.original_import_module

    def report(self) -> dict[str, Any]:
        for name in sorted(set(sys.modules) - self.initial_modules):
            self.record("<module-cache-delta>", name, "sys.modules.after")
        records = []
        for (caller, name, mechanism, status), count in sorted(self.calls.items()):
            records.append({
                "importer": caller, "module": name, "mechanism": mechanism,
                "observation": status, "count": count,
                "source": self.module_binding(name),
            })
        failures = [
            {"importer": caller, "requested": name, "mechanism": mechanism,
             "count": count, "status": "FAILED_IMPORT_ATTEMPT_NOT_DEPENDENCY"}
            for (caller, name, mechanism), count in sorted(self.failed.items())
        ]
        gaps = [
            {"kind": "COVERAGE_UNPROVEN",
             "reason": "Python import wrappers and audit callbacks are not mandatory mediation"},
            {"kind": "PREEXISTING_MODULES",
             "count": len(self.initial_modules),
             "reason": "imports before instrumentation are not reconstructed"},
            {"kind": "CHILD_AND_NATIVE_CODE",
             "reason": "child processes, native loaders, dynamic code and arbitrary execution are not mediated"},
        ]
        if self.audit["subprocess.Popen"] or self.audit["os.exec"]:
            gaps.append({"kind": "CHILD_PROCESS_OBSERVED",
                         "count": self.audit["subprocess.Popen"] + self.audit["os.exec"]})
        if self.audit["ctypes.dlopen"]:
            gaps.append({"kind": "NATIVE_LOADER_OBSERVED",
                         "count": self.audit["ctypes.dlopen"]})
        if self.overflow:
            gaps.append({"kind": "OBSERVATION_CAP_EXCEEDED", "counts": dict(self.overflow)})
        return {
            "witnesses": records,
            "failed_import_attempts": failures,
            "audit_event_counts": dict(sorted(self.audit.items())),
            "gaps": gaps,
            "limits": {"max_witnesses": MAX_WITNESSES,
                       "max_failed_attempts": MAX_ATTEMPTS,
                       "max_audit_events": MAX_AUDIT_EVENTS},
        }


def run(args: argparse.Namespace) -> int:
    if not args.trusted_local:
        raise ValueError("explicit --trusted-local is required; target code executes unsandboxed")
    repo = Path(args.repo).resolve(strict=True)
    target = (repo / args.script).resolve(strict=True)
    if target.suffix != ".py" or not target.is_file():
        raise ValueError("entrypoint must be an existing Python script")
    try:
        relative = target.relative_to(repo).as_posix()
    except ValueError as exc:
        raise ValueError("entrypoint escapes Git checkout") from exc
    output = Path(args.out).resolve()
    if output.exists():
        raise ValueError("output already exists")
    if output.parent == repo or repo in output.parents:
        raise ValueError("report output must be outside the source checkout")
    if not output.parent.is_dir():
        raise ValueError("report parent directory must already exist")

    commit, tree, tracked = git_identity(repo, args.rev)
    if relative not in tracked:
        raise ValueError("entrypoint must be a tracked Git blob")
    collector = WitnessCollector(repo, tracked)
    mode, expected_oid = tracked[relative]
    raw = target.read_bytes()
    actual = hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()
    if mode not in {"100644", "100755"} or actual != expected_oid:
        raise ValueError("entrypoint differs from pinned Git source")

    original_argv, original_path = sys.argv, list(sys.path)
    sys.argv = [str(target), *args.arguments]
    sys.path.insert(0, str(target.parent))
    started = time.perf_counter_ns()
    failure = None
    status = 0
    collector.install()
    try:
        runpy.run_path(str(target), run_name="__main__")
    except SystemExit as exc:
        status = (exc.code if isinstance(exc.code, int) else 1) if exc.code else 0
        if status:
            failure = {"type": "SystemExit", "message": str(exc.code)[:300]}
    except BaseException as exc:
        status = 1
        failure = {"type": type(exc).__name__, "message": str(exc)[:300]}
    finally:
        collector.uninstall()
        sys.argv = original_argv
        sys.path[:] = original_path
    elapsed_ms = round((time.perf_counter_ns() - started) / 1e6, 3)
    receipt = {
        "schema": SCHEMA,
        "claim": "OBSERVED_RUNTIME_ONLY",
        "source_commit": commit,
        "source_tree": tree,
        "entrypoint": {"path": relative, "blob_oid": expected_oid, "mode": mode},
        "interpreter": {"version": sys.version.split()[0],
                        "executable": str(Path(sys.executable).resolve()),
                        "sha256": sha256_file(Path(sys.executable).resolve())},
        "execution": {"exit_code": status, "failure": failure,
                      "elapsed_wall_ms": elapsed_ms},
        **collector.report(),
    }
    receipt["receipt_sha256"] = hashlib.sha256(
        json.dumps(receipt, ensure_ascii=True, sort_keys=True,
                   separators=(",", ":")).encode()
    ).hexdigest()
    output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"runtime witness receipt: {output} ({len(receipt['witnesses'])} records)",
          file=sys.stderr)
    return status


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--rev", default="HEAD")
    parser.add_argument("--script", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--trusted-local", action="store_true")
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.arguments and args.arguments[0] == "--":
        args.arguments.pop(0)
    try:
        code = run(args)
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        parser.exit(2, f"REFUSED: {exc}\n")
    raise SystemExit(code)


if __name__ == "__main__":
    main()
