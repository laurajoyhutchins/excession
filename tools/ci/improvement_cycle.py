#!/usr/bin/env python3
"""Verify that a native improvement is an exact, repeatable Git transformation.

This module does not run general LLM edits or certify semantic equivalence.
The integration authority lives in the workflow after independent CI receipts.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

CATALOG = "quality/improvement-recipes.json"
SCHEMA = "excession.improvement-recipes.v1"
RECEIPT = "excession.native-improvement-reproduction.v1"
# This is an executable allowlist, not a field that a candidate can grant itself.
AUTO_NATIVE_PRODUCERS = {"native-rustfmt": "cargo-fmt-pinned",
                         "native-cargo-lock-refresh": "cargo-update-locked"}
REVIEW_NATIVE_PRODUCERS = {"native-clippy": "cargo-clippy-fix-pinned"}
MAX_CHANGED_PATHS = 128


class Refused(Exception):
    pass


def run(argv: list[str], *, cwd: Path, timeout: int = 120) -> bytes:
    try:
        completed = subprocess.run(argv, cwd=cwd, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Refused(f"native command unavailable or timed out: {argv[0]}") from exc
    if completed.returncode:
        raise Refused(f"{argv[0]} exited {completed.returncode}: "
                      + completed.stderr.decode(errors="replace")[-1000:])
    return completed.stdout


def git(repo: Path, *args: str) -> bytes:
    return run(["git", "-C", str(repo), *args], cwd=repo, timeout=60)


def pin(repo: Path, value: str) -> str:
    if not re.fullmatch(r"[a-f0-9]{40}", value):
        raise Refused("exact 40-digit SHA-1 commit required")
    if git(repo, "rev-parse", "--verify", "--end-of-options",
           value + "^{commit}").decode().strip() != value:
        raise Refused("source did not resolve to requested immutable commit")
    return value


def git_blob(repo: Path, rev: str, path: str) -> bytes:
    if not path.startswith("quality/") and not path.startswith("core/"):
        raise Refused("unexpected source path")
    return git(repo, "show", f"{rev}:{path}")


def validated_catalog(data: bytes) -> dict:
    if len(data) > 32 * 1024:
        raise Refused("recipe catalog too large")
    try:
        obj = json.loads(data)
        if not isinstance(obj, dict) or set(obj) != {"schema", "recipes"} or obj["schema"] != SCHEMA:
            raise ValueError("unexpected catalog fields")
        recipes = obj["recipes"]
        if not isinstance(recipes, list) or not 1 <= len(recipes) <= 24:
            raise ValueError("invalid recipe collection")
        seen = set()
        for item in recipes:
            if not isinstance(item, dict) or set(item) != {
                "id", "producer", "integration", "paths", "rationale",
                "evidence", "independent_checks",
            }:
                raise ValueError("invalid recipe fields")
            rid = item["id"]
            if not isinstance(rid, str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", rid) or rid in seen:
                raise ValueError("duplicate or invalid recipe ID")
            seen.add(rid)
            allowed_paths = (["core/Cargo.lock"] if rid == "native-cargo-lock-refresh"
                             else ["core/**/*.rs"])
            if item["paths"] != allowed_paths:
                raise ValueError("recipe path escapes its hard-allowlisted scope")
            if any(not isinstance(item[k], str) or not item[k].strip()
                   for k in ("producer", "integration", "rationale", "evidence")):
                raise ValueError("incomplete recipe")
            if not isinstance(item["independent_checks"], list) or not all(
                isinstance(c, str) and c for c in item["independent_checks"]
            ):
                raise ValueError("invalid check list")
            if rid in AUTO_NATIVE_PRODUCERS:
                if item["producer"] != AUTO_NATIVE_PRODUCERS[rid] or item["integration"] != "automatic-after-exact-head-verification":
                    raise ValueError("automatic integration recipe changed its authority")
                required = ({"cargo-lock-graph-audit", "cargo-tests", "quality-ratchet",
                             "verify-workflow", "github-evidence-workflow"}
                            if rid == "native-cargo-lock-refresh" else
                            {"formatter-replay", "quality-ratchet", "cargo-tests",
                             "verify-workflow", "github-evidence-workflow"})
                if not required.issubset(item["independent_checks"]):
                    raise ValueError("missing mandatory auto-integration checks")
            elif rid in REVIEW_NATIVE_PRODUCERS:
                if item["producer"] != REVIEW_NATIVE_PRODUCERS[rid] or item["integration"] != "review-required":
                    raise ValueError("review producer attempted to gain automatic authority")
            elif item["integration"] != "review-required":
                raise ValueError("unknown recipes must require explicit review")
        if set(AUTO_NATIVE_PRODUCERS) - seen:
            raise ValueError("auto integration recipe missing")
        return obj
    except (ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        raise Refused(f"invalid improvement recipe catalog: {exc}") from exc


def changed_entries(repo: Path, base: str, head: str) -> list[str]:
    data = git(repo, "diff", "--no-renames", "--name-status", "-z", base, head)
    records = data.split(b"\0")
    if records[-1:] != [b""]:
        raise Refused("malformed Git changed-path stream")
    paths = []
    pos = 0
    while pos < len(records) - 1:
        status = records[pos].decode("ascii")
        path = records[pos + 1].decode("utf-8")
        pos += 2
        if status != "M":
            raise Refused(f"native formatter must not add, delete, rename, copy or change type: {status} {path}")
        if not path.startswith("core/") or not path.endswith(".rs") or any(
            segment in (".", "..", "") for segment in path.split("/")
        ):
            raise Refused(f"formatter changed unapproved path: {path}")
        source_modes = []
        for rev in (base, head):
            entries = [p for p in git(repo, "ls-tree", "-z", "--full-tree", rev, "--", path).split(b"\0") if p]
            if len(entries) != 1:
                raise Refused(f"missing or ambiguous regular Git blob: {path}")
            header, found = entries[0].split(b"\t", 1)
            mode, kind, _oid = header.decode("ascii").split()
            if found.decode("utf-8") != path or mode not in ("100644", "100755") or kind != "blob":
                raise Refused(f"nonregular or mismatched source: {path}")
            source_modes.append(mode)
        if source_modes[0] != source_modes[1]:
            raise Refused(f"native formatter changed executable mode: {path}")
        paths.append(path)
    if not 1 <= len(paths) <= MAX_CHANGED_PATHS or len(set(paths)) != len(paths):
        raise Refused("formatter must change 1-128 distinct Rust source files")
    if git(repo, "diff", "--check", base, head).strip():
        raise Refused("candidate diff has whitespace errors")
    return sorted(paths)


def verify(repo: Path, base: str, candidate: str, *, formatter: list[str] | None = None) -> dict:
    base = pin(repo, base)
    candidate = pin(repo, candidate)
    parents = git(repo, "rev-list", "--parents", "-n", "1", candidate).decode().strip().split()
    if parents != [candidate, base]:
        raise Refused("candidate must be one commit directly atop pinned main")
    old_catalog = git_blob(repo, base, CATALOG)
    new_catalog = git_blob(repo, candidate, CATALOG)
    if old_catalog != new_catalog:
        raise Refused("native automated candidate changed its own recipe authority")
    validated_catalog(old_catalog)
    paths = changed_entries(repo, base, candidate)
    if formatter is None:
        formatter = ["cargo", "fmt", "--manifest-path", "core/Cargo.toml"]
    if not formatter or any(not isinstance(s, str) or not s for s in formatter):
        raise Refused("invalid native formatter invocation")
    with tempfile.TemporaryDirectory(prefix="excession-native-replay-") as root:
        replay = Path(root) / "base"
        attached = False
        try:
            git(repo, "-c", "core.hooksPath=/dev/null", "worktree",
                "add", "--detach", str(replay), base)
            attached = True
            run(formatter, cwd=replay, timeout=180)
            replayed = changed_entries_in_worktree(replay)
            if replayed != paths:
                raise Refused(f"native replay changed {replayed}, not candidate paths {paths}")
            for path in paths:
                if (replay / path).read_bytes() != git_blob(repo, candidate, path):
                    raise Refused(f"formatter replay differs from candidate Git blob: {path}")
        finally:
            if attached:
                git(repo, "worktree", "remove", "--force", str(replay))
    return {
        "schema": RECEIPT, "recipe_id": "native-rustfmt",
        "catalog_sha256": hashlib.sha256(old_catalog).hexdigest(),
        "base_commit": base,
        "base_tree": git(repo, "rev-parse", base + "^{tree}").decode().strip(),
        "candidate_commit": candidate,
        "candidate_tree": git(repo, "rev-parse", candidate + "^{tree}").decode().strip(),
        "changed_paths": paths, "native_replay": "EXACT_GIT_BLOB_MATCH",
        "status": "REPRODUCED_PENDING_INDEPENDENT_CI",
        "remaining_checks": ["Verify", "Excession GitHub evidence", "branch-protection"],
        "ready_to_merge": False, "semantic_equivalence_proved": False,
    }


def changed_entries_in_worktree(repo: Path) -> list[str]:
    raw = git(repo, "status", "--porcelain", "-z", "--untracked-files=all")
    statuses = raw.split(b"\0")
    result = []
    for record in statuses:
        if not record:
            continue
        if not record.startswith(b" M ") or b"\0" in record:
            raise Refused(f"native replay modified unsupported tracked/untracked source: {record[:120]!r}")
        path = record[3:].decode("utf-8")
        if not path.startswith("core/") or not path.endswith(".rs"):
            raise Refused(f"native replay changed unapproved path: {path}")
        result.append(path)
    return sorted(result)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["catalog", "verify"])
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--base")
    parser.add_argument("--candidate")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        repo = args.repo.resolve(strict=True)
        if args.command == "catalog":
            result = validated_catalog((repo / CATALOG).read_bytes())
        else:
            if not args.base or not args.candidate:
                raise Refused("verify requires --base and --candidate")
            result = verify(repo, args.base, args.candidate)
        output = json.dumps(result, indent=2, sort_keys=True) + "\n"
        if args.report:
            args.report.write_text(output, encoding="utf-8")
        print(output, end="")
        return 0
    except (Refused, OSError, UnicodeError) as exc:
        print(f"native improvement REFUSED: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
