"""Real Git-fixture tests for the explicitly trusted structural preservation runner."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SRC = Path(__file__).resolve().parents[1] / "tools" / "ci" / "structural_preservation.py"
spec = importlib.util.spec_from_file_location("structural_preservation", SRC)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def git(repo, *args):
    return subprocess.check_output(("git", "-C", str(repo), *args), text=True).strip()


class StructuralPreservation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Fixture")
        git(self.repo, "config", "user.email", "fixture@example.invalid")
        self.write("module.py", "def answer():\n    return 6 * 7\n")
        self.base = self.commit()
        self.write("module.py", "def answer():\n    return 40 + 2\n")
        self.candidate = self.commit()
        self.doc = {
            "schema": runner.SCHEMA,
            "changed_paths": ["module.py"],
            "obligations": [
                {"id": "observable-output", "kind": "differential",
                 "cwd": ".", "argv": ["python3", "-B", "-c",
                    "import runpy; print(runpy.run_path('module.py')['answer']())"],
                 "timeout_seconds": 10, "max_output_bytes": 1024},
                {"id": "candidate-regression", "kind": "candidate_pass",
                 "cwd": ".", "argv": ["python3", "-B", "-c",
                    "import runpy; assert runpy.run_path('module.py')['answer']() == 42"],
                 "timeout_seconds": 10, "max_output_bytes": 1024},
            ],
        }

    def write(self, name, content):
        file = self.repo / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content, encoding="utf-8")

    def commit(self):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "fixture")
        return git(self.repo, "rev-parse", "HEAD")

    def evaluate(self, base=None, candidate=None):
        return runner.evaluate(self.repo, base or self.base, candidate or self.candidate,
                               self.doc, "contract-digest")

    def test_equivalent_observations_are_scoped_not_merge_approval(self):
        report = self.evaluate()
        self.assertEqual(report["status"], "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE")
        self.assertFalse(report["ready_to_merge"])
        self.assertFalse(report["safe_to_skip_unlisted_tests"])
        self.assertEqual(report["semantic_equivalence"], "NOT_PROVED")
        self.assertEqual([o["result"] for o in report["obligations"]], ["PASS", "PASS"])
        self.assertEqual(report["changed_paths"], ["module.py"])
        self.assertEqual(report["candidate_tree"], git(self.repo, "rev-parse", self.candidate + "^{tree}"))
        self.assertEqual(len(git(self.repo, "worktree", "list").splitlines()), 1)

    def test_changed_behavior_is_a_counterexample(self):
        self.write("module.py", "def answer():\n    return 43\n")
        bad = self.commit()
        report = self.evaluate(candidate=bad)
        self.assertEqual(report["status"], "FAILED")
        self.assertEqual(report["obligations"][0]["result"], "FAIL")
        self.assertEqual(report["obligations"][1]["result"], "FAIL")

    def test_missing_test_tool_is_inconclusive_not_green(self):
        self.doc["obligations"][1]["argv"] = ["excession-tool-never-installed-xyz"]
        report = self.evaluate()
        self.assertEqual(report["status"], "INCONCLUSIVE")
        self.assertEqual(report["obligations"][1]["candidate"]["reason"], "EXECUTABLE_UNAVAILABLE")

    def test_nonzero_baseline_cannot_masquerade_as_equivalence(self):
        self.doc["obligations"][0]["argv"] = ["python3", "-c", "raise SystemExit(7)"]
        self.assertEqual(self.evaluate()["obligations"][0]["result"], "FAIL")

    def test_wrong_changed_paths_are_refused(self):
        self.doc["changed_paths"] = ["different.py"]
        with self.assertRaises(runner.Refused):
            self.evaluate()

    def test_identity_and_ancestry_are_authoritative(self):
        with self.assertRaises(runner.Refused):
            self.evaluate(base="HEAD")
        with self.assertRaises(runner.Refused):
            self.evaluate(base=self.candidate, candidate=self.base)

    def test_missing_cwd_refuses(self):
        self.doc["obligations"][0]["cwd"] = "not-a-directory"
        with self.assertRaises(runner.Refused):
            self.evaluate()

    def test_contract_rejects_semantic_shortcuts(self):
        self.doc["obligations"] = [self.doc["obligations"][1]]
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaises(runner.Refused):
            runner.read_contract(path)

    def test_contract_rejects_traversal(self):
        self.doc["obligations"][0]["cwd"] = "../outside"
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaises(runner.Refused):
            runner.read_contract(path)

    def test_contract_rejects_bad_output_budget(self):
        self.doc["obligations"][0]["max_output_bytes"] = -1
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaises(runner.Refused):
            runner.read_contract(path)

    def test_manifest_is_content_addressed(self):
        path = self.root / "contract.json"
        raw = json.dumps(self.doc, sort_keys=True).encode()
        path.write_bytes(raw)
        manifest, sha = runner.read_contract(path)
        self.assertEqual(manifest, self.doc)
        self.assertEqual(sha, hashlib.sha256(raw).hexdigest())

    def test_cli_refuses_without_trust_opt_in(self):
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        process = subprocess.run(
            ["python3", str(SRC), "--repo", str(self.repo),
             "--base", self.base, "--candidate", self.candidate,
             "--contract", str(path), "--out", str(self.root / "receipt.json")],
            capture_output=True,
        )
        self.assertNotEqual(process.returncode, 0)
        self.assertFalse((self.root / "receipt.json").exists())


if __name__ == "__main__":
    unittest.main()
