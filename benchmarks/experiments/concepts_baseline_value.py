#!/usr/bin/env python3
"""Evaluate the *incremental* value of semantic concepts over native SCIP.

Uses two independently generated Excession outputs from the SAME native SCIP
index: the existing refactor scip-suggest baseline and the concepts model.
Source-bound native interpretation has already been checked by the separate
concepts_scip_oracle. This does not treat reference cycles as architectural
ground truth, and cannot score semantic accuracy without external judgments.

Generate review packets *without* prediction labels, so later reviewers may
evaluate architectural meaning. Because these cases were selected after the
prototype was viewed, these packets are DEVELOPMENT CALIBRATION, not held-out
independent ground truth. No default-enable recommendation is permitted.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
LIMIT = 20 * 1024 * 1024
MAX_REVIEW_NEGATIVES = 8


def fail(message: str) -> None:
    raise ValueError(message)


def unique_object(items: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for k, v in items:
        if k in value:
            fail(f"duplicate JSON field: {k}")
        value[k] = v
    return value


def read(path: Path) -> dict[str, Any]:
    if not path.is_file() or path.stat().st_size > LIMIT:
        fail(f"missing or oversized source evidence file: {path}")
    value = json.loads(path.read_bytes(), object_pairs_hook=unique_object,
                       parse_constant=lambda x: fail(f"nonfinite JSON value: {x}"))
    if not isinstance(value, dict):
        fail("evaluation input must be a JSON object")
    return value


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, ensure_ascii=True,
                      separators=(",", ":"), allow_nan=False).encode("utf-8")


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def checked_id(value: dict[str, Any], field: str) -> None:
    if value.get(field) != digest({k: v for k, v in value.items() if k != field}):
        fail(f"{field} differs from canonical source report")


def identity(raw: dict[str, Any]) -> tuple[str, str]:
    path, blob = raw.get("path"), raw.get("blob")
    if (not isinstance(path, str) or not path or path.startswith("/") or
            "\\" in path or any(x in ("", ".", "..") for x in path.split("/")) or
            not isinstance(blob, str) or len(blob) != 40 or
            any(c not in "0123456789abcdef" for c in blob)):
        fail("source identity must use tracked path and 40-hex Git blob")
    return path, blob


def ordered_pair(*subjects: dict[str, Any]) -> tuple[tuple[str, str], tuple[str, str]]:
    if len(subjects) != 2:
        fail("a source-file hypothesis requires exactly two identities")
    a, b = sorted(identity(x) for x in subjects)
    if a == b:
        fail("a source-file pair cannot contain the same identity twice")
    return a, b


def baseline_pairs(report: dict[str, Any]) -> set[tuple]:
    if report.get("schema") != "excession.scip-refactor.v1":
        fail("unsupported source-bound SCIP baseline")
    if report.get("status") != "UNVERIFIED_SCIP_RELATIONS":
        fail("baseline evidence was promoted beyond native SCIP")
    checked_id(report, "report_id")
    pairs = set()
    for cand in report["candidates"]:
        if cand["kind"] != "reciprocal_symbol_references":
            continue
        if cand.get("status") != "UNVERIFIED_REFACTOR_HYPOTHESIS":
            fail("baseline hypothesis was promoted to verified")
        pair = ordered_pair(*cand["sources"])
        if pair in pairs:
            fail("duplicate baseline reciprocal source pair")
        pairs.add(pair)
    return pairs


def concept_pairs(model: dict[str, Any]) -> set[tuple]:
    if model.get("schema") != "excession.concepts-model.v1":
        fail("unsupported concept model")
    checked_id(model, "model_id")
    subjects = {s["subject_id"]: s for s in model["subjects"]}
    claims = {c["claim_id"]: c for c in model["claims"]}
    if len(subjects) != len(model["subjects"]) or len(claims) != len(model["claims"]):
        fail("duplicate subject or claim in concept model")
    pairs = set()
    for concept in model["concepts"]:
        if concept.get("review_status") != "UNREVIEWED":
            fail("source-only concept unexpectedly reviewed")
        members = []
        for claim_id in concept["membership_claims"]:
            claim = claims[claim_id]
            if (claim.get("predicate") != "REALIZES_CONCEPT" or
                    claim.get("assessment") != "PROPOSED" or
                    claim.get("behavioral_verification") != "UNTESTED" or
                    claim.get("concept_id") != concept["concept_id"] or
                    len(claim["subjects"]) != 1):
                fail("native observation became a verified or differently typed concept claim")
            members.append(subjects[claim["subjects"][0]])
        pair = ordered_pair(*members)
        if pair in pairs:
            fail("multiple concept IDs for the same source pair")
        pairs.add(pair)
    return pairs


def one_way_controls(batch: dict[str, Any], excluded: set[tuple]) -> list[tuple]:
    if (batch.get("schema") != "excession.concepts-observation-batch.v1" or
            batch.get("completeness") != "PARTIAL_CANDIDATES_ONLY"):
        fail("invalid native observation completeness")
    checked_id(batch, "batch_id")
    subjects = {s["subject_id"]: s for s in batch["subjects"]}
    directed: set[tuple] = set()
    for observation in batch["observations"]:
        if (observation["kind"], observation["status"],
                observation["native_relation"]) != (
                "SCIP_REFERENCE", "OBSERVED", "SCIP_SYMBOL_REFERENCE"):
            fail("unexpected native relation in SCIP control universe")
        left = identity(subjects[observation["source_subject_id"]])
        right = identity(subjects[observation["target_subject_id"]])
        directed.add((left, right))
    controls = {
        tuple(sorted((a, b)))
        for a, b in directed
        if (b, a) not in directed and a != b and
        tuple(sorted((a, b))) not in excluded
    }
    return sorted(controls, key=digest)[:MAX_REVIEW_NEGATIVES]


def case_id(name: str, commit: str, pair: tuple) -> str:
    return digest({"fixture": name, "source_commit": commit, "pair": pair})


def fixture_report(evidence: Path, stem: str) -> tuple[dict, list[dict]]:
    source = read(evidence / f"{stem}.report.json")
    model = read(evidence / f"{stem}.concepts.model.json")
    batch = read(evidence / f"{stem}.concepts.batch.json")
    manifest = read(evidence / f"{stem}.concepts.snapshot.json")
    oracle = read(evidence / f"{stem}.concepts.oracle.json")
    checked_id(manifest, "snapshot_id")
    if (oracle.get("status") != "SOURCE_BOUND_NATIVE_INTERPRETATION_AGREES" or
            source["source_commit"] != model["source_commit"] or
            source["source_commit"] != batch["source_commit"] or
            source["source_tree"] != model["source_tree"] or
            source["source_tree"] != batch["source_tree"] or
            manifest["model_id"] != model["model_id"] or
            manifest["input_batch_ids"] != [batch["batch_id"]] or
            oracle["source_commit"] != source["source_commit"] or
            oracle["source_tree"] != source["source_tree"] or
            oracle["indexer_semantic_correctness_independently_verified"] is not False):
        fail(f"{stem}: baseline, input, snapshot or native oracle mismatch")
    if (source["index_sha256"] != batch["inputs"][0]["configuration_id"] or
            oracle["indexer_receipt"] != batch["inputs"][0]["receipt_id"]):
        fail(f"{stem}: baseline SCIP index bytes differ from concept input")
    base = baseline_pairs(source)
    proposed = concept_pairs(model)
    controls = one_way_controls(batch, base | proposed)
    all_cases = sorted(base | proposed | set(controls), key=digest)
    packets = [{
        "case_id": case_id(stem, source["source_commit"], pair),
        "fixture": stem,
        "source_commit": source["source_commit"],
        "source_tree": source["source_tree"],
        "sources": [{"path": a, "blob": b} for a, b in pair],
        "review_question": "Do these files implement one coherent architectural responsibility, beyond their observed source references?",
        "assessment": "UNJUDGED",
        "reviewer": None,
        "rationale": None,
    } for pair in all_cases]
    output = {
        "fixture": stem,
        "source_commit": source["source_commit"],
        "source_tree": source["source_tree"],
        "baseline_report_id": source["report_id"],
        "model_id": model["model_id"],
        "baseline_reciprocal_pairs": len(base),
        "proposed_concept_pairs": len(proposed),
        "unchanged_pairs": len(base & proposed),
        "concept_only_pairs": len(proposed - base),
        "baseline_only_pairs": len(base - proposed),
        "one_way_negative_controls": len(controls),
        "native_indexer_semantics_verified": False,
        "architectural_labels_independently_adjudicated": 0,
        "artifact_bytes": {
            "scip_baseline": (evidence / f"{stem}.report.json").stat().st_size,
            "concept_model": (evidence / f"{stem}.concepts.model.json").stat().st_size,
            "concept_batch": (evidence / f"{stem}.concepts.batch.json").stat().st_size,
            "concept_snapshot": (evidence / f"{stem}.concepts.snapshot.json").stat().st_size,
        },
    }
    return output, packets


def evaluate(directories: list[Path]) -> tuple[dict, dict]:
    rows: list[dict] = []
    packets: list[dict] = []
    seen: set[str] = set()
    for directory in directories:
        stems = sorted(
            p.name.removesuffix(".concepts.model.json")
            for p in directory.glob("*.concepts.model.json")
        )
        if not stems:
            fail(f"no native SCIP concept models found: {directory}")
        for stem in stems:
            if stem in seen:
                fail(f"duplicate evaluation fixture: {stem}")
            seen.add(stem)
            row, reviews = fixture_report(directory, stem)
            rows.append(row)
            packets.extend(reviews)
    if not rows:
        fail("evaluation corpus must not be empty")
    totals = {key: sum(row[key] for row in rows) for key in [
        "baseline_reciprocal_pairs", "proposed_concept_pairs",
        "unchanged_pairs", "concept_only_pairs", "baseline_only_pairs",
        "one_way_negative_controls", "architectural_labels_independently_adjudicated",
    ]}
    for case in packets:
        if case["assessment"] != "UNJUDGED" or case["reviewer"] is not None:
            fail("review packet must not fake external adjudications")
    # In this candidate generator, no baseline-equivalent pair can establish
    # incremental architectural meaning without an independent reviewer.
    report = {
        "schema": "excession.concepts-incremental-value.v1",
        "scope": "DEVELOPMENT_CALIBRATION_NOT_HELD_OUT",
        "fixtures": rows,
        "totals": totals,
        "review_packet_count": len(packets),
        "architectural_precision": None,
        "architectural_recall": None,
        "user_usefulness_gain": None,
        "peak_rss_bytes": None,
        "end_to_end_wall_ms": None,
        "decision": "DO_NOT_ENABLE_BY_DEFAULT",
        "reason": "No independently adjudicated architectural value or measured decision improvement above native SCIP baseline",
    }
    report["report_id"] = digest(report)
    review = {
        "schema": "excession.concepts-unjudged-review-packets.v1",
        "selection": "ALL_BASELINE_AND_CONCEPT_PAIRS_PLUS_DETERMINISTIC_NONRECIPROCAL_CONTROLS",
        "held_out": False,
        "reviewer_identity_authenticated": False,
        "cases": sorted(packets, key=lambda c: c["case_id"]),
    }
    review["packet_id"] = digest(review)
    return report, review


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=Path, action="append")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--review-packets", required=True, type=Path)
    args = parser.parse_args()
    report, review = evaluate(args.evidence_dir)
    args.out.write_bytes(canonical(report) + b"\n")
    args.review_packets.write_bytes(canonical(review) + b"\n")
    print(json.dumps(report["totals"], sort_keys=True))
    print(report["decision"])


if __name__ == "__main__":
    main()
