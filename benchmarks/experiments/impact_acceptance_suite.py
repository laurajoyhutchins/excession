#!/usr/bin/env python3
"""Fail-closed, source-pinned impact acceptance across independent Git repositories.

The independent denominator is literal, in-repository CPython AST imports.
The suite never certifies runtime causality, complete program dependencies,
safe test omission, or authenticated execution receipts.
"""
from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time

from impact_heldout import evaluate, independent_edges, inventory


SCHEMA = "excession.impact-acceptance-cases.v1"
REPORT_SCHEMA = "excession.impact-acceptance-suite.v1"
SHA1 = re.compile(r"[0-9a-f]{40}", re.ASCII)
SLUG = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", re.ASCII)
CASE_ID = re.compile(r"[a-z0-9][a-z0-9-]{1,63}", re.ASCII)
ROOT = re.compile(r"[A-Za-z_][A-Za-z0-9_.-]*", re.ASCII)
PATH = re.compile(r"[A-Za-z0-9_./+-]+", re.ASCII)
REQUIRED = frozenset({
    "id", "repository", "base", "head", "roots",
    "expected_changed_paths", "must_include_head_paths",
    "minimum_head_affected_paths",
})


@dataclass(frozen=True)
class Case:
    id: str
    repository: str
    base: str
    head: str
    roots: tuple[str, ...]
    expected_changed_paths: tuple[str, ...]
    must_include_head_paths: tuple[str, ...]
    minimum_head_affected_paths: int


def _paths(value: object, label: str, *, allow_empty: bool) -> tuple[str, ...]:
    if not isinstance(value, list) or len(value) > 100 or (not value and not allow_empty):
        raise ValueError(f"{label} must be a bounded nonempty list")
    if any(not isinstance(x, str) or not PATH.fullmatch(x)
           or x.startswith("/") or x.startswith("./") or ".." in x.split("/")
           or "//" in x or not x.endswith(".py") for x in value):
        raise ValueError(f"{label} includes an unsafe or unsupported Python path")
    if len(set(value)) != len(value):
        raise ValueError(f"{label} contains duplicates")
    return tuple(sorted(value))


def validate_manifest(value: object) -> tuple[Case, ...]:
    if not isinstance(value, dict) or set(value) != {"schema", "cases"}:
        raise ValueError("manifest fields do not match the acceptance contract")
    if value["schema"] != SCHEMA:
        raise ValueError("unknown acceptance manifest schema")
    raw = value["cases"]
    if not isinstance(raw, list) or not 2 <= len(raw) <= 16:
        raise ValueError("acceptance suite requires 2..16 independent cases")
    cases = []
    ids = set()
    for item in raw:
        if not isinstance(item, dict) or set(item) != REQUIRED:
            raise ValueError("case fields do not match the acceptance contract")
        identifier, repository = item["id"], item["repository"]
        if (not isinstance(identifier, str) or not CASE_ID.fullmatch(identifier)
                or identifier in ids):
            raise ValueError("invalid or duplicate case identifier")
        ids.add(identifier)
        if (not isinstance(repository, str) or not SLUG.fullmatch(repository)
                or any(x in repository.split("/") for x in (".", ".."))):
            raise ValueError("repository must be an explicit GitHub owner/repository")
        base, head = item["base"], item["head"]
        if (not isinstance(base, str) or not isinstance(head, str)
                or not SHA1.fullmatch(base) or not SHA1.fullmatch(head) or base == head):
            raise ValueError("base and head must be distinct exact 40-hex Git commit IDs")
        roots = item["roots"]
        if (not isinstance(roots, list) or not 1 <= len(roots) <= 8
                or any(not isinstance(x, str) or not ROOT.fullmatch(x) for x in roots)
                or len(set(roots)) != len(roots)):
            raise ValueError("roots must be distinct top-level Python source roots")
        changed = _paths(item["expected_changed_paths"], "expected_changed_paths",
                         allow_empty=False)
        required = _paths(item["must_include_head_paths"], "must_include_head_paths",
                          allow_empty=False)
        if not all(path.split("/")[0] in roots for path in required):
            raise ValueError("required head paths must live in selected source roots")
        minimum = item["minimum_head_affected_paths"]
        if type(minimum) is not int or not 1 <= minimum <= 10000:
            raise ValueError("minimum head impact must be a positive bounded integer")
        cases.append(Case(identifier, repository, base, head, tuple(roots),
                          changed, required, minimum))
    if len({case.repository for case in cases}) < 2:
        raise ValueError("suite must exercise at least two independent repositories")
    return tuple(cases)


