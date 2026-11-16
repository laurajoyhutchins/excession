#!/usr/bin/env python3
"""Evaluate one pinned two-revision impact result against Git + CPython AST.

The oracle's denominator is in-repository *static Python import* paths.
Neither analysis demonstrates runtime dependency closure or test-skipping safety.
Timing includes CLI startup/index work and separate oracle Git/AST extraction;
they are measured costs, not comparable implementation speedups.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from dependency_oracle import module_name, python_edges  # noqa: E402


def call(*argv: object) -> bytes:
    cmd = [str(a) for a in argv]
    proc = subprocess.run(cmd, capture_output=True, check=False, timeout=240)
    if proc.returncode:
        raise RuntimeError(
            f"{cmd[:6]!r} exited {proc.returncode}: "
            f"{proc.stderr.decode('utf-8', 'replace')[:1600]}"
        )
    return proc.stdout


def git(repo: Path, *arguments: str) -> str:
    return call("git", "-C", repo, *arguments).decode("utf-8").strip()


def inventory(repo: Path, revision: str) -> tuple[str, dict[str, tuple[str, str]]]:
    commit = git(repo, "rev-parse", revision)
    if commit != revision:
        raise ValueError(f"unresolved pin {revision!r}; supply a literal commit SHA")
    tree = git(repo, "rev-parse", f"{revision}^{{tree}}")
    raw = call("git", "-C", repo, "ls-tree", "-r", "-z", revision)
    entries = {}
    for row in raw.split(b"\0"):
        if not row:
            continue
        meta, name = row.split(b"\t", 1)
        mode, kind, blob = meta.decode("ascii").split()
        if kind == "blob":
            entries[name.decode("utf-8")] = (blob, mode)
    return tree, entries


def independent_edges(
    repo: Path, revision: str, files: dict[str, tuple[str, str]], roots: set[str],
) -> tuple[set[tuple[str, str]], int, int]:
    selected = [
        p for p in files if p.endswith(".py") and p.split("/")[0] in roots
    ]
    modules = {module_name(p): p for p in selected}
    if len(modules) != len(selected):
        raise ValueError("ambiguous Python module name in held-out fixture")
    source = {
        p: call("git", "-C", repo, "show", f"{revision}:{p}").decode("utf-8")
        for p in selected
    }
    edges, unresolved = python_edges(source, modules)
    return {
        (modules[a], modules[b]) for a, b, _line, _syntax in edges
    }, len(selected), len(unresolved)


def reachable(edges: set[tuple[str, str]], changed: set[str]) -> set[str]:
    by_target: dict[str, set[str]] = {}
    for source, target in edges:
        by_target.setdefault(target, set()).add(source)
    seen = set(changed)
    affected = set()
    queue = list(sorted(changed))
    while queue:
        target = queue.pop(0)
        for source in sorted(by_target.get(target, ())):
            if source not in seen:
                seen.add(source)
                affected.add(source)
                queue.append(source)
    return affected


def result_matches_oracle(
    *,
    invalid: list[str],
    source_missing: dict[str, list[str]],
    source_unexpected: dict[str, list[str]],
    source_complete: bool,
) -> bool:
    """Every ranked source must match the independent Git/AST closure."""
    return (
        source_complete
        and not invalid
        and not any(source_missing.values())
        and not any(source_unexpected.values())
    )


def evaluate(
    binary: Path, repo: Path, base: str, head: str, roots: str,
    repository_name: str | None = None,
) -> dict:
    root_set = set(roots.split(","))
    starts = time.perf_counter()
    before_tree, before = inventory(repo, base)
    after_tree, after = inventory(repo, head)
    changed = {p for p in before.keys() | after.keys()
               if before.get(p) != after.get(p)}
    by_revision: dict[str, dict] = {}
    for commit, tree, files in ((base, before_tree, before), (head, after_tree, after)):
        edges, source_count, unresolved = independent_edges(repo, commit, files, root_set)
        by_revision[commit] = {
            "tree": tree,
            "files": files,
            "edges": edges,
            "expected": reachable(edges, changed & files.keys()),
            "source_count": source_count,
            "unresolved_import_sites": unresolved,
        }
    oracle_wall_s = time.perf_counter() - starts

    with tempfile.TemporaryDirectory(prefix="excession-real-impact-") as work:
        args = [
            str(binary), "impact", str(repo), "--base", base, "--head", head,
            "--roots", roots, "--db", str(Path(work) / "index.sqlite"), "--json",
        ]
        start = time.perf_counter()
        raw_report = call(*args)
        report = json.loads(raw_report)
        report_bytes = len(raw_report)
        impact_wall_s = time.perf_counter() - start

    if report.get("schema") != "excession.impact.v3" or "effects" in report:
        raise ValueError("not a source-only excession.impact.v3 report")
    if report.get("base_commit") != base or report.get("head_commit") != head:
        raise ValueError("reported revision differs from pinned input")
    if report.get("safe_to_skip_unlisted_tests") is not False:
        raise ValueError("unearned test-skipping claim")
    if report.get("coverage", {}).get("status") != "PARTIAL":
        raise ValueError("static candidates must not claim full coverage")

    source_actual: dict[str, set[str]] = {base: set(), head: set()}
    source_ranks: dict[tuple[str, str], int] = {}
    invalid = []
    # One finite rank-certified witness per affected source, never one per path.
    witnesses = report.get("candidate_source_impacts", [])
    source_count = report.get("candidate_source_count")
    source_complete = report.get("candidate_source_enumeration_complete")
    if source_count != len(witnesses) or source_complete is not True:
        invalid.append("distinct-source inventory is incomplete or miscounted")
    for item in witnesses:
        entity = item["entity"]
        commit, path = entity["source_commit"], entity["path"]
        if commit not in by_revision:
            invalid.append(f"source witness has unrequested revision {commit}")
            continue
        view = by_revision[commit]
        def source_valid(source: dict) -> bool:
            p = source["path"]
            return (source["source_commit"] == commit
                    and source["source_tree"] == view["tree"]
                    and (source["blob"], source["mode"]) == view["files"].get(p))
        hop = item["next_hop"]
        next_source, next_target = hop["source"], hop["target"]
        rank = item["distance_to_change"]
        key = (commit, path)
        if (key in source_ranks or rank < 1
                or item.get("assessment") != "CANDIDATE_PATH"
                or not source_valid(entity) or not source_valid(next_source)
                or not source_valid(next_target) or next_source != entity
                or hop["relation"] != "STATIC_IMPORT"
                or (path, next_target["path"]) not in view["edges"]
                or not hop["evidence"] or any(
                    observation.get("assessment") != "CANDIDATE"
                    or not observation.get("receipt_id")
                    for observation in hop["evidence"]
                )):
            invalid.append(f"invalid source witness {path}")
        else:
            source_actual[commit].add(path)
            source_ranks[key] = rank
    for item in witnesses:
        entity, hop = item["entity"], item["next_hop"]
        commit, path = entity["source_commit"], entity["path"]
        if (commit, path) not in source_ranks:
            continue
        target = hop["target"]["path"]
        rank = source_ranks[(commit, path)]
        if target in changed and target in by_revision[commit]["files"]:
            if rank != 1:
                invalid.append(f"seed not at rank 1 for {path}")
        elif source_ranks.get((commit, target)) != rank - 1:
            invalid.append(f"witness rank does not decrease for {path}")
    source_missing = {
        r: sorted(by_revision[r]["expected"] - source_actual[r])
        for r in source_actual
    }
    source_unexpected = {
        r: sorted(source_actual[r] - by_revision[r]["expected"])
        for r in source_actual
    }
    passed = result_matches_oracle(
        invalid=invalid,
        source_missing=source_missing,
        source_unexpected=source_unexpected,
        source_complete=source_complete,
    )
    return {
        "schema": "excession.heldout-impact-evaluation.v2",
        "status": "MATCH" if passed else "MISMATCH",
        "upstream_repository": repository_name,
        "base_commit": base,
        "head_commit": head,
        "roots": sorted(root_set),
        "oracle_scope": "in-tree, statically resolved Python imports only",
        "claim_limit": "No runtime reachability, safe-test-elision or behavioral equivalence proof",
        "changed_paths": sorted(changed),
        "oracle": {
            r: {
                "source_count": by_revision[r]["source_count"],
                "static_edge_count": len(by_revision[r]["edges"]),
                "unresolved_import_sites": by_revision[r]["unresolved_import_sites"],
                "expected_affected_paths": sorted(by_revision[r]["expected"]),
            } for r in (base, head)
        },
        "actual": {r: sorted(source_actual[r]) for r in (base, head)},
        "missing": source_missing,
        "unexpected": source_unexpected,
        "invalid_hops": invalid[:50],
        "source_impacts": {
            "reported_count": source_count,
            "complete_in_observed_graph": source_complete,
            "actual": {r: sorted(source_actual[r]) for r in source_actual},
            "missing": source_missing,
            "unexpected": source_unexpected,
        },
        "impact_coverage": report["coverage"],
        "impact_report_bytes": report_bytes,
        "candidate_source_count": source_count,
        "candidate_source_enumeration_complete": source_complete,
        "measured_wall_seconds": {
            "oracle_git_and_ast": round(oracle_wall_s, 3),
            "excession_impact_with_indexing": round(impact_wall_s, 3),
        },
        "timing_warning": "Different operations; do not interpret their ratio as a speedup.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--roots", default="h11,tests")
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    try:
        for revision in (args.base, args.head):
            if len(revision) != 40 or any(ch not in "0123456789abcdef" for ch in revision):
                raise ValueError("revision must be a literal lowercase SHA-1")
        result = evaluate(args.bin.resolve(), args.repo.resolve(),
                          args.base, args.head, args.roots)
    except Exception as exc:
        result = {
            "schema": "excession.heldout-impact-evaluation.v1",
            "status": "ERROR",
            "error": f"{type(exc).__name__}: {exc}",
            "base_commit": args.base,
            "head_commit": args.head,
        }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["status"] == "MATCH" else 1


if __name__ == "__main__":
    raise SystemExit(main())
