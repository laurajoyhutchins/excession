"""Independent Git-fixture tests for the cumulative CI quality ratchet."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "ci" / "quality_ratchet.py"
spec = importlib.util.spec_from_file_location("quality_ratchet", SCRIPT)
ratchet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ratchet)


def run(repo: Path, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


class QualityRatchet(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        run(self.repo, "init", "-q")
        run(self.repo, "config", "user.email", "test@example.invalid")
        run(self.repo, "config", "user.name", "Test")
        (self.repo / "quality").mkdir()
        self.rule = {"id": "no-shell", "paths": ["tools/*.py"],
                     "pattern": r"shell\s*=\s*True", "rationale": "Shell injection is avoidable",
                     "evidence": "docs/CI.md"}
        self.save_catalog([self.rule])
        self.put("tools/runner.py", "runner(shell=True)\n")  # grandfathered
        self.base = self.commit()

    def put(self, path, text):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def save_catalog(self, rules):
        self.put("quality/ratchet.json", json.dumps({"schema": "excession.quality-ratchet.v1", "rules": rules}))

    def commit(self):
        run(self.repo, "add", "-A")
        run(self.repo, "commit", "-qm", "fixture")
        return run(self.repo, "rev-parse", "HEAD")

    def check(self):
        return ratchet.check(self.repo, self.base, run(self.repo, "rev-parse", "HEAD"))

    def test_unchanged_grandfathered_debt_passes(self):
        self.put("tools/runner.py", "# changed unrelated comment\nrunner(shell=True)\n")
        self.commit()
        self.assertEqual(self.check()["status"], "PASS")

    def test_new_violation_and_duplicate_detected(self):
        self.put("tools/runner.py", "runner(shell=True)\nrunner(shell=True)\n")
        self.commit()
        report = self.check()
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(len(report["new_violations"]), 1)

    def test_safe_removal_passes(self):
        self.put("tools/runner.py", "runner(shell=False)\n")
        self.commit()
        self.assertEqual(self.check()["status"], "PASS")

    def test_new_rules_use_old_source_as_baseline(self):
        added = {**self.rule, "id": "another-shell-rule"}
        self.save_catalog([self.rule, added])
        self.put("tools/runner.py", "# changed\nrunner(shell=True)\n")
        self.commit()
        self.assertEqual(self.check()["status"], "PASS")

    def test_deleted_rule_is_refused(self):
        self.save_catalog([])
        self.commit()
        self.assertEqual(self.check()["altered_or_removed_rules"], ["no-shell"])

    def test_altered_rule_is_refused(self):
        self.save_catalog([{**self.rule, "pattern": "never matches"}])
        self.commit()
        self.assertEqual(self.check()["status"], "FAIL")

    def test_renamed_unchanged_file_does_not_reintroduce_violation(self):
        run(self.repo, "mv", "tools/runner.py", "tools/renamed.py")
        self.commit()
        self.assertEqual(self.check()["status"], "PASS")

    def test_new_file_is_scanned(self):
        self.put("tools/created.py", "run(shell=True)\n")
        self.commit()
        self.assertEqual(self.check()["status"], "FAIL")

    def test_invalid_catalog_refuses_instead_of_defaulting_to_pass(self):
        self.save_catalog([{**self.rule, "pattern": "("}])
        self.commit()
        with self.assertRaises(ratchet.Refusal):
            self.check()

    def test_unrelated_file_cannot_trigger_violation(self):
        self.put("docs/notes.md", "shell=True\n")
        self.commit()
        self.assertEqual(self.check()["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
