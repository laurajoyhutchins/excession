"""Evidence snapshot contract: structural validation is not source authentication."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/persistence.v1.schema.json").read_text())
SHA = "b" * 64
GOOD = {
    "schema": "excession.evidence-snapshot.v1",
    "snapshot_id": SHA,
    "source_commit": "a" * 40,
    "source_authentication": "NOT_CHECKED",
    "receipt_ids": ["c" * 64],
    "observation_ids": ["d" * 64],
}

class SnapshotContractTests(unittest.TestCase):
    def setUp(self):
        self.validator = Draft202012Validator(SCHEMA)

    def test_valid_versioned_snapshot(self):
        Draft202012Validator.check_schema(SCHEMA)
        self.validator.validate(GOOD)

    def test_no_extra_authority_and_all_fields_required(self):
        extra = copy.deepcopy(GOOD)
        extra["verified"] = True
        with self.assertRaises(ValidationError):
            self.validator.validate(extra)
        for key in SCHEMA["required"]:
            value = copy.deepcopy(GOOD)
            del value[key]
            with self.subTest(key=key), self.assertRaises(ValidationError):
                self.validator.validate(value)

    def test_no_fabricated_complete_or_signed_status(self):
        for status in ("GIT_AUTHENTICATED", "SEMANTICALLY_VERIFIED", "COMPLETE_DEPENDENCIES"):
            value = {**GOOD, "source_authentication": status}
            with self.subTest(status=status), self.assertRaises(ValidationError):
                self.validator.validate(value)

    def test_bad_hashes_and_duplicate_record_ids_refused(self):
        for field in ("snapshot_id", "source_commit"):
            value = copy.deepcopy(GOOD)
            value[field] = "bad"
            with self.subTest(field=field), self.assertRaises(ValidationError):
                self.validator.validate(value)
        for field in ("receipt_ids", "observation_ids"):
            value = copy.deepcopy(GOOD)
            value[field] = ["c" * 64, "c" * 64]
            with self.subTest(field=field), self.assertRaises(ValidationError):
                self.validator.validate(value)

    @unittest.skip("TDD pending: independent upstream SCIP oracle and exact Git source authentication")
    def test_real_producer_and_git_binding(self):
        self.fail("SQLite audit verifies integrity, not native producer correctness")

if __name__ == "__main__":
    unittest.main()
