"""Fail-closed independent source reachability acceptance policy."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "benchmarks" / "experiments"))
from impact_heldout import result_matches_oracle


class ImpactEvidenceAdmissionTests(unittest.TestCase):
    def check_policy(self, **kwargs):
        options = dict(
            invalid=[],
            source_missing={"base": [], "head": []},
            source_unexpected={"base": [], "head": []},
            source_complete=True,
        )
        options.update(kwargs)
        return result_matches_oracle(**options)

    def test_complete_source_witnesses_are_accepted(self):
        self.assertTrue(self.check_policy())

    def test_missing_or_unexpected_sources_are_refused(self):
        for field in ("source_missing", "source_unexpected"):
            with self.subTest(field=field):
                self.assertFalse(self.check_policy(**{field: {"head": ["wrong.py"]}}))

    def test_incomplete_source_inventory_cannot_pass(self):
        self.assertFalse(self.check_policy(source_complete=False))

    def test_forged_git_or_producer_evidence_is_refused(self):
        self.assertFalse(self.check_policy(invalid=["invalid Git blob or receipt"]))


if __name__ == "__main__":
    unittest.main()
