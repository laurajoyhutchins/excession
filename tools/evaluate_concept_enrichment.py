#!/usr/bin/env python3
"""Evaluate *proposed* concept annotations against externally frozen decisions.

The oracle is an input to this program, not something Excession produces.
A successful consistency check does not authenticate the reviewer or prove
semantic correctness. For release claims the oracle must be independently
reviewed and frozen before inspecting model predictions.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import resource
import subprocess
import sys
from time import perf_counter
from typing import Any

LIMIT = 8 * 1024 * 1024
SUPPORTED = {"LINK_WARRANTED", "NO_LINK", "UNJUDGED", "UNSUPPORTED"}


def fail(message: str) -> None:
    raise ValueError(message)


def unique_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            fail(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def read(path: Path) -> dict[str, Any]:
    if path.stat().st_size > LIMIT:
        fail(f"JSON input exceeds 8 MiB: {path}")
    data = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_pairs,
                      parse_constant=lambda s: fail(f"invalid JSON constant: {s}"))
    if not isinstance(data, dict):
        fail(f"expected JSON object: {path}")
    return data


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                      allow_nan=False).encode("utf-8")


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def check_digest(data: dict[str, Any], field: str) -> None:
    actual = data.get(field)
    expected = digest({k: v for k, v in data.items() if k != field})
    if actual != expected:
        fail(f"noncanonical {field}: receipt tampered or incompatible serialization")


def git(repo: Path, *args: str) -> str:
    run = subprocess.run(["git", "-C", str(repo), *args], capture_output=True,
                         check=False, timeout=15)
    if run.returncode:
        fail(f"git {' '.join(args)}: {run.stderr.decode(errors='replace').strip()}")
    return run.stdout.decode("utf-8").strip()


def verify_source(repo: Path, commit: str, tree: str, path: str, blob: str) -> None:
    if not path or path.startswith("/") or "\\" in path or any(
        part in ("", ".", "..") for part in path.split("/")
    ):
        fail(f"unsafe source path: {path!r}")
    if git(repo, "rev-parse", "--verify", f"{commit}^{{tree}}") != tree:
        fail("oracle source tree does not match pinned commit")
    if git(repo, "rev-parse", "--verify", f"{commit}:{path}") != blob:
        fail(f"oracle Git blob mismatch: {path}")


def report(
    repo: Path,
    baseline: dict[str, Any],
    enriched: dict[str, Any],
    model: dict[str, Any],
    manifest: dict[str, Any],
    oracle: dict[str, Any],
    baseline_revision: str,
    semantic_revision: str,
) -> dict[str, Any]:
    """Compute denominators without silently discarding unsupported cases."""
    start = perf_counter()
    for doc, field in [
        (baseline, "report_id"), (enriched, "link_report_id"),
        (model, "model_id"), (manifest, "snapshot_id"), (oracle, "oracle_id"),
    ]:
        check_digest(doc, field)

    if (baseline.get("schema") != "excession.refactor-candidates.v1"
            or baseline.get("status") != "UNVERIFIED_REFACTOR_HYPOTHESES"
            or enriched.get("schema") != "excession.concepts-analysis-link.v1"
            or model.get("schema") != "excession.concepts-model.v1"
            or manifest.get("schema") != "excession.concepts-snapshot.v1"
            or oracle.get("schema") != "excession.concepts-oracle.v1"):
        fail("unsupported report schema or baseline evidence assessment")

    commit = baseline["source_commit"]
    tree = baseline["source_tree"]
    if any(x.get("source_commit") != commit for x in [enriched, model, manifest, oracle]):
        fail("mismatched source revision")
    if any(x.get("source_tree") != tree for x in [model, manifest, oracle]):
        fail("mismatched source tree")
    if (enriched.get("analysis_report_id") != baseline["report_id"]
            or enriched.get("snapshot_id") != manifest["snapshot_id"]
            or manifest.get("model_id") != model["model_id"]):
        fail("unbound baseline, compiled model, or snapshot")
    if enriched["analysis_kind"] != "REFACTOR_CANDIDATES":
        fail("unsupported analysis type")
    if enriched["coverage_status"] != "PARTIAL":
        fail("candidate-only enrichment cannot assert complete coverage")
    if git(repo, "rev-parse", "--verify", f"{commit}^{{tree}}") != tree:
        fail("source revision tree does not match Git")

    baseline_ids = [c["candidate_id"] for c in baseline["candidates"]]
    if len(baseline_ids) != len(set(baseline_ids)):
        fail("duplicate baseline candidate")
    claimed = {c["claim_id"] for c in model["claims"]}
    predictions: set[str] = set()
    for link in enriched["links"]:
        key = link["analysis_candidate_id"]
        if key not in baseline_ids or key in predictions:
            fail("missing or duplicated source-bound analysis candidate")
        if link["role"] != "CONCEPT_ANNOTATION" or link["assessment"] != "ANNOTATION_ONLY":
            fail("proposed concept link cannot be promoted to verified behavior")
        if not link["concept_claim_ids"] or any(cid not in claimed for cid in link["concept_claim_ids"]):
            fail("concept link references an absent or empty claim set")
        predictions.add(key)

    cases = oracle.get("cases")
    if not isinstance(cases, list):
        fail("oracle cases missing")
    adjudicated: dict[str, str] = {}
    for case in cases:
        key = case["candidate_id"]
        if key in adjudicated or key not in baseline_ids or case["expected"] not in SUPPORTED:
            fail("oracle contains invalid, repeated or unrelated candidate")
        if not case.get("rationale") or not case.get("reviewer"):
            fail("oracle must document independent rationale and reviewer declaration")
        evidence = case.get("source", [])
        if not evidence:
            fail("oracle must identify pinned Git source evidence")
        for item in evidence:
            verify_source(repo, commit, tree, item["path"], item["blob"])
        adjudicated[key] = case["expected"]
    if set(adjudicated) != set(baseline_ids):
        fail("oracle must account for every baseline candidate, including unknowns")

    tp = sum(key in predictions for key, val in adjudicated.items() if val == "LINK_WARRANTED")
    fn = sum(key not in predictions for key, val in adjudicated.items() if val == "LINK_WARRANTED")
    fp = sum(key in predictions for key, val in adjudicated.items() if val == "NO_LINK")
    unresolved = sum(val == "UNJUDGED" for val in adjudicated.values())
    unsupported = sum(val == "UNSUPPORTED" for val in adjudicated.values())
    # A reviewer's declaration is not a signature. Do not claim independently
    # established ground truth from this machine-consistency check.
    output = {
        "schema": "excession.concepts-evaluation.v1",
        "baseline_revision": baseline_revision,
        "semantic_revision": semantic_revision,
        "oracle_manifest_id": oracle["oracle_id"],
        "corpus": [{"repo": oracle.get("repository", "local-git-repository"), "commit": commit, "purpose": "CONTRACT_NEGATIVE"}],
        "metrics": {
            "true_positive": tp, "false_positive": fp, "false_negative": fn,
            "unresolved": unresolved, "wall_seconds": perf_counter() - start,
            "peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024,
        },
        "unsupported_cases": unsupported,
        "tool_versions": {
            "python": sys.version.split()[0],
            "measurement_scope": "evaluation-only, excludes upstream indexing and compilation",
            "oracle_authentication": "reviewer declaration not independently authenticated",
        },
        "evaluation_status": "SYNTHETIC_SCHEMA_FIXTURE",
    }
    output["experiment_id"] = digest(output)
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--enriched", required=True, type=Path)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--oracle", required=True, type=Path)
    parser.add_argument("--baseline-revision", required=True)
    parser.add_argument("--semantic-revision", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    value = report(
        args.repo, read(args.baseline), read(args.enriched), read(args.model),
        read(args.manifest), read(args.oracle),
        args.baseline_revision, args.semantic_revision,
    )
    args.out.write_bytes(canonical(value) + b"\n")


if __name__ == "__main__":
    main()
