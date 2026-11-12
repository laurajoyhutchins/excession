"""Proposed contract shape tests; future acceptance is explicitly pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class Product9ContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.installed-smoke.v1","case_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target":"x86_64-unknown-linux-musl","archive_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","steps":[{"name":"INSTALL","status":"NOT_RUN"},{"name":"DOCTOR","status":"NOT_RUN"},{"name":"ANALYZE","status":"NOT_RUN"},{"name":"IMPACT","status":"NOT_RUN"},{"name":"EXPLAIN","status":"NOT_RUN"},{"name":"AUDIT","status":"NOT_RUN"}],"status":"NOT_RUN"}''')

    def test_accepts_schema_fixture(self):
        validate_schema("installed-smoke.v1.schema.json", self.report())

    def test_rejects_missing_required_steps(self):
        document = self.report()
        document["steps"] = []
        with self.assertRaises(ValidationError):
            validate_schema("installed-smoke.v1.schema.json", document)

    def test_rejects_pretend_attested_status(self):
        document = self.report()
        document["status"] = "ATTESTED_CORRECT"
        with self.assertRaises(ValidationError):
            validate_schema("installed-smoke.v1.schema.json", document)

    def test_rejects_invalid_archive_digest(self):
        document = self.report()
        document["archive_sha256"] = "missing"
        with self.assertRaises(ValidationError):
            validate_schema("installed-smoke.v1.schema.json", document)

    @unittest.skip("TDD pending: run release/real-repo acceptance from actual installed artifacts")
    def test_pending_behavior(self):
        self.fail("not implemented yet")


if __name__ == "__main__":
    unittest.main()
