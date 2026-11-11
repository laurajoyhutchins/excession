"""Structural contract TDD for i-rea; behavioral acceptance is not established."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/rea-import.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.rea-import.v1\",\"producer\":\"REA\",\"format\":\"EVIDENCE_V2_EXPORT\",\"bundle_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"artifact_sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"native_record_ids\":[\"ev_example1\"],\"import_status\":\"ACCEPTED_ARTIFACT_ONLY\",\"raw_retention\":\"RETAIN_ORIGINAL_PAYLOAD\",\"source_binding\":\"ARTIFACT_ONLY\",\"coverage\":{\"status\":\"PARTIAL\",\"limitations\":[\"decompiler missing function\"]},\"security\":\"IMPORT_ONLY_NO_UNTRUSTED_EXECUTION\"}")
BAD = json.loads("[[\"native_record_ids\",[\"function0\"]],[\"source_binding\",\"MATCHED_BY_FUNCTION_NAME\"],[\"security\",\"AUTO_EXECUTE_IMPORTED_BINARY\"]]")

class ContractTests(unittest.TestCase):
    def test_schema_and_positive_shape(self):
        Draft202012Validator.check_schema(SCHEMA)
        Draft202012Validator(SCHEMA).validate(GOOD)

    def test_missing_required_and_unreviewed_fields(self):
        validator = Draft202012Validator(SCHEMA)
        for field in SCHEMA["required"]:
            value = copy.deepcopy(GOOD)
            del value[field]
            with self.subTest(field=field), self.assertRaises(ValidationError):
                validator.validate(value)
        value = copy.deepcopy(GOOD)
        value["assumed_proof"] = True
        with self.assertRaises(ValidationError):
            validator.validate(value)

    def test_negative_semantic_shapes(self):
        validator = Draft202012Validator(SCHEMA)
        for name, bad_value in BAD:
            value = copy.deepcopy(GOOD)
            keys = name.split(".")
            node = value
            for key in keys[:-1]:
                node = node[key]
            node[keys[-1]] = bad_value
            with self.subTest(name=name), self.assertRaises(ValidationError):
                validator.validate(value)

    @unittest.skip("TDD pending: Import a pinned real REA Evidence v2 export, round-trip native IDs, refuse malformed paths/closure and audit artifact digest.")
    def test_real_behavior_and_external_ground_truth(self):
        self.fail("Must execute real, independent acceptance instead of synthetic fixtures")

if __name__ == "__main__":
    unittest.main()
