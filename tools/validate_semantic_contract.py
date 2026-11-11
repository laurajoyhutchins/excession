#!/usr/bin/env python3
"""Offline JSON Schema/ODCS validation for semantic observation receipts.

This validates structure and unkeyed hashes, not Git objects or native-tool output.
The Rust `excession semantic export` command performs source replay before export.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys

from jsonschema import Draft201909Validator, Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "contracts/semantic/semantic-observations.v1.schema.json"
ODCS_PATH = ROOT / "contracts/semantic/semantic-observations.odcs.json"
UPSTREAM_PATH = ROOT / "contracts/vendor/odcs-json-schema-v3.1.0-20260616.json"


def read_json(path: Path) -> dict:
    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def nonfinite(value):
        raise ValueError(f"nonfinite JSON literal: {value}")

    data = json.loads(
        path.read_text(encoding="utf-8"),
        object_pairs_hook=unique_pairs,
        parse_constant=nonfinite,
    )
    if not isinstance(data, dict):
        raise ValueError(f"expected JSON object: {path}")
    return data


def digest(value: dict) -> str:
    # Matches the Rust core's canonical JSON for this contract's string/integer data.
    canonical = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def validate_catalog() -> None:
    data_schema = read_json(SCHEMA_PATH)
    odcs_schema = read_json(UPSTREAM_PATH)
    odcs = read_json(ODCS_PATH)
    Draft202012Validator.check_schema(data_schema)
    Draft201909Validator.check_schema(odcs_schema)
    Draft201909Validator(odcs_schema).validate(odcs)

    if odcs["apiVersion"] != "v3.1.0":
        raise ValueError("ODCS apiVersion drifted from the pinned schema")
    declared = {p["name"] for p in odcs["schema"][0]["properties"]}
    actual = set(data_schema["required"])
    if declared != actual:
        raise ValueError(f"ODCS field catalogue drift: {sorted(declared ^ actual)}")
    paths = [
        p["value"] for p in odcs["customProperties"]
        if p["property"] == "jsonSchemaPath"
    ]
    if paths != [SCHEMA_PATH.relative_to(ROOT).as_posix()]:
        raise ValueError("ODCS must refer to the local versioned JSON Schema")


def validate_report(report: dict) -> None:
    Draft202012Validator(read_json(SCHEMA_PATH)).validate(report)
    records = report["records"]
    gaps = report["gaps"]
    if report["record_count"] != len(records) or report["gap_count"] != len(gaps):
        raise ValueError("declared counts differ from arrays")
    identities = [r["observation_id"] for r in records]
    if len(identities) != len(set(identities)):
        raise ValueError("duplicate semantic observation identity")
    if len(gaps) != len({(g["source_path"], g["status"]) for g in gaps}):
        raise ValueError("duplicate source gap")
    for observation in records:
        body = {k: v for k, v in observation.items() if k != "observation_id"}
        if digest(body) != observation["observation_id"]:
            raise ValueError("observation canonical digest mismatch")
    body = {k: v for k, v in report.items() if k != "report_id"}
    if digest(body) != report["report_id"]:
        raise ValueError("report canonical digest mismatch")


def main(argv: list[str]) -> int:
    validate_catalog()
    for path in argv:
        validate_report(read_json(Path(path)))
    print(f"VALIDATED: ODCS v3.1.0 + semantic JSON Schema; receipts={len(argv)}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except Exception as exc:
        # Fail closed; do not emit a misleading success receipt.
        print(f"REFUSED: {exc}", file=sys.stderr)
        raise SystemExit(2) from exc
