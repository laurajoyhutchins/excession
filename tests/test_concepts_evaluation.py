"""Structural and pending behavioral TDD for concepts evaluation."""
import copy
import json
import os
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/evaluation.v1.schema.json"
SAMPLE = {
    "schema": "excession.concepts-evaluation.v1",
    "experiment_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "baseline_revision": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "semantic_revision": "cccccccccccccccccccccccccccccccccccccccc",
    "oracle_manifest_id": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "corpus": [
        {
            "repo": "synthetic-example",
            "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "purpose": "CONTRACT_NEGATIVE"
        }
    ],
    "metrics": {
        "true_positive": 0,
        "false_positive": 0,
        "false_negative": 0,
        "unresolved": 0,
        "wall_seconds": 0,
        "peak_rss_bytes": 0
    },
    "unsupported_cases": 0,
    "tool_versions": {
        "contract": "v1"
    },
    "evaluation_status": "SYNTHETIC_SCHEMA_FIXTURE"
}


class ConceptsContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        schema = json.loads(SCHEMA.read_text())
        Draft202012Validator.check_schema(schema)
        cls.validator = Draft202012Validator(schema)

    def test_synthetic_valid_fixture(self):
        self.assertEqual(list(self.validator.iter_errors(SAMPLE)), [])

    def test_reject_undeclared_authority(self):
        value = copy.deepcopy(SAMPLE)
        value["certified"] = True
        self.assertTrue(list(self.validator.iter_errors(value)))

    def test_reject_illegal_status(self):
        value = copy.deepcopy(SAMPLE)
        value["metrics"]["false_positive"] = -1
        self.assertTrue(list(self.validator.iter_errors(value)))

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "Independent evaluation harness not implemented")
    def test_held_out_oracle_beats_baseline_without_false_evidence_promotion(self):
        self.fail("Implement frozen held-out corpus oracle and baseline-vs-semantic comparison")


if __name__ == "__main__":
    unittest.main()

# Separate executable acceptance for the evaluator is in test_concepts_evaluator.py.
