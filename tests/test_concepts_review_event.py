"""Structural and pending behavioral TDD for concepts review-event."""
import copy
import json
import os
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/review-event.v1.schema.json"
SAMPLE = {
    "schema": "excession.concepts-review-event.v1",
    "event_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "action": "REJECT",
    "concept_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "concept_revision_id": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "reviewer": "reviewer@example.invalid",
    "scope": {
        "source_commit": "dddddddddddddddddddddddddddddddddddddddd",
        "model_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    },
    "evidence_receipts": [
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    ],
    "rationale": "A method name match does not establish shared behavior",
    "evidence_assessment": "REVIEWED_INTERPRETATION_NOT_BEHAVIOR_PROOF"
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
        value["evidence_assessment"] = "BEHAVIORALLY_CERTIFIED"
        self.assertTrue(list(self.validator.iter_errors(value)))

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "Review event implementation not added")
    def test_review_does_not_promote_behavioral_verification(self):
        self.fail("Implement historical review fixture: accepted concept remains UNTESTED")


if __name__ == "__main__":
    unittest.main()
