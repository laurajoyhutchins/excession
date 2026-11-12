#!/usr/bin/env python3
"""Project audited Excession scanner ingestion into security discovery v1.

This is a read-only projection over the Rust security ingest SQLite tables,
the original report bytes, and independently replayed ingest/audit receipts.
It does not perform Git authentication, scanner analysis or vulnerability
assessment. Framework applicability is explicit input, never inferred from
the mere presence of an alert.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
from typing import Any

from validate_security_discovery_contract import validate_bundle


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=True, allow_nan=False).encode("ascii")


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _nonfinite(value: str) -> None:
    raise ValueError(f"non-JSON numeric literal: {value}")


def _load(data: bytes) -> dict[str, Any]:
    item = json.loads(data, object_pairs_hook=_pairs, parse_constant=_nonfinite)
    if not isinstance(item, dict):
        raise ValueError("expected JSON object")
    return item


def _read(path: Path) -> dict[str, Any]:
    return _load(path.read_bytes())


def _require(ok: bool, reason: str) -> None:
    if not ok:
        raise ValueError(reason)


def _run_id(commit: str, report_hash: str, engine: str) -> str:
    return _sha(f"relgit-security-v1\0{commit}\0{report_hash}\0{engine}".encode())


def _report_results(report: dict[str, Any], engine: str) -> tuple[list[Any], str]:
    if engine == "semgrep":
        _require(isinstance(report.get("results"), list), "Semgrep missing results")
        _require(not report.get("errors"), "Semgrep has scan errors")
        return report["results"], "/results/"
    _require(report.get("version") == "2.1.0", "CodeQL must be SARIF 2.1.0")
    runs = report.get("runs")
    _require(isinstance(runs, list) and len(runs) == 1, "CodeQL expected one SARIF run")
    run = runs[0]
    _require(run["tool"]["driver"]["name"].lower() == "codeql",
             "SARIF tool is not CodeQL")
    _require(isinstance(run.get("results"), list), "CodeQL missing results")
    _require(all(inv.get("executionSuccessful", True) is True
                 for inv in run.get("invocations", [])),
             "CodeQL has unsuccessful invocation")
    return run["results"], "/runs/0/results/"


def _rows(db_path: Path, run_id: str) -> tuple[tuple[Any, ...], list[sqlite3.Row], int]:
    # Connection is *always* read-only. The Rust code owns all SQL writes.
    uri = f"file:{db_path.resolve().as_posix()}?mode=ro"
    with sqlite3.connect(uri, uri=True) as db:
        db.row_factory = sqlite3.Row
        _require(db.execute("PRAGMA quick_check").fetchone()[0] == "ok",
                 "SQLite database failed quick_check")
        _require(not db.execute("PRAGMA foreign_key_check").fetchall(),
                 "foreign key violation in ingest database")
        runs = db.execute(
            "SELECT run_id,source_commit,report_sha256,engine,"
            "candidate_count,flow_step_count,status "
            "FROM security_ingest_run"
        ).fetchall()
        _require(len(runs) == 1 and runs[0]["run_id"] == run_id,
                 "expected exactly one matching audited scan run")
        rows = db.execute(
            "SELECT run_id,candidate_id,logical_key,source_path,source_blob,"
            "line,rule_id,raw_evidence FROM security_ingest_candidate "
            "ORDER BY candidate_id"
        ).fetchall()
        steps = db.execute("SELECT COUNT(*) FROM security_ingest_flow_step").fetchone()[0]
        return tuple(runs[0]), rows, steps


def _receipts(ingest: dict[str, Any], audit: dict[str, Any],
              run: tuple[Any, ...], report_hash: str, engine: str) -> None:
    run_id, commit, stored_hash, stored_engine, n, steps, status = run
    _require(status == "UNVERIFIED_STATIC_CANDIDATES", "unexpected ingest status")
    _require(stored_hash == report_hash and stored_engine == engine,
             "SQLite report digest or engine mismatch")
    expected = {
        "schema": "relgit.security.ingest.receipt.v1",
        "run_id": run_id,
        "source_commit": commit,
        "report_sha256": report_hash,
        "engine": engine,
        "candidate_count": n,
        "flow_step_count": steps,
        "status": status,
    }
    _require(all(ingest.get(k) == v and audit.get(k) == v
                 for k, v in expected.items()), "ingest/audit receipt mismatch")
    _require(ingest.get("independently_replayed") is False and
             audit.get("independently_replayed") is True,
             "expected original ingest and independently replayed audit receipts")


def _within(scope: str, path: str) -> bool:
    return scope == "." or path == scope or path.startswith(scope + "/")


def project(
    config: dict[str, Any], report_bytes: bytes, db_path: Path,
    ingest_receipt: dict[str, Any], audit_receipt: dict[str, Any],
    ruleset_bytes: bytes,
) -> dict[str, Any]:
    _require(config.get("schema") == "excession.security.projection-input.v1",
             "unsupported projection configuration")
    _require(set(config) == {
        "schema", "repository", "scanner", "ruleset", "scope",
        "execution_evidence_refs", "coverages",
    }, "projection configuration has missing or unknown fields")
    engine = config["scanner"]["engine"]
    _require(engine in {"semgrep", "codeql"}, "unsupported scanner engine")
    _require(config["ruleset"]["configuration_sha256"] == _sha(ruleset_bytes),
             "ruleset configuration file digest mismatch")
    _require(config["execution_evidence_refs"],
             "completed scan requires execution evidence references")
    report_hash = _sha(report_bytes)
    _require(len(report_bytes) > 0 and len(report_bytes) <= 16 * 1024 * 1024,
             "report size outside Rust ingest limits")
    report = _load(report_bytes)
    original_results, prefix = _report_results(report, engine)
    _require(len(config["scope"]["languages"]) == 1,
             "projection requires one explicitly selected language")
    commit = audit_receipt["source_commit"]
    run_id = _run_id(commit, report_hash, engine)
    run, rows, steps = _rows(db_path, run_id)
    _receipts(ingest_receipt, audit_receipt, run, report_hash, engine)
    _require(run[1] == commit, "source commit mismatch")
    _require(run[4] == len(rows) == len(original_results) and run[5] == steps,
             "reported candidate or flow counts disagree with audited SQLite")
    _require(len(rows) <= 10_000, "too many candidates")

    # Match each source-bound Rust candidate to exactly one original result,
    # not a reinterpreted or normalized result. Preserve raw report bytes.
    indices: dict[bytes, list[int]] = {}
    for idx, result in enumerate(original_results):
        indices.setdefault(_canonical(result), []).append(idx)
    located: list[tuple[sqlite3.Row, int]] = []
    for row in rows:
        source_result = _load(row["raw_evidence"].encode("utf-8"))
        matches = indices.get(_canonical(source_result), [])
        _require(len(matches) == 1, "ambiguous or missing original scanner result")
        index = matches.pop()
        _require(row["run_id"] == run_id, "candidate foreign run mismatch")
        _require(row["candidate_id"] == _sha(
            f"{row['logical_key']}\0{row['source_blob']}\0{row['raw_evidence']}".encode()),
            "Rust candidate ID does not match canonical stored evidence")
        _require(row["logical_key"] == _sha(
            f"{engine}\0{row['rule_id']}\0{row['source_path']}\0{row['line']}".encode()),
            "Rust logical candidate key mismatch")
        located.append((row, index))
    _require(all(not values for values in indices.values()),
             "unmatched original scanner result")

    scanner = config["scanner"]
    scan = {
        "schema": "excession.security.scan.v1",
        "scan_id": run_id,
        "source": {"repository": config["repository"], "commit": commit},
        "scanner": scanner,
        "ruleset": config["ruleset"],
        "scope": config["scope"],
        "execution": {"status": "COMPLETE", "exit_code": 0, "errors": [],
                      "evidence_refs": config["execution_evidence_refs"]},
        "report": {"format": "SEMGREP_JSON" if engine == "semgrep" else "SARIF_2_1_0",
                   "sha256": report_hash, "bytes": len(report_bytes)},
        "ingest": {"run_id": run_id, "independently_replayed": True},
    }

    coverages: list[dict[str, Any]] = []
    by_target: dict[tuple[str, str | None, str, str], dict[str, Any]] = {}

    def add_coverage(spec: dict[str, Any]) -> dict[str, Any]:
        target = spec["target"]
        key = (target["language"], target["framework"],
               target["weakness_family"], target["source_scope"])
        _require(key not in by_target, "duplicate coverage target")
        item = {
            "schema": "excession.security.coverage.v1",
            "coverage_id": _sha(b"excession-security-coverage-v1\0" +
                                run_id.encode() + b"\0" + _canonical(target)),
            "scan_id": run_id,
            "target": target,
            "claim": spec["claim"],
            "rule_ids": sorted(set(spec["rule_ids"])),
            "evidence_refs": sorted(set(spec["evidence_refs"])),
        }
        coverages.append(item)
        by_target[key] = item
        return item

    for item in config["coverages"]:
        add_coverage(item)

    candidates: list[dict[str, Any]] = []
    for row, index in located:
        matches = [
            coverage for coverage in coverages
            if row["rule_id"] in coverage["rule_ids"]
            and _within(coverage["target"]["source_scope"], row["source_path"])
        ]
        _require(len(matches) <= 1,
                 "ambiguous coverage mapping for scanner rule and source path")
        if not matches:
            # An alert without declared family applicability is never dropped,
            # or silently promoted to a known CWE/framework classification.
            key = (config["scope"]["languages"][0], None,
                   "UNMAPPED_SCANNER_RULE", row["source_path"])
            unknown = by_target.get(key)
            if unknown is None:
                unknown = add_coverage({
                    "target": {"language": key[0], "framework": None,
                               "weakness_family": key[2], "source_scope": key[3]},
                    "claim": "SCANNED_APPLICABILITY_UNPROVEN",
                    "rule_ids": [],
                    "evidence_refs": [f"audited-run:{run_id}:unmapped-rule"],
                })
            unknown["rule_ids"] = sorted(set(unknown["rule_ids"]) | {row["rule_id"]})
            matches = [unknown]
        candidate = {
            "schema": "excession.security.candidate.v1",
            "candidate_id": row["candidate_id"],
            "logical_key": row["logical_key"],
            "scan_id": run_id,
            "coverage_id": matches[0]["coverage_id"],
            "source": {"path": row["source_path"], "blob": row["source_blob"],
                       "start_line": row["line"]},
            "rule_id": row["rule_id"],
            "raw_result": {"json_pointer": f"{prefix}{index}",
                           "sha256": _sha(row["raw_evidence"].encode())},
            "claim": "UNVERIFIED_STATIC_CANDIDATE",
        }
        candidates.append(candidate)

    bundle = {"scan": scan, "coverages": sorted(coverages, key=lambda x: x["coverage_id"]),
              "candidates": sorted(candidates, key=lambda x: x["candidate_id"])}
    validate_bundle(bundle)
    return bundle


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("config", "ruleset-configuration", "report", "database",
                 "ingest-receipt", "audit-receipt", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    bundle = project(
        _read(args.config), args.report.read_bytes(), args.database,
        _read(args.ingest_receipt), _read(args.audit_receipt),
        args.ruleset_configuration.read_bytes(),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2, sort_keys=True) + "\n",
                           encoding="utf-8")
    print(json.dumps({"scan_id": bundle["scan"]["scan_id"],
                      "coverages": len(bundle["coverages"]),
                      "candidates": len(bundle["candidates"])}))


if __name__ == "__main__":
    main()
