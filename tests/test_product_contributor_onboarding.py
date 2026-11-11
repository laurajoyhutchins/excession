"""Proposed contract shape tests; future acceptance is explicitly pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class Product10ContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.onboarding-example.v1","example_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","supported_release_status":"PROPOSED_NOT_SHIPPED","commands":[{"command":"excession analyze . --json","role":"ANALYZE"}],"required_tools":["git"]}''')

    def test_accepts_schema_fixture(self):
        validate_schema("onboarding-example.v1.schema.json", self.report())

    def test_rejects_invented_supported_status(self):
        document = self.report()
        document["supported_release_status"] = "READY_FOR_PRODUCTION"
        with self.assertRaises(ValidationError):
            validate_schema("onboarding-example.v1.schema.json", document)

    def test_rejects_empty_commands(self):
        document = self.report()
        document["commands"] = []
        with self.assertRaises(ValidationError):
            validate_schema("onboarding-example.v1.schema.json", document)

    def test_rejects_unknown_command_role(self):
        document = self.report()
        document["commands"][0]["role"] = "EVALUATE_CVES"
        with self.assertRaises(ValidationError):
            validate_schema("onboarding-example.v1.schema.json", document)

    @unittest.skip("TDD pending: run release/real-repo acceptance from actual installed artifacts")
    def test_pending_behavior(self):
        self.fail("not implemented yet")


if __name__ == "__main__":
    unittest.main()
