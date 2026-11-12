#!/usr/bin/env python3
"""Compare audited scanner candidates with independent, Git-pinned native controls.

This is a *local benchmark assessment*, not an exploit or CVE verdict. Native
oracle execution is opt-in and must only be used on a trusted source checkout.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
from urllib.parse import quote


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=True,
        timeout=10,
    )
    return result.stdout.strip()


def checked_path(repo: Path, relative: str) -> tuple[str, int]:
    require(relative != "" and not relative.startswith("/") and "\\" not in relative,
            "source path must be checkout-relative")
    require(all(part not in ("", ".", "..") for part in relative.split("/")),
            "source path has unsafe components")
    entry = git(repo, "ls-tree", "HEAD", "--", relative).splitlines()
    require(len(entry) == 1 and entry[0].split("\t")[-1] == relative,
            f"source not tracked: {relative}")
    meta = entry[0].split("\t")[0].split()
    require(len(meta) == 3 and meta[0] in ("100644", "100755") and meta[1] == "blob",
            "source must be a regular Git file")
    source = (repo / relative).resolve()
    require(source.is_relative_to(repo.resolve()) and source.is_file(), "source escapes checkout")
    require(git(repo, "hash-object", relative) == meta[2], "checkout does not match source blob")
    return meta[2], len(source.read_bytes().splitlines())


def load_ground_truth(repo: Path, path: Path) -> tuple[dict, dict[str, str]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    require(data.get("schema") == "excession.security.ground-truth.v1", "unsupported truth schema")
    require(data.get("oracle_kind") == "go-test", "only trusted Go test oracles are supported")
    require(isinstance(data.get("family"), str) and bool(data["family"]), "missing weakness family")
    oracle_dir = data.get("oracle_directory")
    require(isinstance(oracle_dir, str), "missing oracle directory")
    checked_path(repo, f"{oracle_dir}/go.mod")
    cases = data.get("cases")
    require(isinstance(cases, list) and len(cases) >= 2, "need positive and safe controls")
    allowed_rules = data.get("rule_ids")
    require(isinstance(allowed_rules, dict) and
            all(engine in ("semgrep", "codeql") and isinstance(rules, list)
                and bool(rules) and all(isinstance(rule, str) and bool(rule) for rule in rules)
                and len(set(rules)) == len(rules)
                for engine, rules in allowed_rules.items()),
            "ground truth must bind explicit scanner rule IDs")
    tests: dict[str, str] = {}
    ids: set[str] = set()
    ranges: dict[str, list[tuple[int, int]]] = {}
    expectations: set[str] = set()
    for case in cases:
        require(isinstance(case, dict), "case must be an object")
        ident, path_name, expected, test_name = (case.get(k) for k in
                                                  ("id", "path", "expected", "oracle_test"))
        require(all(isinstance(x, str) and bool(x) for x in
                    (ident, path_name, expected, test_name)), "missing truth case fields")
        require(ident not in ids, "duplicate ground-truth case")
        ids.add(ident)
        require(expected in ("vulnerable-behavior", "safe-control"), "unknown ground-truth class")
        expectations.add(expected)
        blob, max_line = checked_path(repo, path_name)
        require(case.get("source_blob") == blob, "ground truth source blob changed")
        start, end = case.get("start_line"), case.get("end_line")
        require(type(start) is int and type(end) is int and 1 <= start <= end <= max_line,
                "ground-truth span is not in the exact source")
        require(all(end < lo or start > hi for lo, hi in ranges.setdefault(path_name, [])),
                "ground-truth spans must not overlap")
        ranges[path_name].append((start, end))
        require(test_name.startswith("Test") and test_name.replace("_", "").isalnum(),
                "invalid oracle test name")
        tests[test_name] = expected
    require(expectations == {"vulnerable-behavior", "safe-control"},
            "positive and negative controls both required")
    extra = data.get("additional_required_tests", [])
    require(isinstance(extra, list) and all(isinstance(t, str) and t.startswith("Test")
                                             and t.replace("_", "").isalnum() for t in extra),
            "invalid additional oracle tests")
    tests.update({t: "additional-control" for t in extra})
    return data, tests


def run_oracle(repo: Path, truth: dict, required_tests: dict[str, str]) -> dict:
    # Explicitly trusted local code only; Go's test runner is not a sandbox.
    directory = (repo / truth["oracle_directory"]).resolve()
    require(directory.is_relative_to(repo.resolve()), "oracle directory escapes checkout")
    env = {**os.environ, "GOPROXY": "off", "GOTOOLCHAIN": "local",
           "GOSUMDB": "off", "GOWORK": "off", "GOFLAGS": "-mod=readonly"}
    process = subprocess.run(["go", "test", "-count=1", "-json", "./..."],
                             cwd=directory, capture_output=True, timeout=60, env=env)
    require(process.returncode == 0, "native oracle test process failed")
    require(len(process.stdout) <= 2_000_000, "oracle output exceeds size budget")
    final: dict[str, str] = {}
    for raw_line in process.stdout.splitlines():
        event = json.loads(raw_line)
        name = event.get("Test")
        action = event.get("Action")
        if name in required_tests and action in ("pass", "fail", "skip"):
            require(name not in final, "duplicate oracle test outcome")
            final[name] = action
    require(set(final) == set(required_tests), "missing native oracle test observations")
    require(all(outcome == "pass" for outcome in final.values()),
            "native oracle did not pass all controls")
    require(git(repo, "status", "--porcelain", "--untracked-files=all") == "",
            "oracle modified the source checkout")
    runtime = subprocess.run(["go", "version"], capture_output=True, text=True,
                             check=True, timeout=10).stdout.strip()
    return {"output_sha256": hashlib.sha256(process.stdout).hexdigest(),
            "go_version": runtime, "tests": final,
            "kind": "trusted-local-go-test-not-sandboxed"}


def read_ingestion(db_path: Path, run_id: str) -> tuple[dict, list[dict]]:
    require(db_path.is_file(), "evidence database absent")
    uri = f"file:{quote(str(db_path.resolve()))}?mode=ro"
    with sqlite3.connect(uri, uri=True) as db:
        db.row_factory = sqlite3.Row
        row = db.execute("SELECT run_id,source_commit,report_sha256,engine,candidate_count,status "
                         "FROM security_ingest_run WHERE run_id=?", (run_id,)).fetchone()
        require(row is not None, "ingestion run absent")
        run = dict(row)
        require(run["status"] == "UNVERIFIED_STATIC_CANDIDATES", "unexpected scanner verdict")
        candidates = [dict(x) for x in db.execute(
            "SELECT candidate_id,source_path,source_blob,line,rule_id "
            "FROM security_ingest_candidate WHERE run_id=? ORDER BY candidate_id", (run_id,))]
        require(len(candidates) == run["candidate_count"], "ingestion count mismatch")
    return run, candidates


def assess(repo: Path, db_path: Path, run_id: str, truth_path: Path,
           audit_receipt: dict, *, trusted_local: bool) -> dict:
    require(trusted_local, "native code execution requires explicit --trusted-local true")
    require(git(repo, "status", "--porcelain", "--untracked-files=all") == "",
            "source checkout must be clean")
    commit = git(repo, "rev-parse", "HEAD")
    run, candidates = read_ingestion(db_path, run_id)
    require(run["source_commit"] == commit, "scanner source revision differs from oracle")
    require(audit_receipt.get("run_id") == run_id and
            audit_receipt.get("source_commit") == commit and
            audit_receipt.get("report_sha256") == run["report_sha256"] and
            audit_receipt.get("engine") == run["engine"] and
            audit_receipt.get("independently_replayed") is True,
            "fresh Excession scanner audit required")
    truth, tests = load_ground_truth(repo, truth_path)
    truth_blob, _ = checked_path(repo, str(truth_path.resolve().relative_to(repo.resolve())))
    oracle = run_oracle(repo, truth, tests)
    outcomes = []
    summary = {"reproduced_boundaries": 0, "safe_control_hits": 0,
               "unjudged_candidates": 0, "known_positive_cases_without_hit": 0}
    matched_positives: set[str] = set()
    for candidate in candidates:
        source_path, source_blob, line = (candidate[k] for k in
                                          ("source_path", "source_blob", "line"))
        case = next((c for c in truth["cases"]
                     if candidate["rule_id"] in truth["rule_ids"].get(run["engine"], [])
                     and c["path"] == source_path and c["source_blob"] == source_blob
                     and c["start_line"] <= line <= c["end_line"]), None)
        if case is None:
            status = "UNJUDGED_CANDIDATE"
            summary["unjudged_candidates"] += 1
        elif case["expected"] == "vulnerable-behavior":
            status = "REPRODUCED_LOCAL_BOUNDARY"
            matched_positives.add(case["id"])
            summary["reproduced_boundaries"] += 1
        else:
            status = "SAFE_CONTROL_HIT"
            summary["safe_control_hits"] += 1
        outcomes.append({"candidate_id": candidate["candidate_id"], "rule_id": candidate["rule_id"],
                         "path": source_path, "line": line, "status": status,
                         "case_id": case["id"] if case else None})
    summary["known_positive_cases_without_hit"] = sum(
        c["expected"] == "vulnerable-behavior" and c["id"] not in matched_positives
        for c in truth["cases"])
    # No recall/TN estimate: scanner target and rule coverage are not attested.
    return {"schema": "excession.security.validity-assessment.v1", "source_commit": commit,
            "run_id": run_id, "engine": run["engine"], "report_sha256": run["report_sha256"],
            "family": truth["family"], "ground_truth_blob": truth_blob,
            "native_oracle": oracle, "summary": summary,
            "candidate_outcomes": outcomes,
            "claim_boundary": "local behavioral controls only; not vulnerability reachability, impact, scanner recall or CVE confirmation"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--ground-truth", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--excession", type=Path, required=True)
    parser.add_argument("--trusted-local", choices=("true", "false"), default="false")
    args = parser.parse_args()
    try:
        require(args.trusted_local == "true", "trusted-local opt-in required")
        run, _ = read_ingestion(args.db, args.run_id)
        audit = subprocess.run([str(args.excession), "security", "audit", "--repo", str(args.repo),
                                "--report", str(args.report), "--db", str(args.db), "--engine", run["engine"]],
                               capture_output=True, text=True, timeout=30, check=True)
        receipt = json.loads(audit.stdout)
        result = assess(args.repo, args.db, args.run_id, args.ground_truth, receipt,
                        trusted_local=True)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except (ValueError, OSError, subprocess.SubprocessError, sqlite3.Error, json.JSONDecodeError) as exc:
        print(f"REFUSED: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
