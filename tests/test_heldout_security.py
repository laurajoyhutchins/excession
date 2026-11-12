"""Held-out scan contracts deliberately keep hits distinct from truth labels."""
import json
from pathlib import Path
import tempfile
import unittest

from tools.compare_heldout_security import (
    relative_match, semgrep_observations, codeql_observations, summarize, markdown,
)


class CompareHeldoutTests(unittest.TestCase):
    def test_exact_source_path_not_sibling(self):
        self.assertTrue(relative_match("./starlette/staticfiles.py", "starlette/staticfiles.py"))
        self.assertFalse(relative_match("starlette/staticfiles.py.bak", "starlette/staticfiles.py"))
        self.assertFalse(relative_match("other/staticfiles.py", "starlette/staticfiles.py"))

    def test_semgrep_empty_findings_is_not_error(self):
        result = semgrep_observations({"results": [], "errors": []}, "a/b.py")
        self.assertEqual(result["target_file_alerts"], 0)

    def test_semgrep_config_errors_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "errors"):
            semgrep_observations({"results": [], "errors": [{"message": "bad rule"}]}, "a.py")

    def test_codeql_only_counts_matching_source_paths(self):
        report = {"runs": [{"results": [
            {"ruleId": "py/path-injection", "locations": [
                {"physicalLocation": {"artifactLocation": {"uri": "a/b.py"}, "region": {"startLine": 12}}}]},
            {"ruleId": "other", "locations": [
                {"physicalLocation": {"artifactLocation": {"uri": "a/other.py"}}}]},
        ]}]}
        obs = codeql_observations(report, "a/b.py")
        self.assertEqual(obs["total_alerts"], 2)
        self.assertEqual(obs["target_file_alerts"], 1)
        self.assertEqual(obs["target_rules"], ["py/path-injection"])

    def test_no_incomplete_source_or_oracle(self):
        manifest = {
            "schema": "excession.security.heldout.v1",
            "claim_boundary": "no accuracy estimate",
            "cases": [{"id": "x", "file": "x.py",
                       "vulnerable": {"commit": "1", "blob": "a"},
                       "patched": {"commit": "2", "blob": "b"}}],
        }
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp) / "heldout-x-vulnerable"
            p.mkdir()
            (p / "metadata.json").write_text(json.dumps({
                "case": "x", "phase": "vulnerable",
                "source_commit": "1", "source_blob": "incorrect",
            }))
            with self.assertRaisesRegex(ValueError, "provenance mismatch"):
                summarize(manifest, Path(temp))

    def test_markdown_reports_both_without_accuracy_claim(self):
        out = markdown({"comparisons": [{
            "case": "x", "engine": "codeql",
            "vulnerable_target_file_alerts": 1, "patched_target_file_alerts": 1
        }]})
        self.assertIn("| x | codeql | 1 | 1 |", out)
        self.assertIn("not automatically a false positive", out)


if __name__ == "__main__":
    unittest.main()
