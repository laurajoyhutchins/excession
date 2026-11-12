"""Pure structural and relational checks for Excession security discovery v1.

No scanner is run here. Schema validity is not Git/source authentication, rule
applicability proof, or behavioral vulnerability verification. The Rust
security ingest/audit layer supplies source-bound evidence independently.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMAS = ROOT / "contracts" / "security"


def _read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_schema(kind: str, value: dict[str, Any]) -> None:
    schema = _read(SCHEMAS / f"{kind}.v1.schema.json")
    Draft202012Validator.check_schema(schema)
    Draft202012Validator(schema).validate(value)


def _contains(scope: str, path: str) -> bool:
    """Repository-relative literal directory containment; no globs or symlinks."""
    return scope == "." or path == scope or path.startswith(scope + "/")


def _overlaps(left: str, right: str) -> bool:
    return _contains(left, right) or _contains(right, left)


def _require(condition: bool, reason: str) -> None:
    if not condition:
        raise ValueError(reason)


def validate_bundle(bundle: dict[str, Any]) -> dict[str, int]:
    """Check relations; leave all claims of scanner accuracy to other evidence."""
    _require(set(bundle) == {"scan", "coverages", "candidates"},
             "expected scan, coverages and candidates only")
    scan = bundle["scan"]
    validate_schema("scan", scan)
    _require(isinstance(bundle["coverages"], list) and
             isinstance(bundle["candidates"], list),
             "coverage and candidate tables must be lists")
    status = scan["execution"]["status"]
    if scan["scanner"]["engine"] == "codeql" and scan["report"] is not None:
        _require(scan["report"]["format"] == "SARIF_2_1_0", "CodeQL report format mismatch")
    if scan["scanner"]["engine"] == "semgrep" and scan["report"] is not None:
        _require(scan["report"]["format"] == "SEMGREP_JSON", "Semgrep report format mismatch")
    if scan["ingest"] is not None:
        _require(scan["ingest"]["run_id"] == scan["scan_id"], "ingest identity mismatch")
        _require(status == "COMPLETE", "incomplete scan cannot claim ingest")
    _require(status == "COMPLETE" or not bundle["candidates"],
             "incomplete scans cannot supply source-bound candidates")

    seen_coverage: dict[str, dict[str, Any]] = {}
    unique_targets: set[tuple[str, str | None, str, str]] = set()
    for coverage in bundle["coverages"]:
        validate_schema("coverage", coverage)
        _require(coverage["scan_id"] == scan["scan_id"], "foreign scan reference mismatch")
        _require(coverage["coverage_id"] not in seen_coverage, "duplicate coverage ID")
        target = coverage["target"]
        key = (target["language"], target["framework"],
               target["weakness_family"], target["source_scope"])
        _require(key not in unique_targets, "contradictory coverage for same target")
        unique_targets.add(key)
        seen_coverage[coverage["coverage_id"]] = coverage

        claim = coverage["claim"]
        language_scanned = target["language"] in scan["scope"]["languages"]
        within_includes = any(
            _contains(include, target["source_scope"])
            for include in scan["scope"]["include_paths"]
        )
        intersects_includes = any(
            _overlaps(include, target["source_scope"])
            for include in scan["scope"]["include_paths"]
        )
        excluded = any(
            _overlaps(exclude, target["source_scope"])
            for exclude in scan["scope"]["excluded_paths"]
        )
        completely_excluded = any(
            _contains(exclude, target["source_scope"])
            for exclude in scan["scope"]["excluded_paths"]
        )
        if claim.startswith("SCANNED_"):
            _require(status == "COMPLETE", "coverage asserted for incomplete scan")
            _require(language_scanned, "unscanned language cannot be covered")
            _require(within_includes and not excluded,
                     "coverage scope not fully included or intersects exclusions")
        elif claim == "NOT_SCANNED":
            _require(status == "NOT_RUN" or not language_scanned or
                     not intersects_includes or completely_excluded,
                     "NOT_SCANNED contradicts an executed in-scope scan")
        elif claim == "INCOMPLETE":
            _require(status in ("PARTIAL", "FAILED") or
                     (status == "COMPLETE" and language_scanned
                      and intersects_includes and (excluded or not within_includes)),
                     "INCOMPLETE requires failed/partial execution or partial path scope")

    seen_candidate: set[str] = set()
    for candidate in bundle["candidates"]:
        validate_schema("candidate", candidate)
        _require(candidate["scan_id"] == scan["scan_id"],
                 "candidate scan reference mismatch")
        _require(candidate["candidate_id"] not in seen_candidate,
                 "duplicate source-bound candidate")
        seen_candidate.add(candidate["candidate_id"])
        coverage = seen_coverage.get(candidate["coverage_id"])
        _require(coverage is not None, "candidate lacks coverage record")
        _require(coverage["claim"].startswith("SCANNED_"),
                 "candidate cannot originate from unscanned or incomplete scope")
        _require(scan["ingest"] is not None and scan["ingest"]["independently_replayed"] is True,
                 "candidate lacks independently replayed Git-bound ingest receipt")
        path = candidate["source"]["path"]
        _require(candidate["source"]["end_line"] >= candidate["source"]["start_line"]
                 if "end_line" in candidate["source"] else True,
                 "candidate has decreasing line range")
        _require(_contains(coverage["target"]["source_scope"], path),
                 "candidate lies outside declared coverage scope")
        _require(any(_contains(include, path)
                     for include in scan["scope"]["include_paths"]) and
                 not any(_contains(exclusion, path)
                         for exclusion in scan["scope"]["excluded_paths"]),
                 "candidate lies outside executed source scope")
        if coverage["claim"] == "SCANNED_APPLICABLE":
            _require(candidate["rule_id"] in coverage["rule_ids"],
                     "candidate rule not recorded as applicable")
    return {"scans": 1, "coverages": len(seen_coverage),
            "candidates": len(seen_candidate)}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path,
                        help="JSON with scan, coverages and candidates")
    args = parser.parse_args()
    print(json.dumps(validate_bundle(_read(args.bundle)), sort_keys=True))


if __name__ == "__main__":
    main()
