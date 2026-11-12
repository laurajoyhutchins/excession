"""Fail-closed acceptance for experimental Python observation reuse.

The experiment is read-only. Its parser remains the production CPython AST
adapter and its semantic oracle is independently derived from pinned Git blobs.
"""
from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
import incremental_python as inc  # noqa: E402


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, text=True,
        capture_output=True, timeout=20,
    ).stdout.strip()


class IncrementalPythonExperiment(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory(prefix="excession-inc-python-")
        self.addCleanup(tmp.cleanup)
        self.repo = Path(tmp.name) / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.repo)],
                       check=True, timeout=20)
        git(self.repo, "config", "user.name", "Incremental fixture")
        git(self.repo, "config", "user.email", "incremental@example.invalid")
        self.write("pkg/__init__.py", "")
        self.write("pkg/core.py", "VALUE = 1\n")
        self.write("pkg/consumer.py", "import pkg.core\n")
        self.write("pkg/unrelated.py", "VALUE = 9\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "base")
        self.base = git(self.repo, "rev-parse", "HEAD")
        self.roots = ("pkg",)

    def write(self, path: str, content: str) -> None:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")

    def commit(self, message: str) -> str:
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", message)
        return git(self.repo, "rev-parse", "HEAD")

    def base_cache(self):
        paths = inc.inventory(self.repo, self.base, self.roots)
        adapter = inc.read_adapter()
        observations, _ = inc.full_scan(self.repo, paths, adapter)
        version = inc.producer(self.roots)
        cache = inc.cache_snapshot(
            self.base, git(self.repo, "rev-parse", f"{self.base}^{{tree}}"),
            paths, observations, version,
        )
        return adapter, version, cache

    def test_reuses_unchanged_modules_and_matches_independent_oracle(self):
        self.write("pkg/core.py", "VALUE = 2\n")
        head = self.commit("change core")
        report = inc.evaluate(self.repo, self.base, head, self.roots, 2)
        self.assertEqual(report["status"], "MATCH")
        self.assertEqual(report["reparsed_modules"], 1)
        self.assertEqual(report["reused_modules"], 3)
        self.assertIsNone(report["fallback_reason"])
        self.assertEqual(report["missing_edges"], [])
        self.assertEqual(report["unexpected_edges"], [])

    def test_unrelated_non_source_change_reparses_nothing(self):
        self.write("README.md", "New prose\n")
        head = self.commit("update readme")
        report = inc.evaluate(self.repo, self.base, head, self.roots, 1)
        self.assertEqual(report["status"], "MATCH")
        self.assertEqual(report["reparsed_modules"], 0)
        self.assertEqual(report["reused_modules"], 4)

    def test_added_module_forces_full_parse(self):
        self.write("pkg/new.py", "import pkg.core\n")
        head = self.commit("add module")
        report = inc.evaluate(self.repo, self.base, head, self.roots, 1)
        self.assertEqual(report["status"], "MATCH")
        self.assertEqual(report["fallback_reason"], "MODULE_CATALOG_CHANGED")
        self.assertEqual(report["reparsed_modules"], 5)
        self.assertEqual(report["reused_modules"], 0)

    def test_producer_drift_forces_full_parse(self):
        adapter, version, cache = self.base_cache()
        self.write("pkg/core.py", "VALUE = 2\n")
        head = self.commit("edit source")
        paths = inc.inventory(self.repo, head, self.roots)
        observed, stats = inc.incremental(self.repo, paths, cache, adapter, version + "-drift")
        full, _ = inc.full_scan(self.repo, paths, adapter)
        self.assertEqual(observed, full)
        self.assertEqual(stats["fallback"], "PRODUCER_CHANGED")
        self.assertEqual(stats["reparsed"], len(paths))

    def test_cached_receipt_tamper_refused(self):
        adapter, version, cache = self.base_cache()
        forged = copy.deepcopy(cache)
        forged["observations"]["pkg/consumer.py"]["imports"] = []
        self.write("pkg/core.py", "VALUE = 2\n")
        head = self.commit("edit source")
        paths = inc.inventory(self.repo, head, self.roots)
        with self.assertRaisesRegex(ValueError, "cached observation receipt changed"):
            inc.incremental(self.repo, paths, forged, adapter, version)


if __name__ == "__main__":
    unittest.main()
