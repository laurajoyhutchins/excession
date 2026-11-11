"""Deterministic contract and independent-source consistency checks for evaluation.

A synthetic reviewer label is not evidence of true semantic correctness.
"""
from __future__ import annotations

import copy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import evaluate_concept_enrichment as evaluation  # noqa: E402


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], timeout=20).decode().strip()


def seal(value: dict, key: str) -> dict:
    value[key] = evaluation.digest({k: v for k, v in value.items() if k != key})
    return value


class ConceptEvaluationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-concepts-eval-")
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Evaluation fixture")
        git(self.repo, "config", "user.email", "eval@example.invalid")
        (self.repo / "source.py").write_text("def f():\n    return 7\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "Pinned independent source")
        commit = git(self.repo, "rev-parse", "HEAD")
        tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        blob = git(self.repo, "rev-parse", "HEAD:source.py")
        self.candidate = "a" * 64
        self.baseline = seal({
            "schema": "excession.refactor-candidates.v1",
            "status": "UNVERIFIED_REFACTOR_HYPOTHESES",
            "source_commit": commit, "source_tree": tree,
            "candidates": [{"candidate_id": self.candidate}],
        }, "report_id")
        self.model = seal({
            "schema": "excession.concepts-model.v1",
            "source_commit": commit, "source_tree": tree,
            "claims": [{"claim_id": "b" * 64}],
        }, "model_id")
        self.manifest = seal({
            "schema": "excession.concepts-snapshot.v1",
            "source_commit": commit, "source_tree": tree,
            "model_id": self.model["model_id"],
        }, "snapshot_id")
        self.enriched = seal({
            "schema": "excession.concepts-analysis-link.v1",
            "source_commit": commit, "snapshot_id": self.manifest["snapshot_id"],
            "analysis_report_id": self.baseline["report_id"],
            "analysis_kind": "REFACTOR_CANDIDATES", "coverage_status": "PARTIAL",
            "links": [{
                "analysis_candidate_id": self.candidate,
                "concept_claim_ids": ["b" * 64],
                "role": "CONCEPT_ANNOTATION", "assessment": "ANNOTATION_ONLY",
            }],
        }, "link_report_id")
        self.oracle = seal({
            "schema": "excession.concepts-oracle.v1",
            "source_commit": commit, "source_tree": tree,
            "cases": [{
                "candidate_id": self.candidate,
                "expected": "LINK_WARRANTED",
                "source": [{"path": "source.py", "blob": blob}],
                "reviewer": "fixture reviewer",
                "rationale": "Synthetic fixture judgment only",
            }],
        }, "oracle_id")

    def evaluate(self):
        return evaluation.report(
            self.repo, self.baseline, self.enriched, self.model,
            self.manifest, self.oracle, "c" * 40, "d" * 40,
        )

    def test_synthetic_positives_are_counted_without_verification_claim(self):
        report = self.evaluate()
        self.assertEqual(report["metrics"]["true_positive"], 1)
        self.assertEqual(report["metrics"]["false_positive"], 0)
        self.assertEqual(report["metrics"]["false_negative"], 0)
        self.assertEqual(report["evaluation_status"], "SYNTHETIC_SCHEMA_FIXTURE")
        self.assertEqual(report["experiment_id"], evaluation.digest({
            k: v for k, v in report.items() if k != "experiment_id"
        }))

    def test_false_annotation_and_unknown_cases_are_not_discarded(self):
        self.oracle["cases"][0]["expected"] = "NO_LINK"
        seal(self.oracle, "oracle_id")
        result = self.evaluate()
        self.assertEqual(result["metrics"]["false_positive"], 1)
        self.oracle["cases"][0]["expected"] = "UNJUDGED"
        seal(self.oracle, "oracle_id")
        result = self.evaluate()
        self.assertEqual(result["metrics"]["unresolved"], 1)
        self.assertEqual(result["metrics"]["true_positive"], 0)

    def test_missing_case_is_rejected(self):
        self.oracle["cases"] = []
        seal(self.oracle, "oracle_id")
        with self.assertRaisesRegex(ValueError, "account for every baseline"):
            self.evaluate()

    def test_stale_git_blob_is_rejected_even_with_new_receipt(self):
        self.oracle["cases"][0]["source"][0]["blob"] = "e" * 40
        seal(self.oracle, "oracle_id")
        with self.assertRaisesRegex(ValueError, "Git blob mismatch"):
            self.evaluate()

    def test_source_derived_prediction_cannot_claim_verified_safety(self):
        self.enriched["links"][0]["assessment"] = "SAFE_TO_REFACTOR"
        seal(self.enriched, "link_report_id")
        with self.assertRaisesRegex(ValueError, "cannot be promoted"):
            self.evaluate()

    def test_forged_report_receipt_is_refused(self):
        altered = copy.deepcopy(self.baseline)
        altered["status"] = "VERIFIED_SAFE"
        self.baseline = altered
        with self.assertRaisesRegex(ValueError, "noncanonical report_id"):
            self.evaluate()


if __name__ == "__main__":
    unittest.main()
