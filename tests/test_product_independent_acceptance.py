"""Proposed contract shape tests; future acceptance is explicitly pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class Product8ContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.acceptance-case.v1","case_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","repository":"pinned-upstream-h11","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","oracle_kind":"INDEPENDENT_GIT_AND_NATIVE","expected_relationships":[],"mutations":["INSERT_FALSE_EDGE","DELETE_TRUE_EDGE"],"measurement_status":"NOT_RUN","coverage_status":"PARTIAL"}''')

    def test_accepts_schema_fixture(self):
        validate_schema("acceptance-case.v1.schema.json", self.report())

    def test_rejects_sql_self_replay_oracle(self):
        document = self.report()
        document["oracle_kind"] = "SQL_SELF_REPLAY"
        with self.assertRaises(ValidationError):
            validate_schema("acceptance-case.v1.schema.json", document)

    def test_rejects_no_mutation_cases(self):
        document = self.report()
        document["mutations"] = []
        with self.assertRaises(ValidationError):
            validate_schema("acceptance-case.v1.schema.json", document)

    def test_rejects_fake_measurement_status(self):
        document = self.report()
        document["measurement_status"] = "BENCHMARK_PROVEN"
        with self.assertRaises(ValidationError):
            validate_schema("acceptance-case.v1.schema.json", document)

    @unittest.skip("TDD pending: run release/real-repo acceptance from actual installed artifacts")
    def test_pending_behavior(self):
        self.fail("not implemented yet")


if __name__ == "__main__":
    unittest.main()
