"""Structural-only TDD for h-artifacts; runtime acceptance deliberately pending."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

SCHEMA = json.loads((Path(__file__).resolve().parents[1] / "contracts/evidence-core/build-provenance.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.build-provenance.v1\",\"source_commit\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"source_tree\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"artifact\":{\"sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"size_bytes\":8192,\"representation\":\"EXECUTABLE\"},\"build\":{\"toolchain_digest\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"configuration_digest\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"source_input_receipts\":[\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"],\"invocation_receipt_id\":null,\"authority\":\"DECLARED_RECIPE_ONLY\"},\"source_artifact_relation\":\"REQUIRES_BUILD_EXECUTION_EVIDENCE\",\"execution_security\":\"UNTRUSTED_BUILDS_REQUIRE_ISOLATION\"}")
BAD = json.loads("[[\"artifact.sha256\",\"not-a-digest\"],[\"build.authority\",\"BUILT_FROM_GIT_PROVEN\"],[\"execution_security\",\"EXECUTE_IN_HOST_UNSANDBOXED\"]]")

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

    @unittest.skip("TDD pending: Rebuild a pinned Rust fixture, compare actual output digest to claimed artifact and reject mismatched or recipe-only attribution.")
    def test_independent_rust_behavior(self):
        self.fail("Must implement, execute and audit the real behavior")

if __name__ == "__main__":
    unittest.main()
