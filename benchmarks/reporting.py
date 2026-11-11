#!/usr/bin/env python3
"""Build and validate versioned, source-bound benchmark receipts.

This module preserves original benchmark output. It does not infer missing
measurements or turn source-set agreement into an independent semantic oracle.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

SCHEMA_NAME = "excession.benchmark-report.v1"
SCHEMA_PATH = Path(__file__).resolve().parent / "schemas/benchmark-report.v1.schema.json"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> dict:
    def reject_constant(value: str) -> None:
        raise ValueError(f"non-finite JSON number: {value}")
    value = json.loads(path.read_text(encoding="utf-8"), parse_constant=reject_constant)
    if not isinstance(value, dict):
        raise ValueError(f"expected JSON object: {path}")
    return value


def validator():
    try:
        from jsonschema import Draft202012Validator
    except ImportError as exc:
        raise RuntimeError("install requirements/research.txt for JSON Schema validation") from exc
    schema = read_json(SCHEMA_PATH)
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def artifact(base: Path, relative_path: str, role: str, media_type: str) -> dict:
    if not relative_path or Path(relative_path).is_absolute() or ".." in Path(relative_path).parts:
        raise ValueError(f"invalid artifact path: {relative_path!r}")
    source = (base / relative_path).resolve(strict=True)
    if not source.is_relative_to(base.resolve()) or not source.is_file():
        raise ValueError(f"artifact escapes report directory or is not a file: {relative_path}")
    return {
        "path": relative_path, "role": role, "media_type": media_type,
        "sha256": sha256(source), "bytes": source.stat().st_size,
    }


def run_id(report: dict) -> str:
    """Deterministic digest of the entire normalized report except its own ID.

    This is tamper evidence for the receipt, not a cryptographic signature.
    Artifact hashes independently bind the raw inputs.
    """
    payload = {key: value for key, value in report.items() if key != "run_id"}
    return hashlib.sha256(
        json.dumps(payload, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    ).hexdigest()


def validate(report_path: Path) -> dict:
    report_path = report_path.resolve(strict=True)
    report = read_json(report_path)
    problems = sorted(validator().iter_errors(report), key=lambda e: tuple(map(str, e.path)))
    if problems:
        first = problems[0]
        pointer = "/" + "/".join(str(part) for part in first.path)
        raise ValueError(f"invalid benchmark report at {pointer}: {first.message}")
    if report["run_id"] != run_id(report):
        raise ValueError("report run_id differs from normalized content identity")
    if len({a["path"] for a in report["artifacts"]}) != len(report["artifacts"]):
        raise ValueError("duplicate artifact path")
    if len({(m["arm"], m["name"], m["statistic"]) for m in report["measurements"]}) != len(report["measurements"]):
        raise ValueError("duplicate measurement identity")
    if len({(s["name"], s["commit"], s["tree"]) for s in report["source_revisions"]}) != len(report["source_revisions"]):
        raise ValueError("duplicate source revision")
    for a in report["artifacts"]:
        expected = artifact(report_path.parent, a["path"], a["role"], a["media_type"])
        if a != expected:
            raise ValueError(f"artifact digest or size mismatch: {a['path']}")
    if report["status"] == "passed" and any(
        c["status"] != "passed" for c in report["checks"]
    ):
        raise ValueError("passed report contains failed or unsupported check")
    return report


def measured(name: str, arm: str, unit: str, statistic: str, value: float | int,
             count: int) -> dict:
    if isinstance(value, bool) or count < 1:
        raise ValueError("measured metric must have a numeric value and positive sample count")
    return {
        "name": name, "arm": arm, "unit": unit, "statistic": statistic,
        "sample_count": count, "status": "measured", "value": value,
    }


def environment() -> dict:
    facts = {
        "system": platform.system(), "machine": platform.machine(),
        "python": platform.python_version(),
    }
    runner = os.environ.get("RUNNER_NAME")
    if runner:
        facts["runner"] = runner
    return facts


def write_report(out: Path, report: dict, filename: str = "report.json") -> Path:
    if Path(filename).name != filename or not filename.endswith(".json"):
        raise ValueError("report filename must be a plain JSON filename")
    out.mkdir(parents=True, exist_ok=True)
    report["run_id"] = run_id(report)
    destination = out / filename
    # Check schema and every linked artifact before publishing a report.
    data = json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n"
    temporary = out / ("." + filename + ".tmp")
    temporary.write_text(data, encoding="utf-8")
    try:
        validate(temporary)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return destination


def build_relational(out: Path) -> Path:
    summary = read_json(out / "summary.json")
    if summary.get("schema") != "excession.relational-retrieval.v1":
        raise ValueError("expected relational retrieval v1 summary")
    trials = [
        json.loads(line, parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))
        for line in (out / "trials.jsonl").read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if len(trials) != summary["query_count_per_arm"] * 2:
        raise ValueError("trial count disagrees with raw summary")
    sources, checks, artifacts = [], [], [
        artifact(out, "summary.json", "summary", "application/json"),
        artifact(out, "trials.jsonl", "raw-samples", "application/x-ndjson"),
    ]
    for item in summary["workloads"]:
        bundle = item["source_bundle"]
        actual_bundle = artifact(out, bundle, "source-bundle", "application/octet-stream")
        if actual_bundle["sha256"] != item["bundle_sha256"]:
            raise ValueError(f"bundle digest does not match summary: {bundle}")
        artifacts.append(actual_bundle)
        for i, rev in enumerate(item["revisions"]):
            label = f"{item['name']}-r{i}"
            sources.append({
                "name": label, "commit": rev["commit"], "tree": rev["tree"],
                "bundle_sha256": item["bundle_sha256"],
            })
            checks.append({
                "name": "indexed_edges_match_source_spec",
                "source": label,
                "description": "Indexed edges equal the independently authored fixture graph",
                "status": "passed" if rev["independently_specified_edges_equal"] else "failed",
            })
    # The timings are reported even if a trial failed; failed results cannot be
    # mistaken for successes or silently discarded from the denominator.
    for arm in ("graph", "view"):
        found = [t for t in trials if t["arm"] == arm]
        if len(found) != summary["query_count_per_arm"]:
            raise ValueError(f"missing trials for {arm}")
        for state, declared in summary["status_counts"][arm].items():
            if sum(t["status"] == state for t in found) != declared:
                raise ValueError(f"summary status counts disagree for {arm}/{state}")
    pairs: dict[tuple, dict[str, dict]] = {}
    for trial in trials:
        key = tuple(trial[k] for k in ("workload", "revision", "commit", "seed", "kind", "repetition"))
        arms = pairs.setdefault(key, {})
        if trial["arm"] in arms:
            raise ValueError(f"duplicate arm for paired query: {key}")
        arms[trial["arm"]] = trial
    paired = all(set(arms) == {"graph", "view"} and
                 all(a["status"] == "ok" and a["exact_oracle_match"] for a in arms.values()) and
                 arms["graph"]["source_paths"] == arms["view"]["source_paths"]
                 for arms in pairs.values())
    checks.append({
        "name": "source_written_oracle_and_pair_parity",
        "description": "Both arms match the source-written oracle and each other for every paired query",
        "sample_count": len(pairs),
        "status": "passed" if paired and len(pairs) == summary["query_count_per_arm"] else "failed",
    })
    metrics = []
    for arm in ("graph", "view"):
        count = summary["query_count_per_arm"]
        metrics.extend([
            measured("query_wall_time", arm, "ms", "median", summary["median_wall_ms"][arm], count),
            measured("query_wall_time", arm, "ms", "sum", summary["total_query_wall_ms"][arm], count),
            measured("stdout_size", arm, "bytes", "median", summary["median_raw_stdout_bytes"][arm], count),
            measured("query_count", arm, "count", "count", count, count),
        ])
    index_count = sum(len(item["revisions"]) for item in summary["workloads"])
    metrics.extend([
        measured("index_wall_time", "index", "ms", "sum",
                 round(sum(r["index_ms"] for w in summary["workloads"] for r in w["revisions"]), 3), index_count),
        measured("index_cpu_time", "index", "ms", "sum",
                 round(sum(r["index_cpu_ms"] for w in summary["workloads"] for r in w["revisions"]), 3), index_count),
    ])
    report = {
        "schema": SCHEMA_NAME, "suite": "relational-retrieval",
        "status": "passed" if all(c["status"] == "passed" for c in checks) else "failed",
        "subject": {"name": "excession", "binary_sha256": summary["binary_sha256"]},
        "oracle": {"kind": "source-written", "independent": True,
                   "description": "Expected import edges and paths are defined by the fixture generator before indexing"},
        "source_revisions": sources, "environment": environment(),
        "configuration": {
            "sizes": summary["sizes"], "fanouts": summary["fanouts"],
            "revisions": summary["revision_count"], "repetitions": summary["repetitions"],
        },
        "measurements": metrics, "checks": checks, "artifacts": artifacts,
        "limitations": [summary["limitations"]],
    }
    return write_report(out, report)


def build_native(out: Path, raw_name: str) -> Path:
    receipt = read_json(out / raw_name)
    if receipt.get("schema") != "excession.native-paired-calibration.v1":
        raise ValueError("expected native paired calibration v1")
    paired = receipt["paired_queries"]
    parity = receipt["source_set_matches"] == paired
    metrics = []
    for arm in ("graph", "paths"):
        count = sum(s["arm"] == arm for s in receipt["samples"])
        if count != paired:
            raise ValueError(f"native calibration samples missing for {arm}")
        for name, unit, values in (
            ("query_wall_time", "ms", receipt["summary"][arm]["wall_ms"]),
            ("child_cpu_time", "ms", receipt["summary"][arm]["child_cpu_ms"]),
            ("stdout_size", "bytes", receipt["summary"][arm]["stdout_bytes"]),
        ):
            metrics.extend([
                measured(name, arm, unit, "median", values["median"], count),
                measured(name, arm, unit, "p95", values["nearest_rank_p95"], count),
            ])
    report = {
        "schema": SCHEMA_NAME, "suite": "native-paired-calibration",
        "status": "passed" if parity else "failed",
        "subject": {"name": "excession", "binary_sha256": receipt["binary_sha256"]},
        "oracle": {
            "kind": "shared-index", "independent": False,
            "description": "Expected path sets derive from the same indexed source and authenticated full view, not an independent semantic oracle",
        },
        "source_revisions": [{
            "name": "h11", "commit": receipt["source_commit"],
            "tree": receipt["source_tree"], "bundle_sha256": receipt["bundle_sha256"],
        }],
        "environment": environment(),
        "configuration": {"paired_queries": paired, "repetitions": receipt["repetitions"]},
        "measurements": metrics,
        "checks": [{
            "name": "source_set_parity",
            "description": "Both native arms returned source sets matching the authenticated full view",
            "sample_count": paired,
            "status": "passed" if parity else "failed",
        }],
        "artifacts": [artifact(out, raw_name, "raw-samples", "application/json")],
        "limitations": [receipt["limitations"]],
    }
    return write_report(out, report, Path(raw_name).stem + ".report.json")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("build-relational", "validate"):
        sub = commands.add_parser(name)
        sub.add_argument("path", type=Path)
    native = commands.add_parser("build-native")
    native.add_argument("directory", type=Path)
    native.add_argument("input", help="JSON filename within report directory")
    args = parser.parse_args()
    try:
        if args.command == "build-relational":
            path = build_relational(args.path)
        elif args.command == "build-native":
            path = build_native(args.directory, args.input)
        else:
            validate(args.path)
            path = args.path
    except (OSError, ValueError, RuntimeError, KeyError, TypeError) as exc:
        parser.exit(1, f"benchmark report refused: {exc}\n")
    print(f"VALID {path}")


if __name__ == "__main__":
    main()
