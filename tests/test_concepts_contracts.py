"""Structural contract checks for proposed concepts model. No implementation assertions."""
import copy
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/model.v1.schema.json"


def model():
    return {
        "schema": "excession.concepts-model.v1",
        "model_id": "a" * 64,
        "source_commit": "b" * 40,
        "source_tree": "c" * 40,
        "subjects": [{
            "subject_id": "d" * 64, "kind": "SYMBOL", "source_commit": "b" * 40,
            "path": "src/lib.rs", "blob": "e" * 40, "symbol": "scip-rust cargo demo 1.0.0 foo",
            "producer": "rust-analyzer", "producer_receipt_id": "f" * 64
        }],
        "claims": [{
            "claim_id": "1" * 64, "predicate": "POSSIBLY_SAME_RESPONSIBILITY",
            "subjects": ["d" * 64], "assessment": "PROPOSED",
            "behavioral_verification": "UNTESTED",
            "support": [{"receipt_id": "f" * 64, "role": "SUPPORTS",
                         "observation_kind": "SCIP_REFERENCE"}]
        }],
        "concepts": [{
            "concept_id": "2" * 64, "revision_id": "3" * 64,
            "name": "Connection lifecycle", "definition": "Lifecycle of a connection",
            "membership_claims": [], "review_status": "UNREVIEWED"
        }],
        "coverage": {"status": "PARTIAL", "producer_receipts": ["f" * 64],
                     "gaps": [{"reason": "UNKNOWN_COVERAGE"}]}
    }


class ConceptsModelContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads(SCHEMA.read_text())
        Draft202012Validator.check_schema(cls.schema)
        cls.validator = Draft202012Validator(cls.schema)

    def assert_rejected(self, document):
        self.assertNotEqual(list(self.validator.iter_errors(document)), [])

    def test_example_is_structurally_valid(self):
        self.assertEqual(list(self.validator.iter_errors(model())), [])

    def test_untyped_relationship_rejected(self):
        value = model()
        value["claims"][0]["predicate"] = "RELATED_TO"
        self.assert_rejected(value)

    def test_evidence_role_must_be_explicit(self):
        value = model()
        del value["claims"][0]["support"][0]["role"]
        self.assert_rejected(value)

    def test_no_fabricated_verification_state(self):
        value = model()
        value["claims"][0]["behavioral_verification"] = "CERTIFIED_BY_MODEL"
        self.assert_rejected(value)

    def test_unsupported_source_path_rejected(self):
        value = model()
        value["subjects"][0]["path"] = "../outside.rs"
        self.assert_rejected(value)

    def test_unrecognized_coverage_state_rejected(self):
        value = model()
        value["coverage"]["status"] = "EVERYTHING_COMPLETE"
        self.assert_rejected(value)

    def test_extra_authority_field_rejected(self):
        value = model()
        value["claims"][0]["llm_verified"] = True
        self.assert_rejected(value)


if __name__ == "__main__":
    unittest.main()
