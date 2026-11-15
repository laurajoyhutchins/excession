"""Contract tests for complete source witnesses with bounded explanations."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "benchmarks" / "experiments"))
from impact_heldout import result_matches_oracle


class ImpactEvidenceAdmissionTests(unittest.TestCase):
    def check_policy(self, **kwargs):
        options = dict(invalid=[], explanation_missing={"base": [], "head": []},
                       explanation_unexpected={"base": [], "head": []},
                       source_missing={"base": [], "head": []},
                       source_unexpected={"base": [], "head": []},
                       explanations_truncated=False)
        options.update(kwargs)
        return result_matches_oracle(**options)

    def test_complete_explanations_are_required_without_truncation(self):
        self.assertFalse(self.check_policy(explanation_missing={"head": ["test.py"]}))

    def test_truncated_explanations_can_omit_alternatives_with_complete_sources(self):
        self.assertTrue(self.check_policy(explanations_truncated=True,
            explanation_missing={"base": ["a.py"], "head": ["test.py"]}))

    def test_truncation_cannot_excuse_missing_sources_or_false_positive_paths(self):
        for field in ("source_missing", "source_unexpected", "explanation_unexpected"):
            with self.subTest(field=field):
                self.assertFalse(self.check_policy(
                    explanations_truncated=True, **{field: {"head": ["wrong.py"]}}))

    def test_truncation_cannot_excuse_forged_evidence(self):
        self.assertFalse(self.check_policy(
            explanations_truncated=True, invalid=["invalid Git blob or receipt"]))


if __name__ == "__main__":
    unittest.main()
