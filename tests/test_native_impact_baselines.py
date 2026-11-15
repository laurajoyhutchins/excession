"""Offline contract checks for independent, real-repository native benchmark."""
from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "benchmarks/experiments/native_impact_baselines.py"
spec = importlib.util.spec_from_file_location("native_impact_baselines", SCRIPT)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)

MANIFEST = Path(__file__).resolve().parents[1] / "benchmarks/acceptance/native-impact-cases.json"


class NativeImpactBenchmarkContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = json.loads(MANIFEST.read_text())

    def test_three_independently_maintained_source_pins(self) -> None:
        cases = bench.validate_cases(self.manifest)
        self.assertEqual({c["language"] for c in cases}, {"java", "go", "c"})
        self.assertEqual(len({c["repository"] for c in cases}), 3)
        self.assertTrue(all(len(c["commit"]) == 40 for c in cases))

    def test_refuses_unreviewed_repos_and_mutable_references(self) -> None:
        invalid = json.loads(json.dumps(self.manifest))
        invalid["cases"][0]["commit"] = "main"
        with self.assertRaisesRegex(ValueError, "reviewed upstream"):
            bench.validate_cases(invalid)
        invalid = json.loads(json.dumps(self.manifest))
        invalid["cases"][2]["repository"] = "laurajoyhutchins/excession"
        with self.assertRaisesRegex(ValueError, "reviewed upstream"):
            bench.validate_cases(invalid)
        invalid = json.loads(json.dumps(self.manifest))
        invalid["cases"][0]["changed_path"] = "../outside.java"
        with self.assertRaisesRegex(ValueError, "unsafe"):
            bench.validate_cases(invalid)

    def test_refuses_incomplete_or_duplicated_language_suite(self) -> None:
        bad = json.loads(json.dumps(self.manifest))
        bad["cases"] = bad["cases"][:2]
        with self.assertRaisesRegex(ValueError, "all three"):
            bench.validate_cases(bad)
        bad = json.loads(json.dumps(self.manifest))
        bad["cases"][1] = bad["cases"][0].copy()
        with self.assertRaisesRegex(ValueError, "duplicate"):
            bench.validate_cases(bad)

    def test_precision_recall_keeps_all_missing_and_extra_paths(self) -> None:
        result = bench.precision_recall({"a", "b"}, {"a", "c"})
        self.assertEqual(result["precision"], 0.5)
        self.assertEqual(result["recall"], 0.5)
        self.assertEqual(result["missing"], ["c"])
        self.assertEqual(result["extra"], ["b"])
        self.assertIsNone(bench.precision_recall(set(), {"x"})["precision"])
        self.assertIsNone(bench.precision_recall({"x"}, set())["recall"])

    def test_refuses_empty_or_wrong_schema(self) -> None:
        with self.assertRaises(ValueError):
            bench.validate_cases({"schema": "wrong", "cases": self.manifest["cases"]})
        with tempfile.TemporaryDirectory() as temp:
            self.assertFalse(Path(temp, "checkout").exists())


if __name__ == "__main__":
    unittest.main()