def _command(*args: str, timeout: int = 180) -> bytes:
    process = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             check=False, timeout=timeout)
    if process.returncode:
        raise RuntimeError(f"{args[0]} {args[1:3]} exited {process.returncode}: "
                           f"{process.stderr.decode('utf-8', 'replace')[-1200:]}")
    return process.stdout


def _git(repo: Path, *args: str) -> str:
    return _command("git", "-C", str(repo), *args).decode("utf-8").strip()


def fetch_exact(case: Case, destination: Path) -> dict[str, str]:
    """Fetch a bounded Git history, verify both hashes and detached head."""
    destination.mkdir(parents=True)
    _git(destination, "init", "-q")
    _git(destination, "remote", "add", "origin",
         f"https://github.com/{case.repository}.git")
    _git(destination, "fetch", "-q", "--no-tags", "--depth=2", "origin", case.head)
    try:
        _git(destination, "cat-file", "-e", f"{case.base}^{{commit}}")
    except RuntimeError:
        _git(destination, "fetch", "-q", "--no-tags", "--depth=1", "origin", case.base)
    if _git(destination, "rev-parse", case.head) != case.head:
        raise ValueError("fetched head does not resolve to the pinned commit")
    if _git(destination, "rev-parse", case.base) != case.base:
        raise ValueError("fetched base does not resolve to the pinned commit")
    _git(destination, "checkout", "-q", "--detach", case.head)
    if _git(destination, "rev-parse", "HEAD") != case.head:
        raise ValueError("detached checkout does not match pinned head")
    if _git(destination, "status", "--porcelain"):
        raise ValueError("acceptance checkout is not clean")
    return {"base_tree": _git(destination, "rev-parse", f"{case.base}^{{tree}}"),
            "head_tree": _git(destination, "rev-parse", f"{case.head}^{{tree}}")}


