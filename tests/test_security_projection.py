"""Adapter acceptance: real SQLite relation shape, both native report formats.

These tests simulate Rust ingest rows precisely; only the Rust executable can
authenticate source Git blobs and independently replay an actual ingest.
"""
from __future__ import annotations

import copy
import hashlib
import json
import sqlite3
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from project_security_discovery import project  # noqa: E402


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value: object) -> str:
    return json.dumps(value, separators=(",", ":"), sort_keys=True, ensure_ascii=True)


def case(engine: str = "semgrep", findings: bool = True):
    root = tempfile.TemporaryDirectory(prefix="security-adapter-")
    db_path = Path(root.name) / "ingest.sqlite"
    ruleset_bytes = b"exact immutable scanner configuration\n"
    source_commit = "a" * 40
    blob = "b" * 40
    rule = "python.path-traversal" if engine == "semgrep" else "py/path-injection"
    if engine == "semgrep":
        original = {"check_id": rule, "path": "src/app.py",
                    "start": {"line": 4, "col": 1},
                    "end": {"line": 5, "col": 1},
                    "extra": {"metadata": {"cwe": ["CWE-22"]}}}
        report = {"results": [original] if findings else [], "errors": []}
    else:
        original = {
            "ruleId": rule,
            "locations": [{"physicalLocation": {
                "artifactLocation": {"uri": "src/app.py"},
                "region": {"startLine": 4}}}],
            "message": {"text": "Path input may escape a root"},
        }
        report = {"version": "2.1.0", "runs": [{
            "tool": {"driver": {"name": "CodeQL"}},
            "results": [original] if findings else [],
            "invocations": [{"executionSuccessful": True}],
        }]}
    raw = (json.dumps(report, separators=(",", ":"), sort_keys=True) + "\n").encode()
    report_hash = sha(raw)
    run_id = sha(f"relgit-security-v1\0{source_commit}\0{report_hash}\0{engine}".encode())
    evidence = canonical(original)
    logical_key = sha(f"{engine}\0{rule}\0src/app.py\04".encode())
    candidate_id = sha(f"{logical_key}\0{blob}\0{evidence}".encode())
    with sqlite3.connect(db_path) as db:
        db.executescript("""
          CREATE TABLE security_ingest_run (
            run_id TEXT, source_commit TEXT, report_sha256 TEXT, engine TEXT,
            candidate_count INTEGER, flow_step_count INTEGER, status TEXT
          );
          CREATE TABLE security_ingest_candidate (
            run_id TEXT, candidate_id TEXT, logical_key TEXT, source_path TEXT,
            source_blob TEXT, line INTEGER, rule_id TEXT, raw_evidence TEXT
          );
          CREATE TABLE security_ingest_flow_step (run_id TEXT);
        """)
        db.execute("INSERT INTO security_ingest_run VALUES(?,?,?,?,?,?,?)",
                   (run_id, source_commit, report_hash, engine, int(findings), 0,
                    "UNVERIFIED_STATIC_CANDIDATES"))
        if findings:
            db.execute("INSERT INTO security_ingest_candidate VALUES(?,?,?,?,?,?,?,?)",
                       (run_id, candidate_id, logical_key, "src/app.py", blob,
                        4, rule, evidence))
    receipt = {
        "schema": "relgit.security.ingest.receipt.v1",
        "run_id": run_id,
        "source_commit": source_commit,
        "report_sha256": report_hash,
        "engine": engine,
        "candidate_count": int(findings),
        "flow_step_count": 0,
        "status": "UNVERIFIED_STATIC_CANDIDATES",
        "independently_replayed": False,
    }
    audit = {**receipt, "independently_replayed": True}
    config = {
        "schema": "excession.security.projection-input.v1",
        "repository": "fixture/repository",
        "scanner": {"engine": engine, "version": "fixture-1.0",
                    "toolchain_ref": "toolchain@immutable-source-commit"},
        "ruleset": {"immutable_ref": "ruleset@immutable-source-commit",
                    "configuration_sha256": sha(ruleset_bytes),
                    "selection": ["path-traversal"]},
        "scope": {"languages": ["python"], "include_paths": ["."],
                  "excluded_paths": []},
        "execution_evidence_refs": ["controlled-fixture:completed-run"],
        "coverages": [{
            "target": {"language": "python", "framework": None,
                       "weakness_family": "CWE-22", "source_scope": "."},
            "claim": "SCANNED_APPLICABLE",
            "rule_ids": [rule],
            "evidence_refs": ["ruleset:declared-applicability"],
        }],
    }
    return root, config, raw, db_path, receipt, audit, ruleset_bytes


