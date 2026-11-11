"""Independent assessor contracts: native behavior, tampering, and honest limits."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("assessor", HERE / "tools/assess_security_findings.py")
assert spec and spec.loader
assessor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assessor)


def git(repo: Path, *args: str) -> str:
    p = subprocess.run(["git", "-C", str(repo), *args], check=True, text=True, capture_output=True)
    return p.stdout.strip()


class ValidityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-validity-")
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name) / "repo"
        corpus = HERE / "benchmarks/security-validity"
        target = self.repo / "benchmarks/security-validity"
        target.mkdir(parents=True)
        for file in corpus.iterdir():
            if file.is_file():
                shutil.copy2(file, target / file.name)
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.name", "Validity Test")
        git(self.repo, "config", "user.email", "validity@example.invalid")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "fixture")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        self.blob = git(self.repo, "hash-object", "benchmarks/security-validity/path.go")
        self.run_id = "a" * 64
        self.db = Path(self.tmp.name) / "ingest.sqlite"
        self.truth = target / "ground-truth.json"
        with sqlite3.connect(self.db) as connection:
            connection.executescript("""
                CREATE TABLE security_ingest_run(run_id TEXT,source_commit TEXT,report_sha256 TEXT,
                  engine TEXT,candidate_count INTEGER,status TEXT);
                CREATE TABLE security_ingest_candidate(run_id TEXT,candidate_id TEXT,
                  source_path TEXT,source_blob TEXT,line INTEGER,rule_id TEXT);
            """)
        self.set_candidates([14, 30, 25, 2])

    def set_candidates(self, lines: list[int]) -> None:
        with sqlite3.connect(self.db) as connection:
            connection.execute("DELETE FROM security_ingest_run")
            connection.execute("DELETE FROM security_ingest_candidate")
            connection.execute("INSERT INTO security_ingest_run VALUES(?,?,?,?,?,?)",
                               (self.run_id, self.commit, "f" * 64, "semgrep", len(lines),
                                "UNVERIFIED_STATIC_CANDIDATES"))
            for i, line in enumerate(lines):
                connection.execute("INSERT INTO security_ingest_candidate VALUES(?,?,?,?,?,?)",
                                   (self.run_id, f"candidate-{i}", "benchmarks/security-validity/path.go",
                                    self.blob, line, "benchmarks.security-validity.excession.go.joined-read"))

    def audit(self) -> dict:
        return {"run_id": self.run_id, "source_commit": self.commit,
                "report_sha256": "f" * 64, "engine": "semgrep", "independently_replayed": True}

    def assess(self) -> dict:
        return assessor.assess(self.repo, self.db, self.run_id, self.truth,
                               self.audit(), trusted_local=True)

    def test_native_oracle_distinguishes_positive_negative_and_unknown(self) -> None:
        result = self.assess()
        self.assertEqual(result["summary"], {
            "reproduced_boundaries": 1, "safe_control_hits": 2,
            "unjudged_candidates": 1, "known_positive_cases_without_hit": 0})
        self.assertEqual([c["status"] for c in result["candidate_outcomes"]],
                         ["REPRODUCED_LOCAL_BOUNDARY", "SAFE_CONTROL_HIT",
                          "SAFE_CONTROL_HIT", "UNJUDGED_CANDIDATE"])
        self.assertEqual(len(result["native_oracle"]["tests"]), 4)
        self.assertIn("not vulnerability reachability", result["claim_boundary"])

    def test_unrelated_rule_at_positive_location_is_unjudged(self) -> None:
        with sqlite3.connect(self.db) as connection:
            connection.execute("UPDATE security_ingest_candidate SET rule_id='unrelated.rule' "
                               "WHERE line=14")
        result = self.assess()
        self.assertEqual(result["summary"]["reproduced_boundaries"], 0)
        self.assertEqual(result["summary"]["known_positive_cases_without_hit"], 1)
        self.assertEqual(result["candidate_outcomes"][0]["status"], "UNJUDGED_CANDIDATE")

    def test_missing_scanner_hit_is_not_called_clean(self) -> None:
        self.set_candidates([])
        result = self.assess()
        self.assertEqual(result["summary"]["known_positive_cases_without_hit"], 1)
        self.assertNotIn("recall", result["summary"])

    def test_source_drift_rejected(self) -> None:
        with self.truth.open(encoding="utf-8") as f:
            truth = json.load(f)
        truth["cases"][0]["source_blob"] = "0" * 40
        # The independent truth is changed and committed; it no longer binds to source.
        self.truth.write_text(json.dumps(truth) + "\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "tamper truth")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        self.set_candidates([14])
        with self.assertRaisesRegex(ValueError, "source blob changed"):
            self.assess()

    def test_uncommitted_change_rejected(self) -> None:
        with (self.repo / "benchmarks/security-validity/path.go").open("a") as out:
            out.write("\n// modified\n")
        with self.assertRaisesRegex(ValueError, "clean"):
            self.assess()

    def test_mismatched_audit_receipt_rejected(self) -> None:
        audit = self.audit()
        audit["independently_replayed"] = False
        with self.assertRaisesRegex(ValueError, "fresh Excession scanner audit"):
            assessor.assess(self.repo, self.db, self.run_id, self.truth,
                            audit, trusted_local=True)

    def test_missing_positive_control_rejected(self) -> None:
        truth = json.loads(self.truth.read_text())
        truth["cases"] = [c for c in truth["cases"] if c["expected"] == "safe-control"]
        self.truth.write_text(json.dumps(truth) + "\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "remove control")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        self.set_candidates([30])
        with self.assertRaisesRegex(ValueError, "positive and negative"):
            self.assess()

    def test_no_untrusted_execution(self) -> None:
        with self.assertRaisesRegex(ValueError, "explicit --trusted-local"):
            assessor.assess(self.repo, self.db, self.run_id, self.truth,
                            self.audit(), trusted_local=False)


if __name__ == "__main__":
    unittest.main()
