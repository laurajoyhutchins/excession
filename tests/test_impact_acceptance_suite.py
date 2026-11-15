"""Independent tests for the pinned multi-repository impact acceptance harness."""
from __future__ import annotations

from dataclasses import asdict
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks" / "experiments"))

from impact_acceptance_suite import Case, assess, validate_manifest, _paths, _write_json


BASE = "a" * 40
HEAD = "b" * 40


def manifest():
    return {
        "schema": "excession.impact-acceptance-cases.v1",
        "cases": [
            {"id": "sample-repo-a", "repository": "org/a", "base": BASE,
             "head": HEAD, "roots": ["pkg"], "expected_changed_paths": ["pkg/change.py"],
             "must_include_head_paths": ["pkg/importer.py"], "minimum_head_affected_paths": 1},
            {"id": "sample-repo-b", "repository": "org/b", "base": BASE,
             "head": HEAD, "roots": ["src"], "expected_changed_paths": ["src/change.py"],
             "must_include_head_paths": ["src/importer.py"], "minimum_head_affected_paths": 1},
        ],
    }


def evidence(case):
    affected = "pkg/importer.py"
    return {
        "schema": "excession.heldout-impact-evaluation.v1",
        "status": "MATCH",
        "upstream_repository": case.repository,
        "roots": list(case.roots),
        "base_commit": case.base,
        "head_commit": case.head,
        "changed_paths": ["pkg/change.py"],
        "invalid_hops": [],
        "oracle": {case.head: {"expected_affected_paths": [affected],
                              "unresolved_import_sites": 1}},
        "compact_source_impacts": {
            "complete_in_observed_graph": True,
            "actual": {case.head: [affected], case.base: [affected]},
            "missing": {case.head: [], case.base: []},
            "unexpected": {case.head: [], case.base: []},
        },
        "candidate_source_enumeration_complete": True,
        "candidate_source_count": 2,
        "candidate_effect_count": 512,
        "legacy_explanations_truncated": True,
        "impact_coverage": {"gaps": [{"reason": "TRUNCATED"}]},
        "measured_wall_seconds": {"oracle_git_and_ast": 0.2,
                                  "excession_impact_with_indexing": 0.4},
        "impact_report_bytes": 12345,
    }


class ManifestValidationTests(unittest.TestCase):
    def test_accepts_two_distinct_pinned_repositories(self):
        cases = validate_manifest(manifest())
        self.assertEqual([case.repository for case in cases], ["org/a", "org/b"])
        self.assertEqual(cases[0].expected_changed_paths, ("pkg/change.py",))
        self.assertEqual(cases[0].must_include_head_paths, ("pkg/importer.py",))

    def test_refuses_one_repository_or_duplicate_cases(self):
        data = manifest()
        data["cases"][1]["repository"] = "org/a"
        with self.assertRaisesRegex(ValueError, "two independent"):
            validate_manifest(data)
        data = manifest()
        data["cases"][1]["id"] = data["cases"][0]["id"]
        with self.assertRaisesRegex(ValueError, "duplicate"):
            validate_manifest(data)

    def test_refuses_ambiguous_or_unsafe_git_identifiers(self):
        for changed in [
            {"repository": "https://evil.invalid/repo"},
            {"repository": "org/../repo"},
            {"repository": "org/repo;echo"},
            {"head": "HEAD"},
            {"base": HEAD},
            {"id": "../escape"},
            {"roots": ["pkg/../outside"]},
            {"expected_changed_paths": ["pkg/../../outside.py"]},
            {"must_include_head_paths": ["other/impl.py"]},
            {"minimum_head_affected_paths": True},
            {"minimum_head_affected_paths": 0},
        ]:
            with self.subTest(changed=changed):
                data = manifest()
                data["cases"][0].update(changed)
                with self.assertRaises(ValueError):
                    validate_manifest(data)

    def test_refuses_unsupported_or_extra_case_fields(self):
        data = manifest()
        data["cases"][0]["source_url"] = "https://example.invalid"
        with self.assertRaisesRegex(ValueError, "fields"):
            validate_manifest(data)
        data = manifest()
        data["schema"] = "wrong.v1"
        with self.assertRaisesRegex(ValueError, "schema"):
            validate_manifest(data)

    def test_paths_are_sorted_for_deterministic_pinning(self):
        self.assertEqual(_paths(["z/z.py", "a/a.py"], "paths", allow_empty=False),
                         ("a/a.py", "z/z.py"))


class IndependentAssessmentTests(unittest.TestCase):
    def setUp(self):
        self.case = validate_manifest(manifest())[0]
        self.result = evidence(self.case)
        self.edges = {("pkg/importer.py", "pkg/change.py")}
        self.files = {"pkg/change.py", "pkg/importer.py"}

    def score(self, result=None):
        return assess(self.case, self.result if result is None else result,
                      self.edges, self.files)

    def test_bounded_explanations_do_not_exhaust_distinct_source_evidence(self):
        scored = self.score()
        self.assertEqual(scored["status"], "PASS")
        self.assertTrue(scored["explanations_truncated"])
        self.assertEqual(scored["missing_vs_independent_ast"], 0)
        self.assertEqual(scored["baselines"]["independent_full_transitive"], 1)
        self.assertEqual(scored["cost"]["excession_impact_json_bytes"], 12345)
        self.assertFalse(scored.get("safe_to_skip_tests", False))

    def test_refuses_invented_or_missing_source(self):
        for field, replacement in [
            ("expected", []),
            ("actual", []),
        ]:
            with self.subTest(field=field):
                copy = json.loads(json.dumps(self.result))
                if field == "expected":
                    copy["oracle"][HEAD]["expected_affected_paths"] = replacement
                else:
                    copy["compact_source_impacts"]["actual"][HEAD] = replacement
                with self.assertRaises(ValueError):
                    self.score(copy)

    def test_refuses_incomplete_inventory_even_if_explanations_truncated(self):
        copy = json.loads(json.dumps(self.result))
        copy["candidate_source_enumeration_complete"] = False
        with self.assertRaisesRegex(ValueError, "not complete"):
            self.score(copy)

    def test_refuses_forged_evidence_and_repository_substitution(self):
        copy = json.loads(json.dumps(self.result))
        copy["invalid_hops"] = ["path has different blob"]
        with self.assertRaisesRegex(ValueError, "unverified"):
            self.score(copy)
        copy = json.loads(json.dumps(self.result))
        copy["upstream_repository"] = "different/repo"
        with self.assertRaisesRegex(ValueError, "identity"):
            self.score(copy)

    def test_refuses_swapped_revisions_and_changed_path_disagreement(self):
        copy = json.loads(json.dumps(self.result))
        copy["head_commit"] = BASE
        with self.assertRaises(ValueError):
            self.score(copy)
        copy = json.loads(json.dumps(self.result))
        copy["changed_paths"] = []
        with self.assertRaises(ValueError):
            self.score(copy)

    def test_source_bound_artifact_digest_is_reproducible(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "artifact.json"
            digest = _write_json(path, {"b": 2, "a": 1})
            self.assertEqual(digest, _write_json(path, {"a": 1, "b": 2}))
            self.assertEqual(json.loads(path.read_text()), {"a": 1, "b": 2})


if __name__ == "__main__":
    unittest.main()
