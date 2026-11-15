"""Structural contract TDD for j-query; behavioral acceptance is not established."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/investigation.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.investigation.v1\",\"source_commit\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"query\":{\"operation\":\"EXPLAIN\",\"focus_subject_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"facet\":\"SCIP\",\"require\":\"INCLUDE_CANDIDATES\"},\"answer\":{\"answerability\":\"PARTIAL\",\"evidence_receipts\":[\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"],\"coverage_receipts\":[],\"unknown_ids\":[]},\"limit\":{\"max_nodes\":500,\"max_edges\":1000},\"interpretation\":\"NO_GENERIC_DEPENDENCY_EDGE_PROMOTION\"}")
BAD = json.loads("[[\"answer.answerability\",\"ALL_RELATIONSHIPS_PROVEN\"],[\"limit.max_nodes\",0],[\"interpretation\",\"ALL_EDGES_ARE_IMPORTS\"]]")

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

    @unittest.skip("TDD pending: Implement and replay a multi-hop evidence explanation in h11 plus an unknown coverage result, using existing Rust CLI grammar.")
    def test_real_behavior_and_external_ground_truth(self):
        self.fail("Must execute real, independent acceptance instead of synthetic fixtures")

if __name__ == "__main__":
    unittest.main()