class SecurityProjectionAdapterTests(unittest.TestCase):
    def setUp(self):
        self.tmp, self.config, self.report, self.db, self.ingest, self.audit, self.ruleset = case()
        self.addCleanup(self.tmp.cleanup)

    def produce(self):
        return project(self.config, self.report, self.db,
                       self.ingest, self.audit, self.ruleset)

    def reject(self, operation, phrase: str):
        operation()
        with self.assertRaisesRegex((ValueError, Exception), phrase):
            self.produce()

    def test_semgrep_rows_project_to_unchanged_git_bound_ids(self):
        result = self.produce()
        self.assertEqual(len(result["candidates"]), 1)
        self.assertEqual(result["scan"]["scan_id"], self.audit["run_id"])
        self.assertEqual(result["scan"]["report"]["sha256"], sha(self.report))
        self.assertEqual(result["candidates"][0]["claim"], "UNVERIFIED_STATIC_CANDIDATE")
        self.assertEqual(result["candidates"][0]["raw_result"]["json_pointer"],
                         "/results/0")
        self.assertEqual(result["coverages"][0]["claim"], "SCANNED_APPLICABLE")
        self.assertTrue(result["scan"]["ingest"]["independently_replayed"])

    def test_codeql_sarif_projects_same_authoritative_record_types(self):
        root, config, raw, db, ingest, audit, rules = case(engine="codeql")
        self.addCleanup(root.cleanup)
        bundle = project(config, raw, db, ingest, audit, rules)
        self.assertEqual(bundle["candidates"][0]["raw_result"]["json_pointer"],
                         "/runs/0/results/0")
        self.assertEqual(bundle["scan"]["report"]["format"], "SARIF_2_1_0")

    def test_no_findings_is_not_no_coverage(self):
        root, config, raw, db, ingest, audit, rules = case(findings=False)
        self.addCleanup(root.cleanup)
        bundle = project(config, raw, db, ingest, audit, rules)
        self.assertEqual(bundle["candidates"], [])
        self.assertEqual(len(bundle["coverages"]), 1)

    def test_unknown_rule_does_not_disappear_or_become_applicable(self):
        self.config["coverages"] = []
        result = self.produce()
        self.assertEqual(len(result["candidates"]), 1)
        self.assertEqual(result["coverages"][0]["claim"],
                         "SCANNED_APPLICABILITY_UNPROVEN")
        self.assertEqual(result["coverages"][0]["target"]["weakness_family"],
                         "UNMAPPED_SCANNER_RULE")
        self.assertEqual(result["candidates"][0]["coverage_id"],
                         result["coverages"][0]["coverage_id"])

    def test_inapplicable_framework_alert_is_preserved_without_promotion(self):
        self.config["coverages"][0]["claim"] = "SCANNED_NOT_APPLICABLE"
        self.config["coverages"][0]["target"]["framework"] = "other-framework"
        self.assertEqual(self.produce()["coverages"][0]["claim"],
                         "SCANNED_NOT_APPLICABLE")

    def test_wrong_report_and_ruleset_bytes_rejected(self):
        with self.assertRaisesRegex(ValueError, "digest"):
            project(self.config, self.report + b" ", self.db,
                    self.ingest, self.audit, self.ruleset)
        with self.assertRaisesRegex(ValueError, "configuration file digest"):
            project(self.config, self.report, self.db,
                    self.ingest, self.audit, b"different rules")

    def test_missing_independent_replay_and_forged_identity_rejected(self):
        self.audit["independently_replayed"] = False
        with self.assertRaisesRegex(ValueError, "replayed"):
            self.produce()
        self.audit["independently_replayed"] = True
        self.audit["source_commit"] = "f" * 40
        with self.assertRaisesRegex(ValueError, "matching audited scan run"):
            self.produce()

    def test_sqlite_candidate_row_tampering_rejected(self):
        with sqlite3.connect(self.db) as db:
            db.execute("UPDATE security_ingest_candidate SET source_blob=?",
                       ("e" * 40,))
        with self.assertRaisesRegex(ValueError, "candidate ID"):
            self.produce()

    def test_raw_result_tampering_rejected(self):
        with sqlite3.connect(self.db) as db:
            db.execute("UPDATE security_ingest_candidate SET raw_evidence=?",
                       ('{"check_id":"different"}',))
        with self.assertRaisesRegex(ValueError, "missing original scanner result"):
            self.produce()

    def test_ambiguous_rule_coverage_rejected(self):
        other = copy.deepcopy(self.config["coverages"][0])
        other["target"]["weakness_family"] = "OTHER"
        self.config["coverages"].append(other)
        with self.assertRaisesRegex(ValueError, "ambiguous coverage"):
            self.produce()

    def test_scanner_error_rejected_even_with_ingest_claim(self):
        bad = json.loads(self.report)
        bad["errors"] = [{"message": "fatal parse failure"}]
        with self.assertRaisesRegex(ValueError, "Semgrep has scan errors"):
            project(self.config, json.dumps(bad).encode(), self.db,
                    self.ingest, self.audit, self.ruleset)

    def test_duplicate_json_keys_are_not_silently_accepted(self):
        bad = b'{"results":[],"results":[],"errors":[]}'
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            project(self.config, bad, self.db, self.ingest, self.audit, self.ruleset)

    def test_out_of_scope_claim_fails_closed(self):
        self.config["scope"]["excluded_paths"] = ["src"]
        with self.assertRaises(ValueError):
            self.produce()

    def test_unknown_config_fields_rejected(self):
        self.config["is_safe"] = True
        with self.assertRaisesRegex(ValueError, "missing or unknown"):
            self.produce()

    def test_sqlite_is_read_only(self):
        self.produce()
        with sqlite3.connect(self.db) as db:
            self.assertEqual(db.execute(
                "SELECT COUNT(*) FROM security_ingest_candidate").fetchone()[0], 1)


if __name__ == "__main__":
    unittest.main()
