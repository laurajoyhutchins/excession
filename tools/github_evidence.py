#!/usr/bin/env python3
"""Publish a bounded GitHub-native envelope around Excession's existing agent evidence.

This adapter does not perform semantic verification or independently run checks.
It refuses mismatched Git objects and publishes no success-shaped report on error.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

OID = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^[0-9a-f]{64}$")
MAX_FOCUS = 16
MAX_OUTPUT_BYTES = 16 * 1024 * 1024
SCHEMA = "excession.github-evidence.v1"
ARTIFACT_PREFIX = "excession-evidence-v1"


class EvidenceError(RuntimeError):
    pass


def git(repo: Path, *args: str) -> bytes:
    command = ["git", "-C", str(repo), *args]
    result = subprocess.run(command, capture_output=True, check=False)
    if result.returncode:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise EvidenceError(f"Git command failed ({' '.join(args)}): {detail[:500]}")
    if len(result.stdout) > MAX_OUTPUT_BYTES:
        raise EvidenceError("Git output exceeded evidence adapter's 16 MiB bound")
    return result.stdout


def resolve_commit(repo: Path, ref: str) -> str:
    if ref.startswith("-") or not ref:
        raise EvidenceError("Invalid revision")
    value = git(repo, "rev-parse", "--verify", "--end-of-options", f"{ref}^{{commit}}").decode().strip()
    if not OID.fullmatch(value):
        raise EvidenceError("Git commit is not an exact 40-character SHA-1 OID")
    return value


def tree_oid(repo: Path, commit: str) -> str:
    value = git(repo, "rev-parse", "--verify", f"{commit}^{{tree}}").decode().strip()
    if not OID.fullmatch(value):
        raise EvidenceError("Git tree is not a SHA-1 OID")
    return value


def run_excession(binary: Path, *args: str) -> dict:
    command = [str(binary), *args]
    completed = subprocess.run(command, capture_output=True, check=False, timeout=420)
    if completed.returncode:
        error = completed.stderr.decode("utf-8", "replace")[:1500]
        raise EvidenceError(f"Excession returned {completed.returncode}: {error}")
    if len(completed.stdout) > MAX_OUTPUT_BYTES:
        raise EvidenceError("Excession JSON exceeded 16 MiB")
    try:
        result = json.loads(completed.stdout)
    except (UnicodeDecodeError, ValueError) as exc:
        raise EvidenceError("Excession did not emit valid UTF-8 JSON") from exc
    if not isinstance(result, dict):
        raise EvidenceError("Excession report must be a JSON object")
    return result


def require(condition: bool, message: str) -> None:
    if not condition:
        raise EvidenceError(message)


def changed_paths(repo: Path, base: str, head: str) -> list[str]:
    # No similarity-based rename elision: a rename is deletion + addition.
    raw = git(repo, "diff", "--no-renames", "--name-only", "-z", base, head)
    try:
        paths = [item.decode("utf-8") for item in raw.split(b"\0") if item]
    except UnicodeDecodeError as exc:
        raise EvidenceError("Non-UTF-8 Git paths cannot be safely represented in the JSON contract") from exc
    return sorted(set(paths))


def tracked_regular(repo: Path, head: str) -> set[str]:
    result = set()
    raw = git(repo, "ls-tree", "-r", "-z", head)
    for record in raw.split(b"\0"):
        if not record:
            continue
        meta, sep, path = record.partition(b"\t")
        require(bool(sep), "Invalid git ls-tree record")
        mode, kind, _oid = meta.split(b" ")
        if kind == b"blob" and mode in (b"100644", b"100755"):
            try:
                result.add(path.decode("utf-8"))
            except UnicodeDecodeError:
                continue
    return result


def validate_verification(report: dict, base: str, base_tree: str, head: str, head_tree: str, changed: list[str]) -> None:
    require(report.get("schema") == "excession.agent-verification.v1", "Unexpected verification schema")
    for key, expected in (
        ("base_commit", base), ("base_tree", base_tree),
        ("candidate_commit", head), ("candidate_tree", head_tree),
    ):
        require(report.get(key) == expected, f"Excession {key} disagrees with Git")
    require(isinstance(report.get("report_id"), str) and bool(DIGEST.fullmatch(report["report_id"])), "Invalid verification report ID")
    require(report.get("change_count") == len(changed), "Excession change inventory differs from Git --no-renames diff")
    require(report.get("ready_to_merge") is False, "Unexpected ready-to-merge assertion")
    require(report.get("safe_to_skip_unlisted_tests") is False, "Unexpected safe-to-skip assertion")
    require(report.get("status") in ("OPEN_OBLIGATIONS", "REPORTED_FAILURE"), "Unexpected verification status")
    require(report.get("replay", {}).get("semantic_correctness") == "NOT_EVALUATED", "Unexpected semantic verification assertion")
    require(report.get("coverage", {}).get("status") == "PARTIAL", "Unexpected verification coverage")
    require(isinstance(report.get("obligations"), list) and all(isinstance(x, str) for x in report["obligations"]), "Invalid verification obligations")


def validate_context(repo: Path, report: dict, head: str, tree: str, focus: list[str]) -> None:
    require(report.get("schema") == "excession.agent-context.v1", "Unexpected context schema")
    require(report.get("source_commit") == head and report.get("source_tree") == tree, "Context Git revision mismatch")
    require(report.get("coverage", {}).get("status") == "PARTIAL", "Unexpected context coverage")
    require(report.get("safe_to_skip_unlisted_tests") is False, "Unexpected context safety flag")
    require(isinstance(report.get("report_id"), str) and bool(DIGEST.fullmatch(report["report_id"])), "Invalid context report ID")
    sources = report.get("sources")
    require(isinstance(sources, list) and len(sources) == len(focus), "Missing context source witnesses")
    require({item.get("path") for item in sources} == set(focus), "Context focus paths differ")
    def check_git_source(source: dict) -> None:
        require(isinstance(source, dict), "Source witness must be an object")
        require(source.get("source_commit") == head and source.get("source_tree") == tree, "Context source identity mismatch")
        require(source.get("mode") in ("100644", "100755"), "Unsupported source file mode")
        path = source.get("path")
        require(isinstance(path, str) and bool(path), "Source witness path missing")
        blob = git(repo, "rev-parse", "--verify", f"{head}:{path}").decode().strip()
        require(source.get("blob") == blob and bool(OID.fullmatch(blob)), "Source witness blob differs from Git")

    for source in sources:
        check_git_source(source)
    checks = report.get("test_candidates")
    require(isinstance(checks, list) and len(checks) <= 64, "Context test suggestions missing or unbounded")
    for check in checks:
        require(isinstance(check, dict), "Invalid check suggestion")
        require(check.get("selection_verified") is False and check.get("reason") in ("PATH_NAME_HEURISTIC", "EXPLICIT_FOCUS_TEST"), "Test suggestions asserted as verified")
        check_git_source(check.get("source"))


def make_report(
    repo: Path,
    binary: Path,
    base_ref: str,
    head_ref: str,
    repository: str,
    tool_commit: str,
    roots: str = "",
    run_id: str = "",
) -> dict:
    require(bool(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository)), "Invalid repository full name")
    require(bool(OID.fullmatch(tool_commit)), "Tool must be pinned to an exact commit")
    base = resolve_commit(repo, base_ref)
    head = resolve_commit(repo, head_ref)
    require(base != head, "Evidence comparison requires two distinct commits")
    base_tree, head_tree = tree_oid(repo, base), tree_oid(repo, head)
    paths = changed_paths(repo, base, head)

    opts = ["--roots", roots] if roots else []
    verification = run_excession(
        binary, "agent", "verify", "--repo", str(repo), "--base", base,
        "--candidate", head, "--limit", "64", *opts,
    )
    validate_verification(verification, base, base_tree, head, head_tree, paths)

    regular = tracked_regular(repo, head)
    # Commas are a delimiter in agent context's --focus grammar. Do not
    # misrepresent these paths as analyzed.
    eligible = [p for p in paths if p in regular and "," not in p]
    focus = eligible[:MAX_FOCUS]
    context = None
    if focus:
        context = run_excession(
            binary, "agent", "context", "--repo", str(repo), "--rev", head,
            "--focus", ",".join(focus), "--limit", "64", *opts,
        )
        validate_context(repo, context, head, head_tree, focus)

    obligations = set(verification["obligations"])
    if len(eligible) > MAX_FOCUS:
        obligations.add("GITHUB_EVIDENCE_FOCUS_TRUNCATED")
    if any(p in regular and "," in p for p in paths):
        obligations.add("GITHUB_EVIDENCE_FOCUS_PATH_UNSUPPORTED")
    if context is None:
        obligations.add("GITHUB_EVIDENCE_NO_CONTEXT_WITNESSES")
    elif context["coverage"].get("truncated"):
        obligations.add("GITHUB_EVIDENCE_CONTEXT_TRUNCATED")
    if context is not None:
        for gap in context["coverage"].get("gaps", []):
            if isinstance(gap, dict) and isinstance(gap.get("reason"), str):
                obligations.add(gap["reason"])
    report = {
        "schema": SCHEMA,
        "repository": repository,
        "base": {"commit": base, "tree": base_tree},
        "head": {"commit": head, "tree": head_tree},
        "producer": {"name": "Excession", "repository": "laurajoyhutchins/excession", "source_commit": tool_commit},
        "source_witnesses": context["sources"] if context else [],
        "coverage": {
            "verification": verification["coverage"],
            "context": context["coverage"] if context else None,
            "changed_paths": len(paths),
            "focus_paths": len(focus),
            "focus_complete": len(focus) == len([p for p in paths if p in regular]),
        },
        "suggested_checks": context["test_candidates"] if context else [],
        "provenance": {
            "verification_report_id": verification["report_id"],
            "context_report_id": context["report_id"] if context else None,
            "github_run_id": run_id or None,
            "independent_checks_executed": False,
            "receipt_authenticity": "NOT_ATTESTED",
        },
        "unresolved_obligations": sorted(obligations),
        "verification": verification,
        "context": context,
        "status": verification["status"],
        "ready_to_merge": False,
        "safe_to_skip_unlisted_tests": False,
    }
    canonical = json.dumps(report, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
    report["report_sha256"] = hashlib.sha256(canonical).hexdigest()
    return report


def summary(report: dict) -> str:
    run_id = report["provenance"]["github_run_id"]
    url = f"https://github.com/{report['repository']}/actions/runs/{run_id}" if run_id else None
    artifact = f"{ARTIFACT_PREFIX}-{report['head']['commit'][:12]}"
    lines = [
        "### Excession source evidence",
        "",
        f"- Base: `{report['base']['commit']}`",
        f"- Head: `{report['head']['commit']}`",
        f"- Changed paths: {report['coverage']['changed_paths']}",
        f"- Source witnesses: {len(report['source_witnesses'])} (maximum {MAX_FOCUS})",
        f"- Suggested checks: {len(report['suggested_checks'])} (unverified)",
        f"- Unresolved obligations: {len(report['unresolved_obligations'])}",
        f"- Artifact: `{artifact}`, file `github-evidence.v1.json`",
    ]
    if url:
        lines.append(f"- [Workflow run and artifacts]({url})")
    lines += [
        "",
        "**This workflow produced a report, not a correctness or merge approval.**",
        "No independent build or test was executed by this evidence adapter.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tool-commit", required=True)
    parser.add_argument("--roots", default="")
    parser.add_argument("--run-id", default="")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    try:
        report = make_report(
            args.repo.resolve(), args.bin.resolve(), args.base, args.head,
            args.repository, args.tool_commit, args.roots, args.run_id,
        )
        (args.out / "github-evidence.v1.json").write_text(
            json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        (args.out / "summary.md").write_text(summary(report), encoding="utf-8")
        return 0
    except (EvidenceError, OSError, ValueError, subprocess.TimeoutExpired) as exc:
        (args.out / "failure.json").write_text(
            json.dumps({"schema": "excession.github-evidence-failure.v1", "error": str(exc)}, indent=2) + "\n",
            encoding="utf-8",
        )
        (args.out / "summary.md").write_text(
            "### Excession source evidence\n\n"
            "Evidence generation failed. No success-shaped evidence report was produced. "
            "See the retained `failure.json` diagnostic.\n",
            encoding="utf-8",
        )
        print(f"Evidence generation failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
