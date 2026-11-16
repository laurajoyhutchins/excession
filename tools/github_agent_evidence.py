#!/usr/bin/env python3
"""Publish an auditable GitHub Actions envelope around native Excession reports."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

SHA = re.compile(r"[0-9a-f]{40}\Z")
MAX_REPORT = 4 * 1024 * 1024


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), *args], text=True, timeout=30
    ).strip()


def identity(repo: Path, sha: str) -> dict[str, str]:
    if not SHA.fullmatch(sha):
        raise ValueError("expected an exact lowercase 40-hex commit")
    commit = git(repo, "rev-parse", "--verify", f"{sha}^{{commit}}")
    tree = git(repo, "rev-parse", "--verify", f"{sha}^{{tree}}")
    if commit != sha or not SHA.fullmatch(tree):
        raise ValueError("Git source identity mismatch")
    return {"commit": commit, "tree": tree}


def is_regular(repo: Path, sha: str, path: str) -> bool:
    if not path or "," in path or "\0" in path or path.startswith("/"):
        return False
    if any(p in ("", ".", "..") for p in path.split("/")):
        return False
    result = subprocess.run(
        ["git", "-C", str(repo), "ls-tree", sha, "--", path],
        check=True, capture_output=True, text=True, timeout=30
    )
    return any(
        line.startswith(("100644 blob ", "100755 blob "))
        and line.partition("\t")[2] == path
        for line in result.stdout.splitlines()
    )


def choose_focus(repo: Path, base: str, head: str, given: str) -> tuple[list[str], int]:
    changed = subprocess.check_output(
        ["git", "-C", str(repo), "diff", "--no-renames", "--name-only",
         "--diff-filter=ACMRT", "-z", base, head, "--"], timeout=30
    ).decode("utf-8").split("\0")
    changed = sorted(set(p for p in changed if p))
    if given:
        paths = given.split(",")
        if len(paths) > 16 or len(paths) != len(set(paths)):
            raise ValueError("focus must contain 1..16 unique source paths")
        if not all(is_regular(repo, head, p) for p in paths):
            raise ValueError("focus must select tracked regular source paths")
        return paths, len(changed)
    eligible = [p for p in changed if is_regular(repo, head, p)]
    return eligible[:16], len(changed)


def native(binary: Path, args: list[str], file: Path) -> dict:
    result = subprocess.run([str(binary), *args], capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError(
            f"Excession {args[1]} refused ({result.returncode}): "
            + result.stderr.decode("utf-8", "replace")[:800]
        )
    if len(result.stdout) > MAX_REPORT:
        raise ValueError("native output exceeded 4 MiB")
    report = json.loads(result.stdout)
    if not isinstance(report, dict):
        raise ValueError("native output is not a JSON object")
    file.write_bytes(result.stdout)
    return report


def validate_verification(v: dict, base: dict, head: dict) -> None:
    expected = {
        "schema": "excession.agent-verification.v1",
        "base_commit": base["commit"],
        "base_tree": base["tree"],
        "candidate_commit": head["commit"],
        "candidate_tree": head["tree"],
        "ready_to_merge": False,
        "safe_to_skip_unlisted_tests": False,
    }
    for key, value in expected.items():
        if v.get(key) != value:
            raise ValueError("verification mismatch: " + key)
    if v.get("replay", {}).get("semantic_correctness") != "NOT_EVALUATED":
        raise ValueError("semantic correctness incorrectly upgraded")
    if not isinstance(v.get("coverage"), dict) or not isinstance(v.get("obligations"), list):
        raise ValueError("verification missing coverage or obligations")


def validate_context(v: dict, head: dict) -> None:
    if (v.get("schema") != "excession.agent-context.v1"
            or v.get("source_commit") != head["commit"]
            or v.get("source_tree") != head["tree"]
            or v.get("safe_to_skip_unlisted_tests") is not False):
        raise ValueError("context identity or safety mismatch")
    if not isinstance(v.get("test_candidates"), list):
        raise ValueError("context missing test candidates")
    for source in v.get("sources", []):
        if (source.get("source_commit") != head["commit"]
                or source.get("source_tree") != head["tree"]):
            raise ValueError("context source identity mismatch")


def record(path: Path) -> dict:
    data = path.read_bytes()
    return {"file": path.name, "sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data)}


def produce(args: argparse.Namespace) -> dict:
    repo = args.repo.resolve(strict=True)
    binary = args.binary.resolve(strict=True)
    tool_repo = args.tool_source.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    base, head = identity(repo, args.base), identity(repo, args.candidate)
    if base["commit"] == head["commit"]:
        raise ValueError("base and candidate must differ")
    tool = identity(tool_repo, args.tool_commit)
    paths, changed_count = choose_focus(repo, args.base, args.candidate, args.focus)

    verification_file = out / "verification.json"
    verification = native(binary, [
        "agent", "verify", "--repo", str(repo), "--base", base["commit"],
        "--candidate", head["commit"], "--limit", "64"
    ], verification_file)
    validate_verification(verification, base, head)

    context = None
    context_file = out / "context.json"
    if paths:
        cmd = ["agent", "context", "--repo", str(repo), "--rev", head["commit"],
               "--focus", ",".join(paths), "--limit", "64"]
        if args.task:
            cmd += ["--task", args.task]
        context = native(binary, cmd, context_file)
        validate_context(context, head)

    envelope = {
        "schema": "excession.github-evidence.v1",
        "repository": args.repository,
        "base": base,
        "candidate": head,
        "analysis_tool": {"repository": "laurajoyhutchins/excession", **tool},
        "workflow": {"run_id": os.getenv("GITHUB_RUN_ID"),
                     "attempt": os.getenv("GITHUB_RUN_ATTEMPT"),
                     "event": os.getenv("GITHUB_EVENT_NAME")},
        "changed_path_count": changed_count,
        "context_focus_paths": paths,
        "context_focus_truncated": (not args.focus and changed_count > len(paths)),
        "reports": {"verification": record(verification_file),
                    "context": record(context_file) if context is not None else None},
        "source_witnesses": verification.get("candidate_source_impacts"),
        "source_witnesses_available": "candidate_source_impacts" in verification,
        "suggested_checks": context.get("test_candidates") if context else None,
        "coverage": {"verification": verification["coverage"],
                     "context": context.get("coverage") if context else None},
        "obligations": verification["obligations"],
        "verification_status": verification.get("status"),
        "source_replay": verification.get("replay"),
        "ready_to_merge": False,
        "safe_to_skip_unlisted_tests": False,
    }
    (out / "report.json").write_text(
        json.dumps(envelope, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    summary = (
        "### Excession evidence (advisory)\n\n"
        f"- Repository: {args.repository}\n"
        f"- Base commit: {base['commit']}\n"
        f"- Candidate commit: {head['commit']}\n"
        f"- Verification: {envelope['verification_status']}; "
        f"{len(verification['obligations'])} unresolved obligation entries\n"
        f"- Context: {'generated' if context else 'not generated'}; "
        f"{len(paths)} focused sources\n"
        "- Semantic correctness not evaluated. Merge not authorized; "
        "unlisted tests are not verified safe to skip.\n"
        "- Full reports: download workflow artifact excession-github-evidence.\n"
    )
    (out / "summary.md").write_text(summary, encoding="utf-8")
    return envelope


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--tool-source", type=Path, required=True)
    parser.add_argument("--tool-commit", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--focus", default="")
    parser.add_argument("--task", default="")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = produce(args)
    except (ValueError, OSError, RuntimeError, subprocess.SubprocessError,
            json.JSONDecodeError) as error:
        print(f"Refused to publish evidence: {error}", file=sys.stderr)
        return 1
    print("Generated " + result["schema"] + " at " + result["candidate"]["commit"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
