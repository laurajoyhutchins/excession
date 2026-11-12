"""Contract tests are deliberately pure, fast and independent of scanner execution."""
import copy
import json
from pathlib import Path
import unittest

try:
    import jsonschema
except ImportError:
    jsonschema = None


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(jsonschema is not None, "jsonschema dependency not installed")
class CandidateSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((ROOT / "contracts/security-candidate.schema.json").read_text())
        cls.fixture = json.loads((ROOT / "tests/fixtures/security-candidate.json").read_text())
        jsonschema.Draft202012Validator.check_schema(cls.schema)
        cls.validator = jsonschema.Draft202012Validator(cls.schema)

    def check_invalid(self, change):
        record = copy.deepcopy(self.fixture)
        change(record)
        self.assertTrue(list(self.validator.iter_errors(record)))

    def test_valid_candidate(self):
        self.assertFalse(list(self.validator.iter_errors(self.fixture)))

    def test_no_absolute_path(self):
        self.check_invalid(lambda r: r["source"].update(path="/etc/passwd"))

    def test_no_parent_traversal(self):
        self.check_invalid(lambda r: r["source"].update(path="src/../secret.py"))

    def test_reject_unpinned_commit(self):
        self.check_invalid(lambda r: r["source"].update(commit="main"))

    def test_reject_missing_report_digest(self):
        self.check_invalid(lambda r: r["scanner"].update(report_sha256=""))

    def test_reject_unknown_rule(self):
        self.check_invalid(lambda r: r["scanner"].update(rule_id=""))

    def test_no_candidate_without_applicability(self):
        self.check_invalid(lambda r: r["applicability"].update(status="UNKNOWN"))

    def test_no_candidate_without_evidence(self):
        self.check_invalid(lambda r: r["observation"].update(evidence=[]))

    def test_no_reproduction_without_candidate(self):
        self.check_invalid(lambda r: (r["observation"].update(status="NO_CANDIDATE"),
                                      r.update(disposition="REPRODUCED")))

    def test_unknown_framework_stays_inconclusive(self):
        r = copy.deepcopy(self.fixture)
        r["applicability"]["status"] = "UNKNOWN"
        r["observation"]["status"] = "INCONCLUSIVE"
        r["disposition"] = "INCONCLUSIVE"
        self.assertFalse(list(self.validator.iter_errors(r)))

    def test_reject_unrecognized_fields(self):
        self.check_invalid(lambda r: r.update(is_safe=True))


if __name__ == "__main__":
    unittest.main()
