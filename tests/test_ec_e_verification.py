"""Structural-only TDD for e-verification; runtime acceptance deliberately pending."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

SCHEMA = json.loads((Path(__file__).resolve().parents[1] / "contracts/evidence-core/verification.v1.schema.json").read_text())
GOOD = json.loads("{\"schema\":\"excession.claim-verification.v1\",\"claim_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"claim\":{\"predicate\":\"MAY_CALL\",\"subject_id\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"scope_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"assessment\":\"CANDIDATE\"},\"support_receipt_ids\":[\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"],\"counterevidence_receipt_ids\":[],\"obligation\":{\"obligation_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"kind\":\"OBSERVE_EXECUTION\",\"scope_id\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"},\"verification\":{\"status\":\"NOT_RUN\",\"receipt_id\":null,\"authority\":\"NONE\"},\"prohibition\":\"SOURCE_REPLAY_NEVER_IMPLIES_SEMANTIC_OR_SECURITY_VALIDITY\"}")
BAD = json.loads("[[\"claim.assessment\",\"PROVED_EXPLOITABLE\"],[\"verification.status\",\"GLOBAL_VERIFIED\"],[\"prohibition\",\"CODEQL_AUDIT_PROVES_CVE\"]]")

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

    @unittest.skip("TDD pending: Rust verification rejects unsupported predicate promotions, invalidated inputs and bogus positive scanner verdicts.")
    def test_independent_rust_behavior(self):
        self.fail("Must implement, execute and audit the real behavior")

if __name__ == "__main__":
    unittest.main()
