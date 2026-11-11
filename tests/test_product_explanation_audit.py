"""Wire-shape tests only; native behavior remains pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema

class Product7SchemaTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.explanation.v1","report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","claim_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","steps":[{"source":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/a.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"target":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/b.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"relation":"STATIC_IMPORT","evidence":[{"receipt_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","producer":"python-ast","observation_kind":"STATIC_IMPORT_CANDIDATE","assessment":"CANDIDATE","source":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/a.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"}}]}],"coverage":{"status":"PARTIAL","analyzed_sources":2,"gaps":[]},"ontology_links":[{"snapshot_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","link_report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","role":"ANNOTATION_ONLY"}]}''')

    def test_valid_wire_shape(self):
        validate_schema("explanation.v1.schema.json", self.report())

    def test_reject_unsupported_missing_evidence(self):
        report = self.report()
        report["steps"][0]["evidence"] = []
        with self.assertRaises(ValidationError):
            validate_schema("explanation.v1.schema.json", report)

    def test_reject_unearned_ontology_promotion(self):
        report = self.report()
        report["ontology_links"][0]["role"] = "PROVED_EQUIVALENCE"
        with self.assertRaises(ValidationError):
            validate_schema("explanation.v1.schema.json", report)

    def test_reject_missing_claim_id(self):
        report = self.report()
        report.pop("claim_id")
        with self.assertRaises(ValidationError):
            validate_schema("explanation.v1.schema.json", report)

    def test_valid_separate_audit_domains(self):
        validate_schema("audit-result.v1.schema.json", {
            "schema":"excession.audit-result.v1","report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "schema_validation":"PASS","git_authentication":"PASS",
            "replay":"PASS","semantic_correctness":"NOT_EVALUATED"
        })

    def test_cannot_confirm_semantics_without_oracle(self):
        audit = {
            "schema":"excession.audit-result.v1","report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "schema_validation":"PASS","git_authentication":"PASS",
            "replay":"PASS","semantic_correctness":"INDEPENDENTLY_TESTED"
        }
        with self.assertRaises(ValidationError):
            validate_schema("audit-result.v1.schema.json", audit)

    @unittest.skip("TDD pending independent Git/native implementation")
    def test_end_to_end_behavior(self):
        self.fail("not implemented")

if __name__ == "__main__":
    unittest.main()
