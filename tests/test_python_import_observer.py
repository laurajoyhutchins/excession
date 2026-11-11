"""End-to-end positive and negative tests for opt-in Python import witnesses."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools/python_import_observer.py"


def call(*args: object, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run([str(a) for a in args], cwd=cwd,
                          text=True, capture_output=True, check=False, timeout=30)


class PythonImportWitnesses(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="excession-observe-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.repo = self.base / "source"
        self.repo.mkdir()
        self.assertEqual(call("git", "init", "-q", self.repo).returncode, 0)
        package = self.repo / "samplepkg"
        package.mkdir()
        (package / "__init__.py").write_text("", encoding="utf-8")
        (package / "plugin.py").write_text("PLUGIN_VALUE = 37\n", encoding="utf-8")
        (self.repo / "exercise.py").write_text(
            "import importlib\n"
            "import samplepkg.plugin\n"
            "mod = importlib.import_module('samplepkg.plugin')\n"
            "assert mod.PLUGIN_VALUE == 37\n"
            "try:\n"
            "    import not_an_installed_module_excession_probe\n"
            "except ImportError:\n"
            "    pass\n",
            encoding="utf-8",
        )
        self.assertEqual(call("git", "-C", self.repo, "add", ".").returncode, 0)
        commit = call("git", "-C", self.repo, "-c", "user.name=Fixture",
                      "-c", "user.email=fixture@example.invalid",
                      "commit", "-qm", "exercise")
        self.assertEqual(commit.returncode, 0, commit.stderr)
        self.commit = call("git", "-C", self.repo, "rev-parse", "HEAD").stdout.strip()
        self.report = self.base / "run.json"

    def observe(self, *extra: str) -> subprocess.CompletedProcess[str]:
        return call(sys.executable, TOOL, "--repo", self.repo, "--rev", self.commit,
                    "--script", "exercise.py", "--out", self.report,
                    *extra)

    def test_dynamic_and_cached_imports_have_positive_witnesses(self):
        result = self.observe("--trusted-local")
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(self.report.read_text())
        self.assertEqual(payload["schema"], "excession.python-runtime-witness.v1")
        self.assertEqual(payload["claim"], "OBSERVED_RUNTIME_ONLY")
        self.assertEqual(payload["source_commit"], self.commit)
        self.assertEqual(payload["execution"]["exit_code"], 0)
        selected = [r for r in payload["witnesses"]
                    if r["module"] == "samplepkg.plugin"
                    and r["source"]["status"] == "GIT_BOUND"]
        self.assertTrue(selected)
        self.assertTrue(any(r["mechanism"] == "importlib.import_module"
                            for r in selected))
        self.assertTrue(any(r["mechanism"] == "builtins.__import__"
                            for r in selected))
        self.assertEqual(selected[0]["source"]["path"], "samplepkg/plugin.py")
        self.assertNotIn("not_an_installed_module_excession_probe",
                         {r["module"] for r in payload["witnesses"]})
        self.assertIn("not_an_installed_module_excession_probe",
                      {r["requested"] for r in payload["failed_import_attempts"]})
        self.assertIn("COVERAGE_UNPROVEN", {gap["kind"] for gap in payload["gaps"]})
        checksum = payload.pop("receipt_sha256")
        canonical = json.dumps(payload, ensure_ascii=True, sort_keys=True,
                               separators=(",", ":")).encode()
        self.assertEqual(checksum, hashlib.sha256(canonical).hexdigest())

    def test_requires_consent_without_running_script(self):
        result = self.observe()
        self.assertEqual(result.returncode, 2)
        self.assertIn("trusted-local", result.stderr)
        self.assertFalse(self.report.exists())

    def test_dirty_tracked_checkout_refuses(self):
        with (self.repo / "exercise.py").open("a") as out:
            out.write("\nraise RuntimeError('must not execute')\n")
        result = self.observe("--trusted-local")
        self.assertEqual(result.returncode, 2)
        self.assertIn("modifications", result.stderr)
        self.assertFalse(self.report.exists())

    def test_untracked_import_target_is_not_git_bound(self):
        (self.repo / "exercise.py").write_text(
            "import importlib\n"
            "mod = importlib.import_module('scratch_plugin')\n"
            "assert mod.VALUE == 1\n",
            encoding="utf-8",
        )
        self.assertEqual(call("git", "-C", self.repo, "add", "exercise.py").returncode, 0)
        self.assertEqual(call("git", "-C", self.repo, "-c", "user.name=Fixture",
                              "-c", "user.email=fixture@example.invalid",
                              "commit", "-qm", "new script").returncode, 0)
        self.commit = call("git", "-C", self.repo, "rev-parse", "HEAD").stdout.strip()
        (self.repo / "scratch_plugin.py").write_text("VALUE=1\n", encoding="utf-8")
        result = self.observe("--trusted-local")
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(self.report.read_text())
        records = [r for r in payload["witnesses"] if r["module"] == "scratch_plugin"]
        self.assertTrue(records)
        self.assertEqual(records[0]["source"]["status"], "UNTRACKED_REPO_PATH")

    def test_runtime_failure_keeps_receipt_and_nonzero_exit(self):
        (self.repo / "exercise.py").write_text(
            "import samplepkg.plugin\n"
            "raise RuntimeError('failure witness')\n",
            encoding="utf-8",
        )
        self.assertEqual(call("git", "-C", self.repo, "add", "exercise.py").returncode, 0)
        self.assertEqual(call("git", "-C", self.repo, "-c", "user.name=Fixture",
                              "-c", "user.email=fixture@example.invalid",
                              "commit", "-qm", "failing script").returncode, 0)
        self.commit = call("git", "-C", self.repo, "rev-parse", "HEAD").stdout.strip()
        result = self.observe("--trusted-local")
        self.assertEqual(result.returncode, 1, result.stderr)
        payload = json.loads(self.report.read_text())
        self.assertEqual(payload["execution"]["failure"]["type"], "RuntimeError")
        self.assertTrue(any(r["module"] == "samplepkg.plugin"
                            for r in payload["witnesses"]))

    def test_refuses_output_inside_repository(self):
        self.report = self.repo / "output.json"
        result = self.observe("--trusted-local")
        self.assertEqual(result.returncode, 2)
        self.assertIn("outside", result.stderr)
        self.assertFalse(self.report.exists())


if __name__ == "__main__":
    unittest.main()
