"""Contract checks execute now; future CLI behaviors remain explicitly unimplemented."""
import unittest
from jsonschema import ValidationError

from product_contract_support import validate_schema

H = "a" * 40
D = "b" * 64


class PublicContractTests(unittest.TestCase):
    def report(self):
        return {
            "schema": "excession.cli-result.v1",
            "operation": "ANALYZE",
            "status": "PARTIAL",
            "report_id": D,
            "source_commit": H,
            "coverage": {
                "status": "PARTIAL",
                "analyzed_sources": 2,
                "gaps": [{"reason": "DYNAMIC_RELATIONSHIP", "path": "src/app.py"}],
            },
        }

    def test_valid_summary(self):
        validate_schema("cli-result.v1.schema.json", self.report())

    def test_rejects_fabricated_complete_status(self):
        report = self.report()
        report["coverage"]["status"] = "ALL_RUNTIME_DEPENDENCIES_PROVEN"
        with self.assertRaises(ValidationError):
            validate_schema("cli-result.v1.schema.json", report)

    def test_rejects_non_git_oid(self):
        report = self.report()
        report["source_commit"] = "not-a-git-object"
        with self.assertRaises(ValidationError):
            validate_schema("cli-result.v1.schema.json", report)

    def test_rejects_unversioned_result(self):
        report = self.report()
        report.pop("schema")
        with self.assertRaises(ValidationError):
            validate_schema("cli-result.v1.schema.json", report)

    def test_rejects_unrecognized_status(self):
        report = self.report()
        report["status"] = "PROVED_CORRECT"
        with self.assertRaises(ValidationError):
            validate_schema("cli-result.v1.schema.json", report)

    @unittest.skip("TDD: not implemented, CLI outputs versioned report + stable exit code")
    def test_cli_end_to_end(self):
        raise AssertionError("implement the public CLI contract")


if __name__ == "__main__":
    unittest.main()