def assess(case: Case, result: dict, independent_head_edges: set[tuple[str, str]],
           independent_head_files: set[str]) -> dict:
    """Score Excession against a separate Git/CPython AST oracle.

    Legacy explanation truncation is not a waiver of distinct-source coverage.
    """
    if result.get("schema") != "excession.heldout-impact-evaluation.v1":
        raise ValueError("unexpected independent evidence schema")
    if result.get("status") != "MATCH":
        raise ValueError(f"independent source/oracle mismatch: "
                         f"{str(result.get('missing', result.get('invalid_hops')))[:500]}")
    if result.get("base_commit") != case.base or result.get("head_commit") != case.head:
        raise ValueError("oracle evaluated a different Git revision")
    if result.get("upstream_repository") != case.repository:
        raise ValueError("oracle source repository identity mismatch")
    if set(result.get("roots", [])) != set(case.roots):
        raise ValueError("oracle evaluated the wrong source roots")
    changed = set(result.get("changed_paths", []))
    if changed != set(case.expected_changed_paths):
        raise ValueError(f"fixture source changes differ from pinned expectations: {sorted(changed)}")
    compact = result.get("compact_source_impacts", {})
    if compact.get("complete_in_observed_graph") is not True:
        raise ValueError("distinct source-impact inventory was truncated")
    if result.get("candidate_source_enumeration_complete") is not True:
        raise ValueError("source coverage is not complete within observed graph")
    expected = set(result["oracle"][case.head]["expected_affected_paths"])
    actual = set(compact["actual"][case.head])
    if (expected != actual or not set(case.must_include_head_paths).issubset(expected)
            or len(expected) < case.minimum_head_affected_paths):
        raise ValueError("head candidate set does not satisfy independent expectations")
    if result["invalid_hops"]:
        raise ValueError("unverified Git or producer evidence in source impact report")
    if any(compact[k][side] for k in ("missing", "unexpected")
           for side in (case.base, case.head)):
        raise ValueError("unexpected or missing distinct sources")
    if not independent_head_files.issuperset(expected):
        raise ValueError("oracle candidate path absent from pinned head tree")
    direct = {
        source for source, target in independent_head_edges
        if target in changed and source not in changed
    }
    if not direct.issubset(expected):
        raise ValueError("one-hop oracle not contained in transitive closure")
    reasons = Counter(str(g.get("reason", "UNKNOWN"))
                      for g in result["impact_coverage"]["gaps"])
    costs = result["measured_wall_seconds"]
    return {
        "status": "PASS",
        "base_commit": case.base,
        "head_commit": case.head,
        "source_scope": list(case.roots),
        "changed_paths": sorted(changed),
        "head_affected_sources": len(expected),
        "head_required_paths_found": list(case.must_include_head_paths),
        "candidate_source_count_both_revisions": result["candidate_source_count"],
        "source_enumeration_complete": True,
        "explanation_paths": result["candidate_effect_count"],
        "explanations_truncated": result["legacy_explanations_truncated"],
        "missing_vs_independent_ast": 0,
        "unexpected_vs_independent_ast": 0,
        "independent_unresolved_import_sites": result["oracle"][case.head]["unresolved_import_sites"],
        "excession_gap_reasons": dict(sorted(reasons.items())),
        "baselines": {
            "changed_files_only_candidates": len(changed),
            "direct_importers": len(direct),
            "independent_full_transitive": len(expected),
            "one_hop_missed_transitive_sources": len(expected - direct),
        },
        "cost": {
            "independent_git_ast_wall_seconds": costs["oracle_git_and_ast"],
            "excession_impact_with_indexing_wall_seconds": costs["excession_impact_with_indexing"],
            "excession_impact_json_bytes": result["impact_report_bytes"],
        },
        "assessment_limit": "STATIC_PYTHON_IMPORT_ONLY_NOT_SAFE_TO_SKIP_TESTS",
    }


def verify_agent_obligations(report: dict, case: Case, trees: dict[str, str]) -> dict:
    """Cross-repository agent verification must not silently grant merge authority."""
    if report.get("schema") != "excession.agent-verification.v1":
        raise ValueError("agent verify returned unexpected schema")
    if (report.get("base_commit") != case.base
            or report.get("candidate_commit") != case.head
            or report.get("base_tree") != trees["base_tree"]
            or report.get("candidate_tree") != trees["head_tree"]):
        raise ValueError("agent verification does not bind to the requested Git commits/trees")
    if (report.get("status") != "OPEN_OBLIGATIONS"
            or report.get("ready_to_merge") is not False
            or report.get("safe_to_skip_unlisted_tests") is not False):
        raise ValueError("agent verify gave unsupported correctness or merge authority")
    obligations = set(report.get("obligations", []))
    necessary = {
        "SEMANTIC_CORRECTNESS_NOT_EVALUATED",
        "INCOMPLETE_DEPENDENCY_COVERAGE",
        "TEST_SELECTION_NOT_PROVEN",
        "INDEPENDENT_TEST_RECEIPT_MISSING",
        "INDEPENDENT_BUILD_RECEIPT_MISSING",
    }
    if not necessary.issubset(obligations):
        raise ValueError("agent verify silently dropped required evidence obligations")
    if (report.get("replay", {}).get("source") != "RECOMPUTED_FROM_PINNED_GIT"
            or report.get("coverage", {}).get("status") != "PARTIAL"):
        raise ValueError("agent replay or coverage status is not source-bound")
    return {
        "status": report["status"],
        "ready_to_merge": False,
        "safe_to_skip_unlisted_tests": False,
        "obligations": sorted(obligations),
        "candidate_source_count": report["candidate_source_count"],
        "output_truncated": report["coverage"]["truncated"],
    }


