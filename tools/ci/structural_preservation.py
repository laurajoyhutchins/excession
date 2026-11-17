#!/usr/bin/env python3
"""Bounded, source-pinned behavioral receipts for reviewed structural candidates.

Runs explicitly trusted, reviewed commands in separate detached Git worktrees.
Never claims all-input equivalence or authorizes a merge.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile

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


def digest_file(file) -> tuple[str, int]:
    file.seek(0)
    h = hashlib.sha256()
    count = 0
    while True:
        chunk = file.read(65536)
        if not chunk:
            break
        count += len(chunk)
        h.update(chunk)
    return h.hexdigest(), count


def run_command(worktree: Path, check: dict, home: Path) -> dict:
    cwd = (worktree / check["cwd"]).resolve()
    if not cwd.is_relative_to(worktree.resolve()) or not cwd.is_dir():
        raise Refused("requested cwd absent or outside detached worktree")
    command = check["argv"]
    executable = shutil.which(command[0], path=os.environ.get("PATH", ""))
    if not executable:
        return {"status": "INCONCLUSIVE", "reason": "EXECUTABLE_UNAVAILABLE", "argv": command}
    environment = {
        "PATH": os.environ.get("PATH", ""), "HOME": str(home),
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC",
        "PYTHONDONTWRITEBYTECODE": "1", "GIT_CONFIG_NOSYSTEM": "1",
    }
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        try:
            process = subprocess.Popen(
                command, cwd=cwd, env=environment, stdout=stdout, stderr=stderr,
                start_new_session=(os.name == "posix"),
            )
        except OSError as exc:
            return {"status": "INCONCLUSIVE", "reason": "EXECUTION_UNAVAILABLE",
                    "detail": type(exc).__name__, "argv": command}
        timed_out = False
        try:
            returncode = process.wait(timeout=check["timeout_seconds"])
        except subprocess.TimeoutExpired:
            timed_out = True
            if os.name == "posix":
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
            returncode = process.wait()
        out_sha, out_bytes = digest_file(stdout)
        err_sha, err_bytes = digest_file(stderr)
        status = "OBSERVED"
        if timed_out or out_bytes > check["max_output_bytes"] or err_bytes > check["max_output_bytes"]:
            status = "INCONCLUSIVE"
        return {
            "argv": command, "status": status,
            "reason": "TIMEOUT_OR_OUTPUT_BUDGET" if status == "INCONCLUSIVE" else None,
            "exit_code": returncode, "timed_out": timed_out,
            "stdout_sha256": out_sha, "stdout_bytes": out_bytes,
            "stderr_sha256": err_sha, "stderr_bytes": err_bytes,
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
        a, b = root / "base", root / "candidate"
        attached = []
        try:
            for path, rev in ((a, original), (b, amended)):
                git(repo, "-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", str(path), rev)
                attached.append(path)
            (root / "home").mkdir()
            for check in doc["obligations"]:
                baseline = run_command(a, check, root / "home") if check["kind"] == "differential" else None
                proposed = run_command(b, check, root / "home")
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
            statuses = {o["result"] for o in report["obligations"]}
            report["status"] = ("FAILED" if "FAIL" in statuses else
                                "INCONCLUSIVE" if "INCONCLUSIVE" in statuses else
                                "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE")
        finally:
            for path in reversed(attached):
                git(repo, "worktree", "remove", "--force", str(path))
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
