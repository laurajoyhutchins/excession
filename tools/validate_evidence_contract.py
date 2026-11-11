#!/usr/bin/env python3
"""Validate typed source-evidence query receipts under JSON Schema/ODCS 3.2.

This performs shape, canonical-digest, and cross-record consistency checks.
Only Excession's Git replay can authenticate source relationships. Schema
validation and an unkeyed hash do not attest compiler or runtime execution.
"""
from __future__ import annotations

import sys
from pathlib import Path

from jsonschema import Draft201909Validator, Draft202012Validator
from validate_semantic_contract import (
    ODCS_VERSION, ROOT, SCHEMA_PATH, UPSTREAM_PATH,
    digest, read_json, validate_catalog,
)

EVIDENCE_SCHEMA_PATH = ROOT / "contracts/evidence/evidence-query.v1.schema.json"
EVIDENCE_ODCS_PATH = ROOT / "contracts/evidence/evidence-query.odcs.json"


def validate_catalog_evidence() -> None:
    schema = read_json(EVIDENCE_SCHEMA_PATH)
    metadata = read_json(EVIDENCE_ODCS_PATH)
    upstream = read_json(UPSTREAM_PATH)
    Draft202012Validator.check_schema(schema)
    Draft201909Validator(upstream).validate(metadata)
    if metadata["apiVersion"] != ODCS_VERSION:
        raise ValueError("evidence ODCS version does not match pinned 3.2 schema")
    if len(metadata["schema"]) != 1 or metadata["schema"][0]["name"] != "evidence_query":
        raise ValueError("expected one evidence query dataset")
    fields = [x["name"] for x in metadata["schema"][0]["properties"]]
    if len(set(fields)) != len(fields) or set(fields) != set(schema["required"]):
        raise ValueError("evidence ODCS catalogue differs from JSON Schema")
    custom = {x["property"]: x["value"] for x in metadata["customProperties"]}
    if custom.get("jsonSchemaPath") != EVIDENCE_SCHEMA_PATH.relative_to(ROOT).as_posix():
        raise ValueError("evidence ODCS schema link differs")
    if custom.get("relatedSemanticContract") != SCHEMA_PATH.relative_to(ROOT).as_posix():
        raise ValueError("evidence ODCS must reference existing semantic v1")
    constraints = [x["id"] for x in metadata["context"]["constraints"]]
    if set(constraints) != {
        "no_type_promotion", "no_unsupported_completeness", "no_digest_attestation"
    } or len(constraints) != 3:
        raise ValueError("evidence ODCS provenance limits were lost")
    if schema["properties"]["static_replay"] != {"const": "REGENERATED_FROM_PINNED_GIT"}:
        raise ValueError("source replay guarantee was weakened")


def semantic_record(item: dict, candidate: bool) -> dict:
    record = {
        "kind": "IMPORTS",
        "subject": {
            "path": item["source_path"], "blob": item["source_blob"],
            "line": item["line"], "language": item["language"],
        },
        "specifier": item["specifier"],
        "assessment": {
            "status": "CANDIDATE" if candidate else "UNRESOLVED",
            "method": "SYNTAX_RESOLUTION",
            "rule": item["resolution"] if candidate else item["reason"],
        },
        "condition_evaluation": "NOT_EVALUATED",
    }
    if candidate:
        record["target"] = {"path": item["target_path"], "blob": item["target_blob"]}
    record["observation_id"] = digest(record)
    return record


def validate_evidence_query(report: dict) -> None:
    Draft202012Validator(read_json(EVIDENCE_SCHEMA_PATH)).validate(report)
    counts = {
        "static_candidates": "static_candidates",
        "unresolved_static_imports": "unresolved_imports",
        "scoped_source_gaps": "source_gaps",
        "compiler_declared_inputs": "compiler_inputs",
        "runtime_witness_records": "runtime_witnesses",
        "runtime_gaps": "runtime_gaps",
    }
    for key, field in counts.items():
        if report["counts"][key] != len(report[field]):
            raise ValueError(f"evidence counts disagree for {field}")

    static = read_json(SCHEMA_PATH)
    observation_validator = Draft202012Validator({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": static["$defs"],
        "$ref": "#/$defs/observation",
    })
    for item in report["static_candidates"]:
        observation_validator.validate(semantic_record(item, True))
    for item in report["unresolved_imports"]:
        observation_validator.validate(semantic_record(item, False))

    native = report["compiler_receipt"]
    if native is not None:
        if native["selected_inputs"] != len(report["compiler_inputs"]):
            raise ValueError("compiler selected input count mismatch")
        assumptions = native["assumptions"]
        if (assumptions["source-commit"] != report["source_commit"]
                or assumptions["source-tree"] != report["source_tree"]):
            raise ValueError("compiler declaration revision mismatch")

    runtime = report["runtime_receipt"]
    if runtime is not None:
        for item in report["runtime_witnesses"]:
            src = item["witness"]["source"]
            if item["selection"] == "PINNED_TARGET_PATH":
                if src["status"] != "GIT_BOUND" or src["path"] != report["focus"]["path"]:
                    raise ValueError("runtime target is not pinned focus")
            elif runtime["entrypoint"]["path"] != report["focus"]["path"]:
                raise ValueError("runtime execution context is not focus entrypoint")

    without_id = {key: value for key, value in report.items() if key != "query_id"}
    if digest(without_id) != report["query_id"]:
        raise ValueError("canonical query digest mismatch")


def main(paths: list[str]) -> int:
    validate_catalog()
    validate_catalog_evidence()
    for path in paths:
        report = read_json(Path(path))
        if report.get("schema") != "excession.evidence-query.v1":
            raise ValueError("expected typed source evidence query v1")
        validate_evidence_query(report)
    print(f"VALIDATED: ODCS {ODCS_VERSION}, semantic v1 + evidence query v1; receipts={len(paths)}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except Exception as exc:
        print(f"REFUSED: {exc}", file=sys.stderr)
        raise SystemExit(2) from exc
