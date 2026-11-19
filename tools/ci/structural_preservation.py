#!/usr/bin/env python3
"""Bounded, source-pinned behavioral receipts for reviewed structural candidates.

Runs explicitly trusted, reviewed commands in separate detached Git worktrees.
Never claims all-input equivalence or authorizes a merge.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import time

SCHEMA = "excession.structural-preservation-contract.v1"
REPORT = "excession.structural-preservation-receipt.v1"


class Refused(Exception):
    pass


def git(repo: Path, *argv: str) -> bytes:
    proc = subprocess.run(["git", "-C", str(repo), *argv], capture_output=True)
    if proc.returncode:
        raise Refused(f"Git operation {argv[0]} failed: {proc.stderr.decode(errors='replace')[-1000:]}")
    return proc.stdout


def pin(repo: Path, name: str) -> tuple[str, str]:
    if not re.fullmatch(r"[0-9a-f]{40}", name):
        raise Refused("base/candidate must be full immutable SHA-1 Git commit IDs")
    commit = git(repo, "rev-parse", "--verify", "--end-of-options", name + "^{commit}").decode().strip()
    tree = git(repo, "rev-parse", "--verify", commit + "^{tree}").decode().strip()
    if commit != name:
        raise Refused("requested commit differs from resolved commit")
    return commit, tree


def regular_path(path: str) -> bool:
    return (isinstance(path, str) and bool(path) and not path.startswith("/")
            and "\\" not in path and all(p not in ("", ".", "..") for p in path.split("/"))
            and not any(ord(ch) < 32 for ch in path))


def read_contract(path: Path) -> tuple[dict, str]:
    raw = path.read_bytes()
    if len(raw) > 128 * 1024:
        raise Refused("contract exceeds 128 KiB")
    doc = json.loads(raw)
    if not isinstance(doc, dict) or set(doc) != {"schema", "changed_paths", "obligations"} or doc["schema"] != SCHEMA:
        raise Refused("unexpected preservation contract schema or fields")
    paths = doc["changed_paths"]
    if not isinstance(paths, list) or not paths or len(paths) != len(set(map(str, paths))) or not all(regular_path(p) for p in paths):
        raise Refused("changed_paths must contain unique regular relative Git paths")
    checks = doc["obligations"]
    if not isinstance(checks, list) or not 1 <= len(checks) <= 16:
        raise Refused("requires 1-16 explicit preservation obligations")
    names = set()
    for c in checks:
        if not isinstance(c, dict) or set(c) != {"id", "kind", "cwd", "argv", "timeout_seconds", "max_output_bytes"}:
            raise Refused("invalid obligation fields")
        if not isinstance(c["id"], str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", c["id"]) or c["id"] in names:
            raise Refused("duplicate or invalid obligation ID")
        names.add(c["id"])
        if c["kind"] not in ("differential", "candidate_pass"):
            raise Refused("unknown preservation obligation kind")
        if c["cwd"] != "." and not regular_path(c["cwd"]):
            raise Refused("execution cwd must be inside worktree")
        if not isinstance(c["argv"], list) or not 1 <= len(c["argv"]) <= 32 or not all(isinstance(s, str) and 0 < len(s) <= 1024 and "\0" not in s for s in c["argv"]):
            raise Refused("command must be an explicit nonempty argv array")
        if not isinstance(c["timeout_seconds"], int) or not 1 <= c["timeout_seconds"] <= 300:
            raise Refused("timeout must be 1-300 seconds")
        if not isinstance(c["max_output_bytes"], int) or not 1 <= c["max_output_bytes"] <= 8 * 1024 * 1024:
            raise Refused("output budget must be 1-8 MiB")
    if not any(c["kind"] == "differential" for c in checks):
        raise Refused("at least one independent differential observation is mandatory")
    if not any(c["kind"] == "candidate_pass" for c in checks):
        raise Refused("at least one candidate acceptance command is mandatory")
    return doc, hashlib.sha256(raw).hexdigest()


def changed_paths(repo: Path, base: str, candidate: str) -> list[str]:
    # No rename heuristic: the manifest must enumerate every actual Git path touched.
    raw = git(repo, "diff", "--no-renames", "--name-only", "-z", base, candidate)
    return sorted(p.decode("utf-8") for p in raw.split(b"\0") if p)


def verify_changed_entry_modes(repo: Path, base: str, candidate: str, paths: list[str]) -> None:
    for revision in (base, candidate):
        for path in paths:
            entries = [item for item in git(repo, "ls-tree", "-z", "--full-tree", revision, "--", path).split(b"\0") if item]
            if not entries:
                continue  # additions and deletions are permitted
            if len(entries) != 1:
                raise Refused("changed path does not denote exactly one source entry")
            header, name = entries[0].split(b"\t", 1)
            if name.decode("utf-8") != path:
                raise Refused("changed path unexpectedly resolved to another Git entry")
            mode, kind, _oid = header.decode("ascii").split()
            if kind != "blob" or mode not in ("100644", "100755"):
                raise Refused("changed path is not a regular tracked Git source file")


def run_command(worktree: Path, check: dict, home: Path) -> dict:
    cwd = (worktree / check["cwd"]).resolve()
    if not cwd.is_relative_to(worktree.resolve()) or not cwd.is_dir():
        raise Refused("requested cwd absent or outside detached worktree")
    command = check["argv"]
    if "/" in command[0]:
        executable_path = Path(command[0])
        executable_path = (cwd / executable_path).resolve() if not executable_path.is_absolute() else executable_path.resolve()
        # A relative executable is located in its own Git worktree, never
        # in the verifier checkout or the other compared worktree.
        if not executable_path.is_file() or not os.access(executable_path, os.X_OK):
            return {"status": "INCONCLUSIVE", "reason": "EXECUTABLE_UNAVAILABLE", "argv": command}
        executable = str(executable_path)
    else:
        executable = shutil.which(command[0], path=os.environ.get("PATH", ""))
        if not executable:
            return {"status": "INCONCLUSIVE", "reason": "EXECUTABLE_UNAVAILABLE", "argv": command}
    environment = {
        "PATH": os.environ.get("PATH", ""), "HOME": str(home),
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC",
        "PYTHONDONTWRITEBYTECODE": "1", "GIT_CONFIG_NOSYSTEM": "1",
    }
    # Pipes are drained incrementally. Never spool unbounded child output to disk.
    # POSIX process groups let us terminate descendants retaining inherited pipes.
    if os.name != "posix":
        return {"status": "INCONCLUSIVE", "reason": "BOUNDED_CAPTURE_UNSUPPORTED", "argv": command}
    try:
        process = subprocess.Popen(
            command, cwd=cwd, env=environment, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, start_new_session=True,
        )
    except OSError as exc:
        return {"status": "INCONCLUSIVE", "reason": "EXECUTION_UNAVAILABLE",
                "detail": type(exc).__name__, "argv": command}
    hashes = {"stdout": hashlib.sha256(), "stderr": hashlib.sha256()}
    counts = {"stdout": 0, "stderr": 0}
    budget = check["max_output_bytes"]
    deadline = time.monotonic() + check["timeout_seconds"]
    timed_out = False
    exceeded = False
    selector = selectors.DefaultSelector()
    try:
        for name, pipe in (("stdout", process.stdout), ("stderr", process.stderr)):
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, name)
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                break
            events = selector.select(timeout=min(remaining, 0.1))
            for key, _mask in events:
                # The single sentinel byte establishes overflow without consuming
                # an arbitrary extra chunk. Counts are observed, not total, bytes.
                chunk = os.read(key.fd, min(65536, budget - counts[key.data] + 1))
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                name = key.data
                counts[name] += len(chunk)
                if counts[name] > budget:
                    exceeded = True
                    break
                hashes[name].update(chunk)
            if exceeded:
                break
        if not timed_out and not exceeded:
            try:
                returncode = process.wait(timeout=max(0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                timed_out = True
        if timed_out or exceeded:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            returncode = process.wait()
    finally:
        # A successful leader exit and EOF do not prove that background
        # descendants have exited: they may have closed both pipes. Revoke
        # the session's process group on every path, including success.
        # This is not a sandbox against setsid() or detached processes.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        selector.close()
        for pipe in (process.stdout, process.stderr):
            pipe.close()
        if process.poll() is None:
            process.wait()
    status = "INCONCLUSIVE" if timed_out or exceeded else "OBSERVED"
    return {
        "argv": command, "status": status,
        "reason": "OUTPUT_BUDGET" if exceeded else "TIMEOUT" if timed_out else None,
        "exit_code": returncode, "timed_out": timed_out,
        # Until both pipes reach EOF, no complete-output digest is defensible.
        # The byte counts on inconclusive runs are observed prefixes only.
        "stdout_sha256": None if exceeded or timed_out else hashes["stdout"].hexdigest(),
        "stdout_bytes": counts["stdout"],
        "stderr_sha256": None if exceeded or timed_out else hashes["stderr"].hexdigest(),
        "stderr_bytes": counts["stderr"],
        "executable_path": str(Path(executable).resolve()),
    }


def evaluate(repo: Path, base: str, candidate: str, doc: dict, sha: str) -> dict:
    original, tree_a = pin(repo, base)
    amended, tree_b = pin(repo, candidate)
    if original == amended or tree_a == tree_b:
        raise Refused("structural candidate must have a distinct Git source tree")
    if subprocess.run(["git", "-C", str(repo), "merge-base", "--is-ancestor", original, amended]).returncode != 0:
        raise Refused("base must be an ancestor of candidate")
    changed = changed_paths(repo, original, amended)
    if changed != sorted(doc["changed_paths"]):
        raise Refused(f"changed-path mismatch: observed {changed} versus declared {sorted(doc['changed_paths'])}")
    verify_changed_entry_modes(repo, original, amended, changed)
    report = {
        "schema": REPORT, "base_commit": original, "base_tree": tree_a,
        "candidate_commit": amended, "candidate_tree": tree_b,
        "contract_sha256": sha, "changed_paths": changed,
        "obligations": [], "status": "INCONCLUSIVE",
        "ready_to_merge": False, "safe_to_skip_unlisted_tests": False,
        "semantic_equivalence": "NOT_PROVED",
        "open_obligations": ["UNTESTED_INPUTS", "UNMODELED_STATE_AND_EFFECTS", "EXECUTION_NOT_ATTESTED"],
    }
    with tempfile.TemporaryDirectory(prefix="excession-preservation-") as temp:
        root = Path(temp)
        for index, check in enumerate(doc["obligations"]):
            # Each obligation gets new worktrees and HOME directories.
            # Earlier tests cannot seed later tests through untracked files,
            # caches or generated outputs in the measured checkout.
            a, b = root / f"base-{index}", root / f"candidate-{index}"
            base_home, candidate_home = root / f"base-home-{index}", root / f"candidate-home-{index}"
            attached = []
            try:
                if check["kind"] == "differential":
                    git(repo, "-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", str(a), original)
                    attached.append(a)
                git(repo, "-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", str(b), amended)
                attached.append(b)
                base_home.mkdir()
                candidate_home.mkdir()
                baseline = run_command(a, check, base_home) if check["kind"] == "differential" else None
                if baseline and git(a, "status", "--porcelain", "--untracked-files=no").strip():
                    raise Refused("baseline command modified tracked Git source")
                proposed = run_command(b, check, candidate_home)
                if git(b, "status", "--porcelain", "--untracked-files=no").strip():
                    raise Refused("candidate command modified tracked Git source")
                outcome = "PASS"
                if proposed["status"] != "OBSERVED" or (baseline and baseline["status"] != "OBSERVED"):
                    outcome = "INCONCLUSIVE"
                elif proposed["exit_code"] != 0 or (baseline and baseline["exit_code"] != 0):
                    outcome = "FAIL"
                elif baseline and any(baseline[k] != proposed[k] for k in ("stdout_sha256", "stderr_sha256", "exit_code")):
                    outcome = "FAIL"
                report["obligations"].append({
                    "id": check["id"], "kind": check["kind"], "result": outcome,
                    "base": baseline, "candidate": proposed,
                })
            finally:
                for path in reversed(attached):
                    git(repo, "worktree", "remove", "--force", str(path))
        statuses = {o["result"] for o in report["obligations"]}
        report["status"] = ("FAILED" if "FAIL" in statuses else
                            "INCONCLUSIVE" if "INCONCLUSIVE" in statuses else
                            "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE")
    return report


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--repo", type=Path, required=True)
    p.add_argument("--base", required=True)
    p.add_argument("--candidate", required=True)
    p.add_argument("--contract", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--trusted-local", choices=["true"], required=True)
    a = p.parse_args()
    try:
        repo = a.repo.resolve(strict=True)
        doc, digest = read_contract(a.contract)
        report = evaluate(repo, a.base, a.candidate, doc, digest)
        text = json.dumps(report, indent=2, sort_keys=True) + "\n"
        a.out.write_text(text, encoding="utf-8")
        print(text, end="")
        return 0 if report["status"] == "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE" else 1
    except (Refused, ValueError, KeyError, TypeError, UnicodeError, OSError, json.JSONDecodeError) as exc:
        print(f"structural preservation REFUSED: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
