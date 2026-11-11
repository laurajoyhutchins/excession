"""Structural-only TDD for f-unknowns; runtime acceptance deliberately pending."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

SCHEMA = json.loads((Path(__file__).resolve().parents[1] / "contracts/evidence-core/unknown-event.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.unknown-event.v1\",\"unknown_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"question\":\"Which indirect target can be called?\",\"scope_commit\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"revision\":1,\"expected_previous_revision\":null,\"state\":\"OPEN\",\"support_receipts\":[\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"],\"counterevidence_receipts\":[],\"required_evidence\":[\"RUNTIME_WITNESS\"],\"action\":\"CREATE\",\"resolution_receipt\":null,\"event_semantics\":\"APPEND_ONLY_WITH_EXPECTED_REVISION\"}")
BAD = json.loads("[[\"revision\",0],[\"state\",\"CERTIFIED_SAFE\"],[\"event_semantics\",\"OVERWRITE_HEAD\"]]")

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

    @unittest.skip("TDD pending: Persist concurrent unknown updates, reject stale expected revisions, and reopen resolution after contradictory new evidence.")
    def test_independent_rust_behavior(self):
        self.fail("Must implement, execute and audit the real behavior")

if __name__ == "__main__":
    unittest.main()
