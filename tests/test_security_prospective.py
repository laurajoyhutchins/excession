"""Cheap preregistration and scoring controls, no scanner or source execution."""
from __future__ import annotations

import copy
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from compare_prospective_security import load_manifest, relevant  # noqa: E402


class ProspectiveSecurityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.path = ROOT / "benchmarks/security-prospective/manifest.v1.json"
        cls.manifest = load_manifest(cls.path)

    def test_frozen_six_distinct_advisories(self):
        cases = self.manifest["cases"]
        self.assertEqual(len(cases), 6)
        self.assertEqual(len({c["advisory"] for c in cases}), 6)
        self.assertEqual(len({c["repository"] for c in cases}), 3)
        self.assertGreaterEqual(len({c["weakness"] for c in cases}), 2)

    def mutate_invalid(self, change):
        document = copy.deepcopy(self.manifest)
        change(document)
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "manifest.json"
            import json
            p.write_text(json.dumps(document))
            with self.assertRaises((ValueError, KeyError, TypeError)):
                load_manifest(p)

    def test_reject_mutable_ref(self):
        self.mutate_invalid(lambda m: m["cases"][0]["vulnerable"].update(commit="main"))

    def test_reject_reused_case(self):
        self.mutate_invalid(lambda m: m["cases"][1].update(id=m["cases"][0]["id"]))

    def test_reject_development_example(self):
        self.mutate_invalid(lambda m: m["cases"][0].update(repository="aio-libs/aiohttp"))

    def test_reject_unchanged_source(self):
        self.mutate_invalid(lambda m: m["cases"][0]["patched"].update(
            blob=m["cases"][0]["vulnerable"]["blob"]))

    def test_reject_incorrect_scanner_version(self):
        self.mutate_invalid(lambda m: m["scanner"].update(version="1.999.0"))

    def test_only_matching_file_and_weakness_counts(self):
        c = {"weakness": "CWE-22", "file": "tornado/web.py"}
        candidates = [
            {"path": "tornado/web.py", "check_id": "a",
             "extra": {"metadata": {"cwe": ["CWE-22"]}}},
            {"path": "tornado/web.py", "check_id": "b",
             "extra": {"metadata": {"cwe": ["CWE-78"]}}},
            {"path": "tornado/else.py", "check_id": "c",
             "extra": {"metadata": {"cwe": ["CWE-22"]}}},
            {"path": "./tornado/web.py", "check_id": "d",
             "extra": {"metadata": {"cwe": ["CWE-22: Path Traversal"]}}},
        ]
        results = relevant({"results": candidates, "errors": []}, c)
        self.assertEqual(results["total"], 4)
        self.assertEqual(results["in_file"], 3)
        self.assertEqual(results["relevant"], 2)
        self.assertEqual(results["relevant_rules"], ["a", "d"])

    def test_scanner_errors_cannot_be_negative_detections(self):
        with self.assertRaises(ValueError):
            relevant({"results": [], "errors": [{"message": "timeout"}]},
                     {"weakness": "CWE-78", "file": "git/cmd.py"})


if __name__ == "__main__":
    unittest.main()
