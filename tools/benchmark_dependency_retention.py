#!/usr/bin/env python3
"""Compare bounded native-reference retention against the *same* Git/SCIP corpus.

This measures loss of already-observed edges at the agent context boundary.
It does not measure completeness or semantic validity of the SCIP producer.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

K = 16
FOCUS = (
    "h11/_connection.py", "h11/_events.py", "h11/_headers.py",
    "h11/_readers.py", "h11/_writers.py",
)


def observed_id(relation):
    if relation.get("kind") != "SCIP_REFERENCE":
        return None
    evidence = relation.get("evidence")
    if not isinstance(evidence, list) or not evidence or any(
        row.get("assessment") != "OBSERVED" for row in evidence
    ):
        raise ValueError("native reference has no valid observed receipt")
    record = relation.get("native_observation")
    if not isinstance(record, dict):
        raise ValueError("native reference has no producer observation identity")
    snapshot, identifier = record.get("snapshot_id"), record.get("observation_id")
    if not isinstance(snapshot, str) or not isinstance(identifier, str):
        raise ValueError("native observation identity must be recorded")
    return (snapshot, identifier)


def scoped(relationships, focus):
    return [r for r in relationships if focus in (
        r["source"]["path"], r["target"]["path"]
    )]


def retention(full, selected):
    expected = {key for relation in full if (key := observed_id(relation)) is not None}
    returned = {key for relation in selected if (key := observed_id(relation)) is not None}
    if not returned.issubset(expected):
        raise ValueError("context invented a native observation")
    return len(returned), len(expected)


def verify_git_source(repo, commit, tree, relations):
    for relation in relations:
        for endpoint in ("source", "target"):
            source = relation[endpoint]
            if source.get("source_commit") != commit or source.get("source_tree") != tree:
                raise ValueError("source endpoint has a stale commit or tree")
            path = source["path"]
            if not path or path.startswith("/") or any(
                p in ("", ".", "..") for p in path.split("/")
            ):
                raise ValueError("invalid Git source path")
            actual = subprocess.run(
                ["git", "-C", str(repo), "rev-parse", "--verify", f"{commit}:{path}"],
                text=True, capture_output=True, timeout=15, check=True
            ).stdout.strip()
            if actual != source["blob"]:
                raise ValueError("source endpoint blob does not match Git")


def measure(full, returned, focus, k):
    eligible = scoped(full, focus)
    before = eligible[:k]  # exact previous first-N implementation
    old_hit, total = retention(eligible, before)
    new_hit, new_total = retention(eligible, returned)
    assert new_total == total
    if len(returned) != min(k, len(eligible)):
        raise ValueError("context budget or relationship-count mismatch")
    if total:
        expected_new = {key for relation in eligible if
                        (key := observed_id(relation)) is not None}
        first_k_native = [
            key for relation in eligible
            if (key := observed_id(relation)) is not None
        ][:k]
        expected_first = set(first_k_native)
        if {key for relation in returned if (key := observed_id(relation)) is not None} != expected_first:
            raise ValueError("priority policy did not retain first k native references")
    return {"focus": focus, "all_scoped_relations": len(eligible),
            "observed_reference_ids": total,
            "old_first_n_retained": old_hit,
            "new_priority_retained": new_hit,
            "old_recall": (old_hit / total if total else None),
            "new_recall": (new_hit / total if total else None),
            "returned_relation_count": len(returned)}


def fixture():
    def edge(kind, source, target, n):
        row = {"kind": kind, "source": {"path": source},
               "target": {"path": target}}
        if kind == "SCIP_REFERENCE":
            row["evidence"] = [{"assessment": "OBSERVED"}]
            row["native_observation"] = {
                "snapshot_id": "source-pinned", "observation_id": f"source-{n}"}
        return row
    rows = [
        edge("STATIC_IMPORT", "a.py", "b.py", 1),
        edge("STATIC_IMPORT", "a.py", "c.py", 2),
        edge("SCIP_REFERENCE", "a.py", "d.py", 3),
    ]
    new = [rows[2], rows[0]]
    result = measure(rows, new, "a.py", 2)
    assert (result["old_first_n_retained"], result["new_priority_retained"]) == (0, 1)
    assert (result["old_recall"], result["new_recall"]) == (0, 1)
    try:
        retention(rows, [edge("SCIP_REFERENCE", "a.py", "ghost.py", 100)])
        raise AssertionError("forged native reference was not rejected")
    except ValueError:
        pass
    print("bounded source-reference retention fixture: PASS")


def evaluate(args):
    repo, output = args.repo.resolve(), args.out.resolve()
    raw = args.analysis.read_bytes()
    full_report = json.loads(raw)
    commit = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
    ).strip()
    if commit != args.revision or full_report.get("source_commit") != commit:
        raise ValueError("full report is not the pinned Git commit")
    tree = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "HEAD^{tree}"], text=True
    ).strip()
    if full_report.get("source_tree") != tree:
        raise ValueError("full report Git tree mismatch")
    relationships = full_report["relationships"]
    cases = []
    native_total = 0
    old_hits = 0
    new_hits = 0
    for focus in FOCUS:
        context = subprocess.run(
            [str(args.excession), "agent", "context", "--repo", str(repo),
             "--rev", commit, "--roots", "h11", "--focus", focus,
             "--db", str(args.db), "--limit", str(K)],
            capture_output=True, text=True, timeout=180, check=True
        )
        report = json.loads(context.stdout)
        if report.get("schema") != "excession.agent-context.v1":
            raise ValueError("native context schema changed")
        if report.get("source_commit") != commit or report.get("source_tree") != tree:
            raise ValueError("native context source identity mismatch")
        returned = report["relationships"]
        verify_git_source(repo, commit, tree, returned)
        case = measure(relationships, returned, focus, K)
        if report["relationship_count"] != case["all_scoped_relations"]:
            raise ValueError("reported total relationship count mismatch")
        if report["observed_reference_count"] != case["observed_reference_ids"]:
            raise ValueError("reported total native reference count mismatch")
        if report["returned_observed_reference_count"] != case["new_priority_retained"]:
            raise ValueError("reported retained native reference count mismatch")
        cases.append(case)
        native_total += case["observed_reference_ids"]
        old_hits += case["old_first_n_retained"]
        new_hits += case["new_priority_retained"]
    if native_total == 0:
        raise ValueError("pinned external corpus contains no eligible native references")
    result = {
        "schema": "excession.native-reference-retention-benchmark.v1",
        "scope": "ALREADY_OBSERVED_SCIP_EDGES_NOT_SEMANTIC_DEPENDENCY_GROUND_TRUTH",
        "repository": "python-hyper/h11", "source_commit": commit,
        "source_tree": tree,
        "source_analysis_sha256": hashlib.sha256(raw).hexdigest(),
        "limit": K, "focus_paths": list(FOCUS), "cases": cases,
        "observed_reference_occurrences": native_total,
        "old_first_n_retained": old_hits, "new_priority_retained": new_hits,
        "old_observed_retention": old_hits / native_total,
        "new_observed_retention": new_hits / native_total,
        "native_semantics_verified": False,
        "source_graph_complete": False,
    }
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, indent=2))
    if new_hits <= old_hits:
        print("No demonstrated gain on preregistered focus paths", file=sys.stderr)
        return 3
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("fixture", "evaluate"))
    parser.add_argument("--repo", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--analysis", type=Path)
    parser.add_argument("--db", type=Path)
    parser.add_argument("--excession", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    if args.mode == "fixture":
        fixture()
        return 0
    if any(getattr(args, name) is None for name in
           ("repo", "revision", "analysis", "db", "excession", "out")):
        parser.error("evaluate requires repo/revision/analysis/db/excession/out")
    return evaluate(args)


if __name__ == "__main__":
    raise SystemExit(main())
