"""Schema contract tests are active; implementation acceptance is deliberately skipped."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class RepositoryDiscoveryContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.discovery.v1","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","repository_path":"/checkout/h11","roots":["h11","tests"],"adapters":[{"name":"python","availability":"AVAILABLE"},{"name":"scip","availability":"UNAVAILABLE"}],"index_path":"/cache/excession/h11.sqlite","coverage":{"status":"PARTIAL","analyzed_sources":12,"gaps":[{"reason":"MISSING_ADAPTER"}]}}''')

    def test_accepts_valid_draft_shape(self):
        validate_schema("discovery.v1.schema.json", self.report())

    def test_rejects_invalid_git_oid(self):
        report = self.report()
        report["source_commit"] = "HEAD"
        with self.assertRaises(ValidationError):
            validate_schema("discovery.v1.schema.json", report)

    def test_rejects_invented_adapter_availability(self):
        report = self.report()
        report["adapters"][0]["availability"] = "VERIFIED"
        with self.assertRaises(ValidationError):
            validate_schema("discovery.v1.schema.json", report)

    def test_rejects_missing_coverage(self):
        report = self.report()
        report.pop("coverage")
        with self.assertRaises(ValidationError):
            validate_schema("discovery.v1.schema.json", report)

    @unittest.skip("Behavioral TDD pending: discovery on two pinned unfamiliar repos without user-configured roots")
    def test_pending_native_behavior(self):
        self.fail("native implementation not yet present")


if __name__ == "__main__":
    unittest.main()
