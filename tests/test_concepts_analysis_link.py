"""Structural and pending behavioral TDD for concepts analysis-link."""
import copy
import json
import os
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/analysis-link.v1.schema.json"
SAMPLE = {
    "schema": "excession.concepts-analysis-link.v1",
    "link_report_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "source_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "snapshot_id": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "analysis_report_id": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "analysis_kind": "REFACTOR_CANDIDATES",
    "links": [
        {
            "analysis_candidate_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "concept_claim_ids": [
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            ],
            "role": "CONCEPT_ANNOTATION",
            "assessment": "ANNOTATION_ONLY"
        }
    ],
    "coverage_status": "PARTIAL"
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
        value["links"][0]["assessment"] = "SAFE_TO_REFACTOR"
        self.assertTrue(list(self.validator.iter_errors(value)))

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "Semantic analysis integration not added")
    def test_original_refactoring_report_is_unchanged_by_annotation(self):
        self.fail("Implement pinned h11 integration: baseline candidate IDs must remain identical")


if __name__ == "__main__":
    unittest.main()
