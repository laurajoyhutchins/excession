"""Structural contract TDD for k-acceptance; behavioral acceptance is not established."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/acceptance.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.evidence-acceptance.v1\",\"evaluation_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"source_commit\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"corpus\":{\"name\":\"rust-artifact-fixture\",\"pinned_revision\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"oracle_digest\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"},\"tool_versions\":[{\"tool\":\"excession\",\"version\":\"0.2.0\"}],\"cases\":[{\"kind\":\"NEGATIVE\",\"expected\":\"UNKNOWN\",\"observed\":\"NOT_RUN\"}],\"measured\":{\"status\":\"NOT_RUN\",\"wall_ms\":null,\"peak_rss_bytes\":null,\"artifact_bytes\":null},\"scope\":\"REPLAY_IS_NOT_SEMANTIC_VALIDITY\"}")
BAD = json.loads("[[\"measured.status\",\"MEASURED_FROM_SYNTHETIC_FIXTURES\"],[\"cases\",[{\"kind\":\"NEGATIVE\",\"expected\":\"NOT_RUN\",\"observed\":\"ACCEPT\"}]],[\"scope\",\"DB_SELF_HASH_PROVES_CORRECTNESS\"]]")

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

    @unittest.skip("TDD pending: Execute pinned h11, Rust binary/REA and Linux scope experiments; independently adjudicate false-positives and measured resource costs.")
    def test_real_behavior_and_external_ground_truth(self):
        self.fail("Must execute real, independent acceptance instead of synthetic fixtures")

if __name__ == "__main__":
    unittest.main()
