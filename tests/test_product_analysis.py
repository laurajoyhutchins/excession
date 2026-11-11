"""Schema contract tests are active; implementation acceptance is deliberately skipped."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class AnalyzeWorkflowContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.analysis.v1","report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","status":"PARTIAL","observation_receipts":["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],"relationships":[],"coverage":{"status":"PARTIAL","analyzed_sources":12,"gaps":[{"reason":"UNRESOLVED_RELATIONSHIP"}]},"ontology_links":[{"snapshot_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","link_report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","role":"ANNOTATION_ONLY"}]}''')

    def test_accepts_valid_draft_shape(self):
        validate_schema("analysis.v1.schema.json", self.report())

    def test_rejects_semantic_promotion(self):
        report = self.report()
        report["relationships"].append({"source": {"source_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "source_tree": "cccccccccccccccccccccccccccccccccccccccc", "path": "app.py", "blob": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "mode": "100644"}, "target":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"dep.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"kind":"BEHAVIORALLY_EQUIVALENT","evidence":[]})
        with self.assertRaises(ValidationError):
            validate_schema("analysis.v1.schema.json", report)

    def test_rejects_ontology_as_authority(self):
        report = self.report()
        report["ontology_links"][0]["role"] = "VERIFIED_DEPENDENCY"
        with self.assertRaises(ValidationError):
            validate_schema("analysis.v1.schema.json", report)

    def test_rejects_unversioned(self):
        report = self.report()
        report["schema"] = "excession.analysis"
        with self.assertRaises(ValidationError):
            validate_schema("analysis.v1.schema.json", report)

    @unittest.skip("Behavioral TDD pending: analyze . returns authentic report, handles missing adapters, preserves clang subcommand")
    def test_pending_native_behavior(self):
        self.fail("native implementation not yet present")


if __name__ == "__main__":
    unittest.main()