def _write_json(path: Path, document: dict) -> str:
    data = (json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False) + "\n").encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return hashlib.sha256(data).hexdigest()


def execute_case(case: Case, binary: Path, root: Path) -> dict:
    evidence_dir = root / case.id
    evidence_dir.mkdir(parents=True, exist_ok=True)
    start = time.perf_counter()
    record: dict = {"id": case.id, "repository": case.repository,
                    "base_commit": case.base, "head_commit": case.head, "status": "FAIL"}
    try:
        with tempfile.TemporaryDirectory(prefix=f"excession-acceptance-{case.id}-") as tmp:
            repository = Path(tmp) / "source"
            trees = fetch_exact(case, repository)
            record.update(trees)
            head_tree, head_files = inventory(repository, case.head)
            if head_tree != trees["head_tree"]:
                raise ValueError("independent Git tree identity disagrees")
            result = evaluate(binary, repository, case.base, case.head,
                              ",".join(case.roots), repository_name=case.repository)
            record["independent_evaluation_sha256"] = _write_json(
                evidence_dir / "independent-evaluation.json", result
            )
            edges, modules, unresolved = independent_edges(
                repository, case.head, head_files, set(case.roots)
            )
            record["independent_modules"] = modules
            record["independent_unresolved_sites"] = unresolved
            record.update(assess(case, result, edges, set(head_files)))
            agent_started = time.perf_counter()
            agent_args = [
                str(binary), "agent", "verify",
                "--repo", str(repository),
                "--base", case.base,
                "--candidate", case.head,
                "--roots", ",".join(case.roots),
                "--db", str(Path(tmp) / "agent-verification.sqlite"),
                "--limit", "64",
            ]
            agent_raw = _command(*agent_args, timeout=180)
            agent_report = json.loads(agent_raw)
            record["agent_verification_sha256"] = _write_json(
                evidence_dir / "agent-verification.json", agent_report
            )
            record["agent_verification"] = verify_agent_obligations(
                agent_report, case, trees
            )
            record["cost"]["agent_verify_wall_seconds"] = round(
                time.perf_counter() - agent_started, 3
            )
            record["cost"]["agent_verify_report_bytes"] = len(agent_raw)
    except Exception as exc:
        record["status"] = "FAIL"
        record["failure"] = f"{type(exc).__name__}: {exc}"[:2000]
    record["total_case_wall_seconds"] = round(time.perf_counter() - start, 3)
    _write_json(evidence_dir / "case-result.json", record)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.manifest.stat().st_size > 256 * 1024:
        parser.error("acceptance manifest exceeds 256 KiB")
    cases = validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    binary = args.bin.resolve(strict=True)
    if not binary.is_file():
        parser.error("Excession executable is not a regular file")
    root = args.out_dir.resolve()
    root.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    results = []
    for case in cases:
        outcome = execute_case(case, binary, root)
        results.append(outcome)
        print(f"{case.id}: {outcome['status']}"
              + (f" ({outcome['failure']})" if "failure" in outcome else ""), flush=True)
    passed = all(x["status"] == "PASS" for x in results)
    summary = {
        "schema": REPORT_SCHEMA,
        "status": "PASS" if passed else "FAIL",
        "case_count": len(cases),
        "passed": sum(x["status"] == "PASS" for x in results),
        "failed": sum(x["status"] != "PASS" for x in results),
        "source_repositories": sorted({x.repository for x in cases}),
        "excession_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "manifest_sha256": hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
        "total_suite_wall_seconds": round(time.perf_counter() - started, 3),
        "cases": results,
        "assessment_limit": "STATIC_PYTHON_IMPORT_ONLY_NOT_SAFE_TO_SKIP_TESTS",
    }
    _write_json(root / "summary.json", summary)
    print(f"Acceptance: {summary['status']} ({summary['passed']}/{summary['case_count']})")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
