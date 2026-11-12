"""Structural-only TDD for g-comparison; runtime acceptance deliberately pending."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

SCHEMA = json.loads((Path(__file__).resolve().parents[1] / "contracts/evidence-core/comparison.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.evidence-comparison.v1\",\"left_subject_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"right_subject_id\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"correspondence\":{\"kind\":\"CANDIDATE_ONLY\",\"receipt_ids\":[]},\"facet\":\"CALLS\",\"left_coverage\":\"PARTIAL\",\"right_coverage\":\"UNKNOWN\",\"result\":\"UNKNOWN\",\"evidence_receipts\":[],\"unresolved_reasons\":[\"right inventory not complete\"],\"interpretation\":\"FACET_SCOPED_NOT_BEHAVIORAL_EQUIVALENCE\"}")
BAD = json.loads("[[\"result\",\"SAME_BEHAVIOR_PROVEN\"],[\"correspondence.kind\",\"SAME_NAME_SUFFICIENT\"],[\"interpretation\",\"ALL_DIMENSIONS_EQUIVALENT\"]]")

class ContractTests(unittest.TestCase):
    def test_good_shape_and_valid_schema(self):
        Draft202012Validator.check_schema(SCHEMA)
        Draft202012Validator(SCHEMA).validate(GOOD)

    def test_missing_fields_and_unknown_authority_refused(self):
        validator = Draft202012Validator(SCHEMA)
        for key in SCHEMA["required"]:
            sample = copy.deepcopy(GOOD)
            del sample[key]
            with self.subTest(missing=key), self.assertRaises(ValidationError):
                validator.validate(sample)
        sample = copy.deepcopy(GOOD)
        sample["new_unreviewed_semantics"] = True
        with self.assertRaises(ValidationError):
            validator.validate(sample)

    def test_negative_semantic_shapes_refused(self):
        validator = Draft202012Validator(SCHEMA)
        for key, value in BAD:
            sample = copy.deepcopy(GOOD)
            path = key.split(".")
            node = sample
            for segment in path[:-1]:
                node = node[segment]
            node[path[-1]] = value
            with self.subTest(key=key), self.assertRaises(ValidationError):
                validator.validate(sample)

    @unittest.skip("TDD pending: Implement deterministic Git revision comparison with coverage-aware one-sided absence and known-difference negative fixtures.")
    def test_independent_rust_behavior(self):
        self.fail("Must implement, execute and audit the real behavior")

if __name__ == "__main__":
    unittest.main()
