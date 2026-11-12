"""Proposed contract shape tests; future acceptance is explicitly pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class Product11ContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.release-manifest.v1","version":"0.2.0-alpha.1","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","assets":[{"target":"x86_64-unknown-linux-musl","filename":"excession-v0.2.0-alpha.1-x86_64-unknown-linux-musl.tar.gz","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],"attestation_status":"NOT_ATTESTED"}''')

    def test_accepts_schema_fixture(self):
        validate_schema("release-manifest.v1.schema.json", self.report())

    def test_rejects_missing_source_commit(self):
        document = self.report()
        document.pop("source_commit")
        with self.assertRaises(ValidationError):
            validate_schema("release-manifest.v1.schema.json", document)

    def test_rejects_claims_attestation_without_receipt(self):
        document = self.report()
        document["attestation_status"] = "ATTESTED"
        with self.assertRaises(ValidationError):
            validate_schema("release-manifest.v1.schema.json", document)

    def test_rejects_invalid_archive_hash(self):
        document = self.report()
        document["assets"][0]["sha256"] = "sha256:not-real"
        with self.assertRaises(ValidationError):
            validate_schema("release-manifest.v1.schema.json", document)

    @unittest.skip("TDD pending: run release/real-repo acceptance from actual installed artifacts")
    def test_pending_behavior(self):
        self.fail("not implemented yet")


if __name__ == "__main__":
    unittest.main()
