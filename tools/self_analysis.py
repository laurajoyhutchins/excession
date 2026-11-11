#!/usr/bin/env python3
"""Run Excession against its own pinned Git source, without granting it test authority.

The Rust CLI supplies source-bound syntax and conservative dependency candidates.
This driver checks source identities independently with Git and retains both replay
receipts. Incomplete language support is reported, never treated as no dependency.
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


def native_boundary_evidence(
    repo: Path,
    binary: Path,
    out: Path,
    summary: dict[str, Any],
    declarations: tuple[tuple[str, Path], ...],
) -> list[dict[str, Any]]:
    """Check real Cargo depfile declarations through Excession's native boundary.

    The compiler-produced source set is configuration-specific. It is neither a
    direct import graph nor evidence of complete dependencies. The originating
    depfile is not a remotely authenticated compiler receipt.
    """
    source_commit = summary["source_commit"]
    source_tree = summary["source_tree"]
    tree = tracked_blobs(repo, source_commit)
    result: list[dict[str, Any]] = []
    for name, original in declarations:
        raw = original.read_bytes()
        declaration = json.loads(raw)
        require(declaration["schema"] == "excession.input-declaration.v1"
                and declaration["coverage"] == "DECLARED_INPUTS_ONLY"
                and declaration["language"] == "rust",
                f"{name}: unexpected native declaration contract")
        assumptions = declaration["assumptions"]
        require(assumptions["source-commit"] == source_commit
                and assumptions["source-tree"] == source_tree,
                f"{name}: native depfile source revision differs")
        require(assumptions["rust-source-evidence"]
                == "UNATTESTED_CARGO_DEPFILE_SOURCE_SET",
                f"{name}: unsupported compiler evidence kind")
        source_inputs = set()
        for item in declaration["inputs"]:
            path = item["path"]
            require(path in tree, f"{name}: input not tracked by Git: {path}")
            if item["role"] == "source":
                source_inputs.add(path)
        require(declaration["subject"] in source_inputs,
                f"{name}: compiler depfile omitted declared subject")
        preflight, _ = invoke_json(
            binary, out / f"native-{name}-preflight.json",
            "syntax", "native-preflight", "--declaration", str(original),
        )
        require(preflight["status"] == "DECLARATION_SHAPE_ONLY",
                f"{name}: native declaration preflight did not pass")
        boundary, _ = invoke_json(
            binary, out / f"native-{name}-comparison.json",
            "syntax", "native-compare", "--repo", str(repo),
            "--syntax-report", str(out / "syntax.json"),
            "--resolution-report", str(out / "dependencies.json"),
            "--declaration", str(original),
        )
        receipt, _ = invoke_json(
            binary, out / f"native-{name}-audit.json",
            "syntax", "native-audit", "--repo", str(repo),
            "--syntax-report", str(out / "syntax.json"),
            "--resolution-report", str(out / "dependencies.json"),
            "--declaration", str(original),
            "--report", str(out / f"native-{name}-comparison.json"),
        )
        require(boundary["source_commit"] == source_commit
                and boundary["source_tree"] == source_tree,
                f"{name}: source identities differ from self-analysis")
        require(boundary["provenance"] == "UNATTESTED_NATIVE_DECLARATION"
                and boundary["coverage"] == "SCOPED_COMPARISON_ONLY",
                f"{name}: boundary overstates source guarantees")
        require(receipt["status"] == "DECLARATION_AND_GIT_REPLAY_MATCH"
                and receipt["boundary_id"] == boundary["boundary_id"],
                f"{name}: native boundary replay did not match")
        pinned = boundary["pinned_inputs"]
        require(len(pinned) == len(declaration["inputs"]),
                f"{name}: native comparison omitted source inputs")
        for item in pinned:
            path = item["path"]
            require(path in tree
                    and (item["mode"], item["blob"]) == tree[path],
                    f"{name}: independent Git identity mismatch: {path}")
        result.append({
            "subject": declaration["subject"],
            "declaration_id": preflight["declaration_id"],
            "boundary_id": boundary["boundary_id"],
            "compiler_observed_source_files": len(source_inputs),
            "matching_candidate_edges": len(boundary["matching_edges"]),
            "static_only_candidate_edges": len(boundary["static_only_edges"]),
            "native_only_sources": len(boundary["native_only_sources"]),
            "unscanned_sources": len(boundary["unscanned_sources"]),
            "unresolved_syntax_imports": len(boundary["unresolved_imports"]),
            "source_input_paths": sorted(source_inputs),
            "provenance": "UNATTESTED_CARGO_DEPFILE_SOURCE_SET",
            "audit": receipt["status"],
            "authority": "ADVISORY_ONLY",
        })
    return result


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
        *[
            f"- Native Rust `{entry['subject']}`: "
            f"{entry['compiler_observed_source_files']} compiler-declared source paths, "
            f"{entry['matching_candidate_edges']} matching candidate edges, "
            f"{entry['static_only_candidate_edges']} static-only candidates, "
            f"{entry['unresolved_syntax_imports']} unresolved syntax imports; "
            f"replay {entry['audit']}."
            for entry in summary.get("native_boundaries", [])
        ],
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
    parser.add_argument("--native-lib", type=Path, default=None)
    parser.add_argument("--native-bin", type=Path, default=None)
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
    native_declarations = tuple(
        (name, path.resolve())
        for name, path in (("lib", arguments.native_lib), ("bin", arguments.native_bin))
        if path is not None
    )
    if native_declarations:
        require(len(native_declarations) == 2,
                "self-analysis native comparison requires both lib and bin inputs")
        summary["native_boundaries"] = native_boundary_evidence(
            repo, binary, out, summary, native_declarations,
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
