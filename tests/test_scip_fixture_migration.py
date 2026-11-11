"""Contract checks for the Python-only upstream SCIP migration harness."""
from __future__ import annotations

import sys
import json
import tempfile
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
from scip_fixture_migration import (  # noqa: E402
    cases, coverage, independently_replayed_occurrences,
    typescript_fixture_config,
)


def symbols(*, occurrences: int = 2, unresolved: int = 0) -> dict:
    return {
        "indexed_occurrences": occurrences,
        "unique_defined_symbols": 1,
        "matched_cross_file_references": 0,
        "unresolved_references": unresolved,
        "line_only_position_documents": 0,
    }


class ScipEvidenceContract(unittest.TestCase):
    def test_no_legacy_parity_requirement(self) -> None:
        result = coverage(
            {"src/a.py", "src/b.py"},
            {"src/a.py", "src/b.py"}, symbols(unresolved=9),
        )
        self.assertEqual(result["status"], "SOURCE_BOUND")
        self.assertEqual(result["unresolved_symbol_references"], 9)
        self.assertEqual(result["matched_cross_file_symbol_references"], 0)
        self.assertFalse(result["semantic_correctness_independently_verified"])
        self.assertFalse(result["indexer_run_attested"])

    def test_missing_document_is_partial_not_invented(self) -> None:
        result = coverage(
            {"src/a.py", "src/b.py"}, {"src/a.py"}, symbols(),
        )
        self.assertEqual(result["status"], "PARTIAL")
        self.assertEqual(result["missing_source_files"], ["src/b.py"])
        self.assertEqual(result["missing_source_file_count"], 1)

    def test_empty_index_and_empty_scope_are_unsupported(self) -> None:
        self.assertEqual(
            coverage({"src/app.py"}, set(), symbols(occurrences=0))["status"],
            "UNSUPPORTED",
        )
        self.assertEqual(
            coverage(set(), {"README.md"}, symbols())["status"],
            "UNSUPPORTED",
        )

    def test_no_occurrences_is_partial_with_explicit_coverage(self) -> None:
        report = coverage({"src/a.py"}, {"src/a.py"},
                          symbols(occurrences=0))
        self.assertEqual(report["status"], "PARTIAL")
        self.assertEqual(report["missing_source_file_count"], 0)

    def test_extra_documents_are_described_not_automatically_rejected(self) -> None:
        report = coverage(
            {"src/a.py"}, {"src/a.py", "tests/test_a.py"}, symbols(),
        )
        self.assertEqual(report["status"], "SOURCE_BOUND")
        self.assertEqual(report["additional_source_files"], ["tests/test_a.py"])

    def test_empty_indexer_outputs_cannot_make_green_smoke(self) -> None:
        self.assertEqual(independently_replayed_occurrences([]), 0)
        self.assertEqual(independently_replayed_occurrences([
            {"status": "UNSUPPORTED", "languages": []},
            {"status": "PARTIAL", "languages": [
                {"status": "PARTIAL",
                 "coverage": {"indexed_occurrences": 0}},
            ]},
        ]), 0)
        self.assertEqual(independently_replayed_occurrences([
            {"status": "PARTIAL", "languages": [
                {"status": "PARTIAL",
                 "coverage": {"indexed_occurrences": 7}},
            ]},
        ]), 7)

    def test_fixture_scoped_js_config_preserves_original_project(self) -> None:
        with tempfile.TemporaryDirectory() as dirname:
            stage = Path(dirname)
            original = '{"compilerOptions":{"strict":true,"noEmit":true}}\n'
            (stage / "tsconfig.json").write_text(original)
            paths = {"lib/core.js", "lib/alt/index.cjs", "lib/main.ts"}
            overlay = typescript_fixture_config(stage, paths)
            data = json.loads(overlay.read_text())
            self.assertEqual(data["extends"], "./tsconfig.json")
            self.assertEqual(data["files"], sorted(paths))
            self.assertTrue(data["compilerOptions"]["allowJs"])
            self.assertFalse(data["compilerOptions"]["checkJs"])
            self.assertEqual(data["include"], [])
            self.assertEqual((stage / "tsconfig.json").read_text(), original)
            with self.assertRaisesRegex(ValueError, "overwrite"):
                typescript_fixture_config(stage, paths)

    def test_fixture_scoped_js_config_without_tsconfig(self) -> None:
        with tempfile.TemporaryDirectory() as dirname:
            stage = Path(dirname)
            overlay = typescript_fixture_config(
                stage, {"src/app.js", "lib/util.cjs"},
            )
            data = json.loads(overlay.read_text())
            self.assertNotIn("extends", data)
            self.assertEqual(data["files"], ["lib/util.cjs", "src/app.js"])
            with self.assertRaisesRegex(ValueError, "empty"):
                typescript_fixture_config(stage / "elsewhere", set())

    def test_immutable_pinned_fixture_selection(self) -> None:
        manifest = ROOT / "benchmarks/fixtures/real-repositories.json"
        standard = cases(manifest, False, set())
        self.assertEqual(
            {case["name"] for case in standard},
            {"h11", "itsdangerous", "click", "requests",
             "axios", "import-cases"},
        )
        self.assertEqual(len(cases(manifest, True, set())), 22)
        old_postcss = cases(manifest, True, {"postcss"})
        self.assertEqual(len(old_postcss), 4)
        self.assertTrue(all(case["source"] == "historical"
                            for case in old_postcss))
        self.assertTrue(all("lib" in case["roots"].split(",")
                            for case in old_postcss))
        with self.assertRaisesRegex(ValueError, "unknown fixture"):
            cases(manifest, False, {"invented-project"})


if __name__ == "__main__":
    unittest.main()
