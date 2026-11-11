"""Structural TDD for a-foundation. These do not establish runtime behavior."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/boundary.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.evidence-boundary.v1\",\"source_authority\":\"GIT_OBJECTS\",\"relation_authority\":\"RUST_SQLITE\",\"current_git_format\":\"SHA1_ONLY\",\"legacy_reports\":[\"excession.semantic-observations.v1\",\"excession.evidence-query.v1\"],\"meaning_preservation\":\"PRODUCER_NATIVE_RELATIONS_NOT_PROMOTED\",\"concept_layer\":\"DERIVED_NOT_AN_AUTHORITY\"}")
BAD = json.loads("[[\"concept_layer\",\"NEW_SOURCE_AUTHORITY\"],[\"current_git_format\",\"ALL_GIT_FORMATS\"]]")

class ContractTests(unittest.TestCase):
    def test_schema_is_valid_and_example_conforms(self):
        Draft202012Validator.check_schema(SCHEMA)
        Draft202012Validator(SCHEMA).validate(GOOD)

    def test_refuses_unknown_fields_and_missing_required_fields(self):
        validator = Draft202012Validator(SCHEMA)
        extra = copy.deepcopy(GOOD)
        extra["undocumented_authority"] = "trusted"
        with self.assertRaises(ValidationError):
            validator.validate(extra)
        for key in SCHEMA["required"]:
            missing = copy.deepcopy(GOOD)
            missing.pop(key)
            with self.subTest(key=key), self.assertRaises(ValidationError):
                validator.validate(missing)

    def test_refuses_invalid_semantic_shapes(self):
        validator = Draft202012Validator(SCHEMA)
        for name, value in BAD:
            wrong = copy.deepcopy(GOOD)
            path = name.split(".")
            target = wrong
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
            with self.subTest(field=name), self.assertRaises(ValidationError):
                validator.validate(wrong)

    @unittest.skip("TDD pending: A Rust cross-schema compatibility validator proves old reports retain byte-for-byte meaning after core normalization.")
    def test_real_rust_behavior_and_independent_audit(self):
        self.fail("Implement and independently verify the behavioral contract")

if __name__ == "__main__":
    unittest.main()
