"""Source-only impact v3 wire-shape tests. Semantic correctness remains unproved."""
import unittest

from jsonschema import ValidationError
from product_contract_support import validate_schema


class ProductImpactSchemaTests(unittest.TestCase):
    def report(self):
        base, head, tree = "a" * 40, "c" * 40, "c" * 40
        digest = "b" * 64
        source = {
            "source_commit": base, "source_tree": tree,
            "path": "src/a.py", "blob": base, "mode": "100644",
        }
        target = dict(source, path="src/b.py")
        receipt = {
            "receipt_id": digest, "producer": "python-ast",
            "observation_kind": "STATIC_IMPORT_CANDIDATE",
            "assessment": "CANDIDATE", "source": source,
        }
        return {
            "schema": "excession.impact.v3",
            "report_id": digest,
            "base_commit": base,
            "head_commit": head,
            "change_inventory_id": digest,
            "candidate_source_impacts": [{
                "entity": source,
                "assessment": "CANDIDATE_PATH",
                "next_hop": {
                    "source": source, "target": target,
                    "relation": "STATIC_IMPORT", "evidence": [receipt],
                },
                "distance_to_change": 1,
            }],
            "candidate_source_count": 1,
            "candidate_source_enumeration_complete": True,
            "native_reference_impacts": [],
            "native_reference_count": 0,
            "native_reference_enumeration_complete": True,
            "unknown": [{"reason": "DYNAMIC_RELATIONSHIP"}],
            "coverage": {
                "status": "PARTIAL", "analyzed_sources": 2,
                "gaps": [{"reason": "DYNAMIC_RELATIONSHIP"}],
            },
            "safe_to_skip_unlisted_tests": False,
        }

    def check(self, document):
        validate_schema("impact.v3.schema.json", document)

    def test_source_only_report(self):
        self.check(self.report())

    def test_reject_unsafe_test_skip(self):
        report = self.report()
        report["safe_to_skip_unlisted_tests"] = True
        with self.assertRaises(ValidationError):
            self.check(report)

    def test_reject_unearned_proof(self):
        report = self.report()
        report["candidate_source_impacts"][0]["assessment"] = "PROVED_TEST_SAFE"
        with self.assertRaises(ValidationError):
            self.check(report)

    def test_reject_empty_hop_evidence(self):
        report = self.report()
        report["candidate_source_impacts"][0]["next_hop"]["evidence"] = []
        with self.assertRaises(ValidationError):
            self.check(report)

    def test_reject_legacy_path_and_ontology_fields(self):
        for field in ("effects", "path_effects_status", "ontology_links"):
            with self.subTest(field=field):
                report = self.report()
                report[field] = []
                with self.assertRaises(ValidationError):
                    self.check(report)

    def test_require_complete_bound_metadata(self):
        for field in ("candidate_source_count", "candidate_source_enumeration_complete",
                      "native_reference_count", "native_reference_enumeration_complete"):
            with self.subTest(field=field):
                report = self.report()
                del report[field]
                with self.assertRaises(ValidationError):
                    self.check(report)


if __name__ == "__main__":
    unittest.main()
