"""Security discovery contracts: fail closed without running either scanner."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import unittest

from jsonschema import ValidationError

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from validate_security_discovery_contract import validate_bundle, validate_schema  # noqa: E402


def fixture():
    return json.loads((ROOT / "tests/fixtures/security-discovery.v1.json").read_text())


class SecurityDiscoveryContractTests(unittest.TestCase):
    def assert_invalid(self, mutate, error=(ValueError, ValidationError)):
        item = fixture()
        mutate(item)
        with self.assertRaises(error):
            validate_bundle(item)

    def test_positive_source_bound_candidate(self):
        self.assertEqual(validate_bundle(fixture()),
                         {"scans": 1, "coverages": 1, "candidates": 1})

    def test_all_three_schemas_validate_independently(self):
        x = fixture()
        for kind, record in (("scan", x["scan"]),
                             ("coverage", x["coverages"][0]),
                             ("candidate", x["candidates"][0])):
            with self.subTest(kind=kind):
                validate_schema(kind, record)

    def test_missing_or_fake_source_binding(self):
        self.assert_invalid(lambda x: x["scan"]["source"].update(commit="main"))
        self.assert_invalid(lambda x: x["candidates"][0]["source"].update(blob="wrong"))
        self.assert_invalid(lambda x: x["candidates"][0]["source"].update(path="../x.py"))
        self.assert_invalid(lambda x: x["candidates"][0]["source"].update(path="/etc/passwd"))
        self.assert_invalid(lambda x: x["candidates"][0]["source"].update(path="src\\x.py"))
        self.assert_invalid(lambda x: x["candidates"][0]["source"].update(start_line=0))
        self.assert_invalid(lambda x: x["candidates"][0]["source"].update(end_line=1))

    def test_unreported_scanner_coverage_cannot_be_safe(self):
        self.assert_invalid(lambda x: x["coverages"][0].update(claim="SAFE"))
        self.assert_invalid(lambda x: x["candidates"][0].update(disposition="CONFIRMED_VULNERABILITY"))

    def test_incomplete_scanner_never_supplies_candidates(self):
        def corrupt(x):
            x["scan"]["execution"].update(status="FAILED", exit_code=1, errors=["query timeout"])
            x["scan"]["ingest"] = None
        self.assert_invalid(corrupt)
        self.assert_invalid(lambda x: x["scan"]["execution"].update(errors=["unexpected incomplete extraction"]))

    def test_scan_without_hits_still_has_coverable_target(self):
        x = fixture()
        x["candidates"] = []
        self.assertEqual(validate_bundle(x)["coverages"], 1)
        x["coverages"][0].update(claim="SCANNED_APPLICABILITY_UNPROVEN", rule_ids=[],
                                evidence_refs=["scanner:completed-but-no-rule-applicability-proof"])
        self.assertEqual(validate_bundle(x)["candidates"], 0)

    def test_not_scanned_does_not_need_fake_filename(self):
        x = fixture()
        x["scan"]["execution"].update(status="NOT_RUN", exit_code=None,
                                      errors=[], evidence_refs=[])
        x["scan"]["report"] = None
        x["scan"]["ingest"] = None
        x["coverages"][0].update(claim="NOT_SCANNED", rule_ids=[], evidence_refs=[])
        x["candidates"] = []
        self.assertEqual(validate_bundle(x)["coverages"], 1)
        self.assertNotIn("source", x["coverages"][0])

    def test_out_of_scope_language_is_not_scanned(self):
        x = fixture()
        x["candidates"] = []
        x["coverages"][0].update(claim="NOT_SCANNED", rule_ids=[], evidence_refs=[])
        x["coverages"][0]["target"]["language"] = "go"
        validate_bundle(x)
        x["coverages"][0].update(
            claim="SCANNED_APPLICABLE", rule_ids=["py/path-injection"],
            evidence_refs=["query-pack:metadata/path-injection"],
        )
        with self.assertRaisesRegex(ValueError, "unscanned language"):
            validate_bundle(x)

    def test_excluded_path_cannot_be_claimed_covered(self):
        def corrupt(x):
            x["scan"]["scope"]["excluded_paths"] = ["starlette"]
        self.assert_invalid(corrupt)
        x = fixture()
        x["candidates"] = []
        x["scan"]["scope"]["excluded_paths"] = ["starlette"]
        x["coverages"][0]["claim"] = "INCOMPLETE"
        x["coverages"][0]["evidence_refs"] = ["scope:excluded-starlette"]
        validate_bundle(x)

    def test_unknown_applicability_is_not_an_automatic_safe_or_positive(self):
        x = fixture()
        x["coverages"][0].update(
            claim="SCANNED_APPLICABILITY_UNPROVEN",
            rule_ids=[],
            evidence_refs=["unmapped-rule-metadata"],
        )
        self.assertEqual(validate_bundle(x)["candidates"], 1)

    def test_framework_specific_alert_preserved_but_not_marked_applicable(self):
        x = fixture()
        x["candidates"][0]["rule_id"] = "python.django.security.missing-ratelimit"
        x["coverages"][0].update(claim="SCANNED_NOT_APPLICABLE",
                                rule_ids=["python.django.security.missing-ratelimit"],
                                evidence_refs=["rule:django-only"])
        validate_bundle(x)
        x["coverages"][0]["claim"] = "SCANNED_APPLICABLE"
        x["coverages"][0]["rule_ids"] = ["py/path-injection"]
        with self.assertRaises(ValueError):
            validate_bundle(x)

    def test_report_and_ingest_must_match_scanner(self):
        self.assert_invalid(lambda x: x["scan"]["scanner"].update(engine="semgrep"))
        self.assert_invalid(lambda x: x["scan"]["ingest"].update(run_id="0" * 64))
        self.assert_invalid(lambda x: x["scan"]["ingest"].update(independently_replayed=False))
        self.assert_invalid(lambda x: x["scan"]["report"].update(sha256=""))

    def test_refuse_cross_record_foreign_keys_or_duplicates(self):
        self.assert_invalid(lambda x: x["coverages"][0].update(scan_id="0" * 64))
        self.assert_invalid(lambda x: x["candidates"][0].update(coverage_id="0" * 64))
        self.assert_invalid(lambda x: x["candidates"].append(copy.deepcopy(x["candidates"][0])))
        self.assert_invalid(lambda x: x["coverages"].append({
            **copy.deepcopy(x["coverages"][0]), "coverage_id": "4" * 64,
        }))

    def test_unscanned_source_scope_cannot_claim_applicability(self):
        self.assert_invalid(lambda x: (x["coverages"][0]["target"].update(source_scope="other"),
                                       x["scan"]["scope"].update(include_paths=["starlette"])))
        self.assert_invalid(lambda x: x["scan"]["scope"].update(include_paths=["src"]))


if __name__ == "__main__":
    unittest.main()
