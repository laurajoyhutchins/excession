#!/usr/bin/env python3
"""Run Excession against its own pinned Git source, without granting it test authority.

The Rust CLI supplies source-bound syntax, conservative non-Rust file candidates,
and rust-analyzer SCIP references. This driver checks source identities
independently with Git and retains replay receipts. Unknown edges stay unknown.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict, deque
import json
from pathlib import Path
import subprocess
import sys
from time import perf_counter
from typing import Any

DEFAULT_ROOTS = ("core/src", "core/adapters", "tests", "tools", "benchmarks")
ESSENTIAL_SUFFIXES = (".rs", ".py")
PARSER_STATUSES = frozenset({
    "PARSED", "PARSED_WITH_ERRORS", "UNSUPPORTED_LANGUAGE",
    "GRAMMAR_NOT_INSTALLED", "PARSER_ERROR", "RESOURCE_LIMIT",
})


class AnalysisError(RuntimeError):
    """A source, replay, or reporting invariant failed."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AnalysisError(message)


def run(*args: str, timeout: int = 300) -> bytes:
    try:
        process = subprocess.run(
            args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        raise AnalysisError(f"command timed out: {args[0]}") from exc
    if process.returncode:
        stderr = process.stderr.decode("utf-8", errors="replace")[-4000:]
        raise AnalysisError(
            f"{args[0]} failed with exit {process.returncode}: {stderr}"
        )
    return process.stdout


def git(repo: Path, *args: str) -> bytes:
    return run("git", "-C", str(repo), *args)


def git_string(repo: Path, *args: str) -> str:
    return git(repo, *args).decode("utf-8").strip()


def tracked_blobs(repo: Path, commit: str) -> dict[str, tuple[str, str]]:
    """Independent Git tree enumeration; values are (mode, blob OID)."""
    result: dict[str, tuple[str, str]] = {}
    for entry in git(repo, "ls-tree", "-r", "-z", "--full-tree", commit).split(b"\0"):
        if not entry:
            continue
        header, path = entry.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split(" ")
        if kind == "blob" and mode in ("100644", "100755"):
            filename = path.decode("utf-8")
            require(filename not in result, f"duplicate Git tree path: {filename}")
            result[filename] = (mode, oid)
    return result


def in_scope(path: str, roots: tuple[str, ...]) -> bool:
    return any(path == root or path.startswith(root + "/") for root in roots)


def independently_verify(
    repo: Path,
    revision: str,
    roots: tuple[str, ...],
    syntax: dict[str, Any],
    resolved: dict[str, Any],
    syntax_receipt: dict[str, Any],
    resolution_receipt: dict[str, Any],
    base_rev: str | None = None,
) -> dict[str, Any]:
    """Validate source bindings and report explicit advisory-only statistics."""
    commit = git_string(repo, "rev-parse", "--verify", revision + "^{commit}")
    tree = git_string(repo, "rev-parse", "--verify", commit + "^{tree}")
    require(commit == git_string(repo, "rev-parse", "HEAD"),
            "self-analysis must target the checked-out Git commit")
    require(not git(repo, "status", "--porcelain", "--untracked-files=all"),
            "self-analysis checkout is dirty")
    require(syntax["schema"] == "excession.syntax-observations.v1",
            "unexpected syntax schema")
    require(resolved["schema"] == "excession.syntax-dependencies.v1",
            "unexpected dependency schema")
    require(syntax["completeness"] == "CONSERVATIVE_SYNTAX_ONLY",
            "syntax report claims unsupported completeness")
    require(resolved["completeness"] == "CONSERVATIVE_CANDIDATES_ONLY",
            "dependency report claims unsupported completeness")
    require(tuple(syntax["roots"]) == roots, "syntax roots differ from selection")
    for report in (syntax, resolved):
        require(report["source_commit"] == commit, "report commit differs from Git")
        require(report["source_tree"] == tree, "report tree differs from Git")
    require(resolved["syntax_report_id"] == syntax["report_id"],
            "dependency report references a different syntax report")
    require(syntax_receipt["status"] == "REPARSED_MATCH"
            and syntax_receipt["report_id"] == syntax["report_id"],
            "syntax replay did not match")
    require(resolution_receipt["status"] == "REGENERATED_MATCH"
            and resolution_receipt["resolution_id"] == resolved["resolution_id"],
            "dependency replay did not match")

    tree_blobs = tracked_blobs(repo, commit)
    sources: dict[str, dict[str, Any]] = {}
    for item in syntax["sources"]:
        path = item["source_path"]
        require(path not in sources, f"duplicate syntax source: {path}")
        require(in_scope(path, roots), f"out-of-scope syntax source: {path}")
        require(item["status"] in PARSER_STATUSES, f"unknown parser status: {path}")
        require(path in tree_blobs, f"syntax path not in regular Git blobs: {path}")
        require((item["source_mode"], item["source_blob"]) == tree_blobs[path],
                f"syntax blob/mode differs from independent Git tree: {path}")
        sources[path] = item
    require(len(sources) == syntax["source_count"], "syntax source count mismatch")
    require("core/src/lib.rs" in sources and "core/src/bin/excession.rs" in sources,
            "core Rust entry points missing from self-analysis")

    expected_essentials = {
        path for path in tree_blobs
        if in_scope(path, roots)
        and path.endswith(ESSENTIAL_SUFFIXES)
        and not any(part in ("vendor", "node_modules") for part in path.split("/"))
    }
    require(expected_essentials <= sources.keys(),
            "Git-tracked Rust/Python sources omitted: "
            + ", ".join(sorted(expected_essentials - sources.keys())[:12]))

    edges = resolved["candidate_edges"]
    unresolved = resolved["unresolved_imports"]
    require(len(edges) == resolved["candidate_count"],
            "candidate count mismatch")
    require(len(unresolved) == resolved["unresolved_count"],
            "unresolved count mismatch")
    for entry in edges:
        for role in ("source", "target"):
            path = entry[role + "_path"]
            require(path in sources, f"candidate {role} outside scanned source: {path}")
            require(entry[role + "_blob"] == sources[path]["source_blob"],
                    f"candidate {role} Git binding mismatch: {path}")
    for entry in unresolved:
        path = entry["source_path"]
        require(path in sources
                and entry["source_blob"] == sources[path]["source_blob"],
                f"unresolved import not bound to Git source: {path}")

    changed: list[str] = []
    base: str | None = None
    if base_rev:
        base = git_string(repo, "rev-parse", "--verify", base_rev + "^{commit}")
        raw = git(repo, "diff", "--name-only", "-z", base, commit)
        changed = sorted(path.decode("utf-8") for path in raw.split(b"\0") if path)

    # This is a navigation hint over conservative edges, NOT a safe test selector.
    dependents: dict[str, set[str]] = defaultdict(set)
    for entry in edges:
        dependents[entry["target_path"]].add(entry["source_path"])
    reached = set(changed)
    queue = deque(changed)
    while queue:
        for parent in sorted(dependents[queue.popleft()]):
            if parent not in reached:
                reached.add(parent)
                queue.append(parent)

    status_counts = Counter(item["status"] for item in sources.values())
    language_counts = Counter(item["language"] for item in sources.values())
    unresolved_counts = Counter(item["reason"] for item in unresolved)
    unavailable_sources = [
        {"path": path, "status": item["status"]}
        for path, item in sorted(sources.items())
        if item["status"] != "PARSED"
    ]
    return {
        "schema": "excession.self-analysis.v1",
        "source_commit": commit,
        "source_tree": tree,
        "roots": list(roots),
        "syntax": {
            "report_id": syntax["report_id"],
            "audit": syntax_receipt["status"],
            "source_count": len(sources),
            "essential_rust_python_files": len(expected_essentials),
            "observed_imports": sum(len(item["imports"]) for item in sources.values()),
            "statuses": dict(sorted(status_counts.items())),
            "languages": dict(sorted(language_counts.items())),
            "nonfully_parsed_sources": unavailable_sources,
        },
        "dependencies": {
            "resolution_id": resolved["resolution_id"],
            "audit": resolution_receipt["status"],
            "candidate_edges": len(edges),
            "unresolved_imports": len(unresolved),
            "unresolved_reasons": dict(sorted(unresolved_counts.items())),
            "skipped_sources": len(resolved["skipped_sources"]),
            "completeness": "NOT_ESTABLISHED",
        },
        "change_impact": {
            "base_commit": base,
            "changed_paths": changed,
            "candidate_affected_sources": sorted(reached - set(changed)),
            "authority": "ADVISORY_ONLY",
        },
        "assurance": {
            "source_binding": "INDEPENDENT_GIT_TREE_MATCH",
            "syntax_replay": "REPARSED_MATCH",
            "resolution_replay": "REGENERATED_MATCH",
            "test_skipping_authorized": False,
            "behavioral_equivalence_established": False,
        },
    }


def scip_evidence(
    repo: Path,
    binary: Path,
    out: Path,
    summary: dict[str, Any],
    index: Path,
) -> dict[str, Any]:
    """Reuse Excession's SCIP/Git reader; do not implement Rust name resolution."""
    require(index.is_file() and not index.is_relative_to(repo),
            "decoded rust-analyzer SCIP must be outside source checkout")
    report, elapsed_ms = invoke_json(
        binary, out / "scip-report.json", "refactor", "scip-suggest",
        "--repo", str(repo), "--rev", summary["source_commit"],
        "--index", str(index), "--source-prefix", "core",
    )
    require(report["source_commit"] == summary["source_commit"]
            and report["source_tree"] == summary["source_tree"],
            "SCIP report differs from pinned Git revision")
    require(report["status"] == "UNVERIFIED_SCIP_RELATIONS",
            "SCIP reports stronger authority than supported")
    require(report["indexer"]["name"] == "rust-analyzer",
            "Rust source relationships must come from rust-analyzer")
    coverage = report["coverage"]
    require("rust" in coverage["languages"] and coverage["indexed_documents"] > 0,
            "Rust SCIP index contains no usable Rust source documents")
    receipt, replay_ms = invoke_json(
        binary, out / "scip-audit.json", "refactor", "scip-audit",
        "--repo", str(repo), "--rev", summary["source_commit"],
        "--index", str(index), "--source-prefix", "core",
        "--report", str(out / "scip-report.json"),
    )
    require(receipt["status"] == "REPLAYED_SCIP_INDEX"
            and receipt["report_id"] == report["report_id"]
            and receipt["source_commit"] == summary["source_commit"],
            "SCIP/Git replay differs from report")
    require(receipt["indexer_independently_executed"] is False,
            "SCIP replay must not misrepresent indexer execution")
    # Independently re-check the Git bindings emitted on every candidate.
    # Rust itself validates every indexed document, including those without
    # candidate edges. The Python check is deliberately a second guard only.
    tree = tracked_blobs(repo, summary["source_commit"])
    def check_source(source: dict[str, Any]) -> None:
        path = source["path"]
        require(path.startswith("core/") and path in tree,
                f"SCIP candidate outside the pinned Cargo project: {path}")
        require(source["blob"] == tree[path][1],
                f"SCIP candidate source identity differs from Git: {path}")
    for candidate in report["candidates"]:
        for source in candidate.get("sources", []):
            check_source(source)
        for ref in candidate.get("references", []):
            check_source(ref["from"])
            check_source(ref["to"])
        if "reference" in candidate:
            check_source(candidate["reference"]["from"])
            check_source(candidate["reference"]["to"])
    triage = report["triage"]
    require(triage["basis"] == "ADVISORY_RUST_ANALYZER_TOP_LEVEL_MODULE_SYMBOL",
            "unsupported SCIP triage basis")
    identifiers = [candidate["candidate_id"] for candidate in report["candidates"]]
    require(len(identifiers) == len(set(identifiers)), "duplicate SCIP candidate identity")
    structural = triage["likely_structural_module_candidate_ids"]
    review = triage["review_candidate_ids"]
    require(len(structural) == len(set(structural))
            and len(review) == len(set(review))
            and set(structural).isdisjoint(review)
            and set(structural) | set(review) == set(identifiers),
            "SCIP triage does not partition candidate identities")
    return {
        "source": "rust-analyzer/SCIP",
        "indexer_version": report["indexer"]["version"],
        "index_sha256": report["index_sha256"],
        "report_id": report["report_id"],
        "indexed_documents": coverage["indexed_documents"],
        "indexed_occurrences": coverage["indexed_occurrences"],
        "matched_cross_file_references": coverage["matched_cross_file_references"],
        "unresolved_references": coverage["unresolved_references"],
        "refactor_hypotheses": len(report["candidates"]),
        "likely_structural_module_links": len(structural),
        "other_review_candidates": len(review),
        "audit": receipt["status"],
        "indexer_independently_executed": False,
        "provenance": "INDEXER_OUTPUT_GIT_BOUND_NOT_ATTESTED",
        "authority": "ADVISORY_ONLY",
        "runtime_ms": {"ingest": elapsed_ms, "audit": replay_ms},
    }


def report_markdown(summary: dict[str, Any]) -> str:
    syntax = summary["syntax"]
    dependencies = summary["dependencies"]
    impact = summary["change_impact"]
    lines = [
        "## Excession self-analysis",
        "",
        f"Git commit: `{summary['source_commit']}`",
        "",
        f"- Scanned: {syntax['source_count']} files "
        f"({syntax['essential_rust_python_files']} tracked Rust/Python source files).",
        f"- Syntax observations: {syntax['observed_imports']} imports; "
        f"statuses: `{json.dumps(syntax['statuses'], sort_keys=True)}`.",
        f"- Conservative dependency candidates: {dependencies['candidate_edges']}; "
        f"unresolved imports: {dependencies['unresolved_imports']}; "
        f"skipped sources: {dependencies['skipped_sources']}.",
        f"- Unresolved reasons: `{json.dumps(dependencies['unresolved_reasons'], sort_keys=True)}`.",
        f"- Non-fully-parsed sources: `{json.dumps(syntax['nonfully_parsed_sources'], sort_keys=True)}`.",
        f"- Independent source binding: {summary['assurance']['source_binding']}.",
        f"- Excession replay: {syntax['audit']} / {dependencies['audit']}.",
        *([
            f"- Rust semantic source: {summary['scip']['source']} "
            f"({summary['scip']['indexed_documents']} documents, "
            f"{summary['scip']['indexed_occurrences']} occurrences, "
            f"{summary['scip']['matched_cross_file_references']} cross-file references; "
            f"{summary['scip']['unresolved_references']} unresolved; "
            f"{summary['scip']['refactor_hypotheses']} advisory hypotheses).",
            f"- SCIP candidate triage: {summary['scip']['other_review_candidates']} "
            f"other candidates; {summary['scip']['likely_structural_module_links']} "
            "likely crate-root/module structure links (retained, not discarded).",
            f"- SCIP/Git replay: {summary['scip']['audit']}.",
        ] if "scip" in summary else []),
        f"- Changed paths: {len(impact['changed_paths'])}; "
        f"candidate dependents: {len(impact['candidate_affected_sources'])}.",
        "",
        "**Not established:** complete dependencies, absence of impact, behavioral "
        "equivalence, or authorization to omit tests.",
        "",
        "Raw syntax, dependency, replay, and machine-readable summary artifacts "
        "are retained. Missing facts are never negative evidence.",
    ]
    return "\n".join(lines) + "\n"


def invoke_json(binary: Path, output: Path, *arguments: str) -> tuple[dict[str, Any], float]:
    start = perf_counter()
    raw = run(str(binary), *arguments)
    elapsed_ms = round((perf_counter() - start) * 1000, 3)
    parsed = json.loads(raw)
    require(isinstance(parsed, dict), "Excession returned a non-object JSON response")
    output.write_bytes(raw)
    return parsed, elapsed_ms


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--rev", default="HEAD")
    parser.add_argument("--base-rev", default=None)
    parser.add_argument("--roots", default=",".join(DEFAULT_ROOTS))
    parser.add_argument("--scip-index", type=Path, required=True,
                        help="JSON decoded from a rust-analyzer SCIP index")
    arguments = parser.parse_args()
    repo = arguments.repo.resolve()
    binary = arguments.bin.resolve()
    out = arguments.out.resolve()
    roots = tuple(arguments.roots.split(","))
    require(repo.is_dir() and (repo / ".git").exists(),
            "source must be an existing Git checkout")
    require(binary.is_file(), "built Excession CLI not found")
    require(not out.is_relative_to(repo), "evidence must be outside source checkout")
    require(all(roots) and len(roots) <= 32 and len(set(roots)) == len(roots),
            "invalid or duplicated analysis roots")
    require(not git(repo, "status", "--porcelain", "--untracked-files=all"),
            "source checkout is not clean")
    commit = git_string(repo, "rev-parse", "--verify", arguments.rev + "^{commit}")
    out.mkdir(parents=True, exist_ok=True)

    syntax, scan_ms = invoke_json(
        binary, out / "syntax.json", "syntax", "scan", "--repo", str(repo),
        "--rev", commit, "--roots", ",".join(roots),
    )
    syntax_receipt, syntax_audit_ms = invoke_json(
        binary, out / "syntax-audit.json", "syntax", "audit", "--repo", str(repo),
        "--report", str(out / "syntax.json"),
    )
    resolved, resolve_ms = invoke_json(
        binary, out / "dependencies.json", "syntax", "resolve",
        "--repo", str(repo), "--report", str(out / "syntax.json"),
    )
    resolution_receipt, resolution_audit_ms = invoke_json(
        binary, out / "dependencies-audit.json", "syntax", "resolve-audit",
        "--repo", str(repo), "--syntax-report", str(out / "syntax.json"),
        "--report", str(out / "dependencies.json"),
    )
    summary = independently_verify(
        repo, commit, roots, syntax, resolved, syntax_receipt,
        resolution_receipt, arguments.base_rev,
    )
    summary["observed_timings_ms"] = {
        "syntax_scan": scan_ms,
        "syntax_audit": syntax_audit_ms,
        "candidate_resolution": resolve_ms,
        "candidate_audit": resolution_audit_ms,
    }
    summary["scip"] = scip_evidence(
        repo, binary, out, summary, arguments.scip_index.resolve(),
    )
    (out / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8",
    )
    markdown = report_markdown(summary)
    (out / "summary.md").write_text(markdown, encoding="utf-8")
    print(markdown)


if __name__ == "__main__":
    try:
        main()
    except (AnalysisError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"self-analysis refused: {error}", file=sys.stderr)
        raise SystemExit(1) from error
