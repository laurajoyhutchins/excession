"""Facet-scoped coverage contract, structural TDD and forbidden promotions."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/coverage.v1.schema.json").read_text())
GOOD = {
    "schema": "excession.evidence-coverage.v1",
    "report_id": "c" * 64,
    "producer_receipt_id": "b" * 64,
    "source_commit": "a" * 40,
    "facet": "SCIP_REFERENCES",
    "scope": {
        "selection": "src/**",
        "configuration": "rust-analyzer default",
        "configuration_resolved": True,
        "universe": "FINITE_DECLARED",
        "declared_total": 3,
    },
    "status": "PARTIAL",
    "examined": 2,
    "unsupported": 1,
    "unresolved": 0,
    "excluded": 0,
    "truncated": False,
    "limitations": ["indexer excludes generated code"],
    "negative_claim_scope": "INDEPENDENT_ORACLE_REQUIRED",
}

class CoverageContractTests(unittest.TestCase):
    def setUp(self):
        self.validator = Draft202012Validator(SCHEMA)

    def reject(self, value):
        with self.assertRaises(ValidationError):
            self.validator.validate(value)

    def test_valid_partial_report(self):
        Draft202012Validator.check_schema(SCHEMA)
        self.validator.validate(GOOD)

    def test_missing_or_unreviewed_fields_fail(self):
        for field in SCHEMA["required"]:
            value = copy.deepcopy(GOOD)
            value.pop(field)
            with self.subTest(field=field):
                self.reject(value)
        self.reject({**GOOD, "can_skip_tests": True})

    def test_no_global_negative_claim_or_invalid_facet(self):
        self.reject({**GOOD, "negative_claim_scope": "ALL_CALLERS_ABSENT"})
        self.reject({**GOOD, "facet": "ALL_RUNTIME_DEPENDENCIES"})

    def test_no_complete_claim_with_missing_configuration_or_gaps(self):
        for field, replacement in (
            ("unsupported", 1), ("unresolved", 1), ("excluded", 1), ("truncated", True)
        ):
            value = copy.deepcopy(GOOD)
            value["status"] = "COMPLETE_DECLARED_SCOPE"
            value["unsupported"] = value["unresolved"] = value["excluded"] = 0
            value["truncated"] = False
            value[field] = replacement
            with self.subTest(field=field):
                self.reject(value)
        value = copy.deepcopy(GOOD)
        value["status"] = "COMPLETE_DECLARED_SCOPE"
        value["scope"]["configuration_resolved"] = False
        self.reject(value)

    def test_unbounded_has_no_declared_total(self):
        value = copy.deepcopy(GOOD)
        value["scope"]["universe"] = "UNBOUNDED_OR_UNKNOWN"
        self.reject(value)
        value["scope"]["declared_total"] = None
        self.validator.validate(value)

    @unittest.skip("TDD pending: real SCIP indexer and independent Git source-coverage oracle")
    def test_independent_native_coverage_semantics(self):
        self.fail("Producer completeness not established by declared counts")

if __name__ == "__main__":
    unittest.main()
