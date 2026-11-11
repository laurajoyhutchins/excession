"""Adversarial tests for bounded executable slices and evidence carry."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import os
from unittest.mock import patch

MODULE = Path(__file__).resolve().parents[1] / "tools" / "excession_slice.py"
spec = importlib.util.spec_from_file_location("excession_slice", MODULE)
assert spec and spec.loader
slices = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = slices
spec.loader.exec_module(slices)


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


class SliceIntegrationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="excession-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "project"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.email", "test@example.invalid")
        git(self.repo, "config", "user.name", "Test")
        self.write("pkg/__init__.py", "")
        self.write("pkg/core.py", "def value():\n    return 7\n")
        self.write("tests/__init__.py", "")
        self.write("tests/check.py", "from pkg.core import value\nassert value() == 7\nprint('slice-ok')\n")
        self.write("unrelated.py", "print('excluded')\n")
        self.commit("base")
        self.base = git(self.repo, "rev-parse", "HEAD")

    def write(self, path: str, contents: str) -> None:
        p = self.repo / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(contents)

    def commit(self, msg: str) -> None:
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", msg)

    def base_run(self):
        manifest = slices.plan(self.repo, "tests/check.py")
        out = self.root / "slice"
        slices.materialize(self.repo, manifest, out)
        receipt = slices.execute(self.repo, manifest, out, 10)
        self.assertEqual(receipt["status"], "OBSERVED_PASS", receipt)
        return manifest, receipt, out

    def test_slice_replays_only_closed_sources(self):
        manifest, receipt, out = self.base_run()
        self.assertEqual({x["path"] for x in manifest["entries"]},
                         {"pkg/__init__.py", "pkg/core.py", "tests/__init__.py", "tests/check.py"})
        self.assertEqual(manifest["scope"], "STATIC_IMPORT_CLOSURE_ONLY")
        self.assertTrue(receipt["receipt_id"])
        self.assertFalse((out / "unrelated.py").exists())
        slices.audit(self.repo, manifest, out)

    def test_independent_branch_merge_carries_only_source_match(self):
        manifest, receipt, _ = self.base_run()
        git(self.repo, "switch", "-qc", "agent-a")
        self.write("docs/notes.md", "independent documentation\n")
        self.commit("agent A")
        git(self.repo, "switch", "-qc", "agent-b", self.base)
        self.write("unrelated.py", "print('branch B')\n")
        self.commit("agent B")
        git(self.repo, "merge", "-q", "--no-edit", "agent-a")
        result = slices.carry(self.repo, manifest, receipt)
        self.assertEqual(result["decision"], "SOURCE_MATCH_ONLY")
        self.assertFalse(result["target_behavior_verified"])
        self.assertEqual(result["changed_closure_paths"], [])

    def test_changed_dependency_requires_reverification_even_if_test_still_passes(self):
        manifest, receipt, _ = self.base_run()
        self.write("pkg/core.py", "def value(): return 7\n")
        self.commit("source changed")
        result = slices.carry(self.repo, manifest, receipt)
        self.assertEqual(result["decision"], "REVERIFY_REQUIRED")
        self.assertEqual(result["changed_closure_paths"], ["pkg/core.py"])
        target = slices.plan(self.repo, "tests/check.py")
        out = self.root / "new-slice"
        slices.materialize(self.repo, target, out)
        self.assertEqual(slices.execute(self.repo, target, out, 10)["status"], "OBSERVED_PASS")

    def test_changed_behavior_fails_target_execution(self):
        manifest, receipt, _ = self.base_run()
        self.write("pkg/core.py", "def value(): return 8\n")
        self.commit("regression")
        result = slices.carry(self.repo, manifest, receipt)
        self.assertEqual(result["decision"], "REVERIFY_REQUIRED")
        target = slices.plan(self.repo, "tests/check.py")
        out = self.root / "regressed-slice"
        slices.materialize(self.repo, target, out)
        self.assertEqual(slices.execute(self.repo, target, out, 10)["status"], "OBSERVED_FAIL")

    def test_added_shadow_import_invalidates_closure(self):
        self.write("tests/check.py", "from pkg import optional\nassert optional.value() == 7\n")
        self.commit("unresolved")
        with self.assertRaises(slices.Refusal):
            slices.plan(self.repo, "tests/check.py")

    def test_tampering_and_extra_files_refused(self):
        manifest, _, out = self.base_run()
        p = out / "pkg/core.py"
        p.chmod(0o644)
        p.write_text("def value(): return 8\n")
        with self.assertRaises(slices.Refusal):
            slices.audit(self.repo, manifest, out)
        p.write_text("def value():\n    return 7\n")
        p.chmod(0o444)
        (out / "extra.py").write_text("bad")
        with self.assertRaises(slices.Refusal):
            slices.audit(self.repo, manifest, out)

    def test_dynamic_load_refused(self):
        self.write("tests/check.py", "__import__('pkg.core')\n")
        self.commit("dynamic")
        with self.assertRaisesRegex(slices.Refusal, "dynamic"):
            slices.plan(self.repo, "tests/check.py")

    def test_external_dependency_refused(self):
        self.write("tests/check.py", "import definitely_external_1234\n")
        self.commit("external")
        with self.assertRaisesRegex(slices.Refusal, "external"):
            slices.plan(self.repo, "tests/check.py")

    def test_untrusted_receipt_rejected(self):
        manifest, receipt, _ = self.base_run()
        receipt["status"] = "OBSERVED_FAIL"
        with self.assertRaises(slices.Refusal):
            slices.carry(self.repo, manifest, receipt)

    def test_dirty_and_symlink_input_refused(self):
        self.write("untracked.py", "x=1\n")
        with self.assertRaises(slices.Refusal):
            slices.plan(self.repo, "tests/check.py")
        (self.repo / "untracked.py").unlink()
        (self.repo / "alias.py").symlink_to("pkg/core.py")
        self.commit("linked input")
        with self.assertRaises(slices.Refusal):
            slices.plan(self.repo, "alias.py")

    def test_execution_that_modifies_slice_is_refused(self):
        self.write("tests/check.py", "from pathlib import Path\nPath('unexpected.txt').write_text('new file')\n")
        self.commit("mutating test")
        manifest = slices.plan(self.repo, "tests/check.py")
        out = self.root / "mutating-slice"
        slices.materialize(self.repo, manifest, out)
        with self.assertRaises(slices.Refusal):
            slices.execute(self.repo, manifest, out, 10)

    def test_isolated_worker_setup_failure_is_not_test_failure(self):
        manifest, _, out = self.base_run()
        # This mock exercises compatibility-layer refusal propagation. The
        # optional namespace integration test exercises actual Rust setup.
        with patch.object(slices.subprocess, "run", return_value=subprocess.CompletedProcess(
                ["excession", "slice", "run"], 2, b"",
                b"REFUSED: isolated worker setup did not establish execution boundary")):
            with self.assertRaisesRegex(slices.Refusal, "execution boundary"):
                slices.execute(self.repo, manifest, out, 2, isolated_linux=True)

    @unittest.skipUnless(os.environ.get("EXCESSION_RUN_NAMESPACE_TESTS") == "1",
                         "opt-in Linux namespace integration")
    def test_linux_namespace_limits_filesystem_access(self):
        self.write("tests/check.py", "from pathlib import Path\n"
                   "from pkg.core import value\nassert value() == 7\n"
                   "assert not Path('/etc/passwd').exists()\n"
                   "assert Path('/work/pkg/core.py').exists()\n"
                   "assert not Path('/work/unrelated.py').exists()\n")
        self.commit("isolation probe")
        manifest = slices.plan(self.repo, "tests/check.py")
        out = self.root / "namespace"
        slices.materialize(self.repo, manifest, out)
        result = slices.execute(self.repo, manifest, out, 10, isolated_linux=True)
        self.assertEqual(result["status"], "OBSERVED_PASS", result)
        self.assertEqual(result["isolation"], "LOCAL_LINUX_NAMESPACES_UNATTESTED")
        slices.audit(self.repo, manifest, out)
        self.write("tests/check.py", "from pathlib import Path\nPath('/work/escape.txt').write_text('bad')\n")
        self.commit("forbidden write")
        second = slices.plan(self.repo, "tests/check.py")
        second_out = self.root / "namespace2"
        slices.materialize(self.repo, second, second_out)
        denied = slices.execute(self.repo, second, second_out, 10, isolated_linux=True)
        self.assertEqual(denied["status"], "OBSERVED_FAIL")
        self.assertFalse((second_out / "escape.txt").exists())
        slices.audit(self.repo, second, second_out)

    def test_forged_manifest_rejected(self):
        manifest, _, _ = self.base_run()
        manifest["entries"].pop()
        with self.assertRaises(slices.Refusal):
            slices.validate_manifest(self.repo, manifest)


if __name__ == "__main__":
    unittest.main()
