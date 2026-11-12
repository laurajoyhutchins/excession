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


def evaluate(binary: Path, repo: Path, base: str, head: str, roots: str) -> dict:
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
        report = json.loads(call(*args))
        impact_wall_s = time.perf_counter() - start

    if report.get("schema") != "excession.impact.v1":
        raise ValueError("not an excession.impact.v1 report")
    if report.get("base_commit") != base or report.get("head_commit") != head:
        raise ValueError("reported revision differs from pinned input")
    if report.get("safe_to_skip_unlisted_tests") is not False:
        raise ValueError("unearned test-skipping claim")
    if report.get("coverage", {}).get("status") != "PARTIAL":
        raise ValueError("static candidates must not claim full coverage")

    actual: dict[str, set[str]] = {base: set(), head: set()}
    invalid = []
    for effect in report["effects"]:
        entity = effect["entity"]
        commit = entity["source_commit"]
        if commit not in by_revision:
            invalid.append("claim uses an unrequested revision")
            continue
        view = by_revision[commit]
        def check_source(source: dict) -> bool:
            path = source["path"]
            return (source["source_commit"] == commit
                    and source["source_tree"] == view["tree"]
                    and (source["blob"], source["mode"]) == view["files"].get(path))
        if effect.get("assessment") != "CANDIDATE_PATH" or not check_source(entity):
            invalid.append(f"unearned/invalid source claim {entity.get('path')}")
            continue
        actual[commit].add(entity["path"])
        hops = effect["hops"]
        if not hops or hops[-1]["target"]["path"] not in changed:
            invalid.append(f"nonterminating path {entity['path']}")
        for hop in hops:
            a, b = hop["source"], hop["target"]
            if (hop["relation"] != "STATIC_IMPORT" or not check_source(a)
                    or not check_source(b)
                    or (a["path"], b["path"]) not in view["edges"]
                    or not hop["evidence"]
                    or any(item.get("assessment") != "CANDIDATE" or not item.get("receipt_id")
                           for item in hop["evidence"])):
                invalid.append(f"unsupported/invented hop {a['path']} -> {b['path']}")
    missing = {r: sorted(by_revision[r]["expected"] - actual[r]) for r in actual}
    unexpected = {r: sorted(actual[r] - by_revision[r]["expected"]) for r in actual}
    passed = not invalid and not any(missing.values()) and not any(unexpected.values())
    return {
        "schema": "excession.heldout-impact-evaluation.v1",
        "status": "MATCH" if passed else "MISMATCH",
        "upstream_repository": "python-hyper/h11",
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
        "actual": {r: sorted(actual[r]) for r in (base, head)},
        "missing": missing,
        "unexpected": unexpected,
        "invalid_hops": invalid[:50],
        "impact_coverage": report["coverage"],
        "candidate_effect_count": len(report["effects"]),
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
