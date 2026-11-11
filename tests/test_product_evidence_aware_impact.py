"""Wire-shape tests only; native behavior remains pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema

class Product6SchemaTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.impact.v1","report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","base_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_commit":"cccccccccccccccccccccccccccccccccccccccc","change_inventory_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","effects":[{"entity":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/a.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"assessment":"CANDIDATE_PATH","hops":[{"source":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/a.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"target":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/b.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"relation":"STATIC_IMPORT","evidence":[{"receipt_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","producer":"python-ast","observation_kind":"STATIC_IMPORT_CANDIDATE","assessment":"CANDIDATE","source":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/a.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"}}]}]}],"unknown":[{"reason":"DYNAMIC_RELATIONSHIP"}],"coverage":{"status":"PARTIAL","analyzed_sources":2,"gaps":[{"reason":"DYNAMIC_RELATIONSHIP"}]},"safe_to_skip_unlisted_tests":false,"ontology_links":[{"snapshot_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","link_report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","assessment":"ANNOTATION_ONLY"}]}''')

    def test_valid_wire_shape(self):
        validate_schema("impact.v1.schema.json", self.report())

    def test_reject_unsafe_test_skip(self):
        report = self.report()
        report["safe_to_skip_unlisted_tests"] = True
        with self.assertRaises(ValidationError):
            validate_schema("impact.v1.schema.json", report)

    def test_reject_unearned_proof(self):
        report = self.report()
        report["effects"][0]["assessment"] = "PROVED_TEST_SAFE"
        with self.assertRaises(ValidationError):
            validate_schema("impact.v1.schema.json", report)

    def test_reject_empty_hop_evidence(self):
        report = self.report()
        report["effects"][0]["hops"][0]["evidence"] = []
        with self.assertRaises(ValidationError):
            validate_schema("impact.v1.schema.json", report)

    def test_reject_ontology_as_dependency(self):
        report = self.report()
        report["ontology_links"][0]["assessment"] = "PROVED_DEPENDENCY"
        with self.assertRaises(ValidationError):
            validate_schema("impact.v1.schema.json", report)

    @unittest.skip("TDD pending independent Git/native implementation")
    def test_end_to_end_behavior(self):
        self.fail("not implemented")

if __name__ == "__main__":
    unittest.main()
