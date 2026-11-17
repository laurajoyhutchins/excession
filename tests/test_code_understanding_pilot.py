"""Fast, offline tests for the understanding pilot's *diagnostic*, not the agent."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / "benchmarks/understanding/pilot.py"
spec = importlib.util.spec_from_file_location("understanding_pilot", MODULE)
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class PilotTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name)
        subprocess.run(["git", "-C", str(self.repo), "init", "-q"], check=True)
        (self.repo/"src").mkdir()
        (self.repo/"test").mkdir()
        (self.repo/"src/worker.c").write_text(
            "int reclaim_lease(void) { return 1; }\n")
        (self.repo/"test/worker.test").write_text(
            "# tests reclaim_lease edge cases\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True)
        subprocess.run(["git", "-C", str(self.repo), "-c", "user.name=Pilot",
                        "-c", "user.email=pilot@example.invalid", "commit", "-qm",
                        "Source under test"], check=True)
        self.commit = subprocess.check_output(
            ["git", "-C", str(self.repo), "rev-parse", "HEAD"], text=True).strip()
        self.case = {
            "id": "worker", "repository": "upstream/project",
            "commit": self.commit,
            "question": "Where does reclaim lease occur and what tests cover it?",
            "oracle": [
                {"path": "src/worker.c", "role": "implementation",
                 "anchors": ["int reclaim_lease(void)"]},
                {"path": "test/worker.test", "role": "test",
                 "anchors": ["tests reclaim_lease edge cases"]},
            ],
        }

    def test_oracle_source_anchors_read_git_objects_not_worktree(self):
        expected = pilot.oracle_validate(self.case, self.repo)
        self.assertEqual(expected["oracle_paths"],
                         ["src/worker.c", "test/worker.test"])
        (self.repo/"src/worker.c").write_text("not the committed source\n")
        self.assertEqual(pilot.oracle_validate(self.case, self.repo), expected)
        self.case["oracle"][0]["anchors"] = ["not in any Git blob"]
        with self.assertRaisesRegex(pilot.Refusal, "anchor absent"):
            pilot.oracle_validate(self.case, self.repo)

    def test_git_baseline_and_proxy_do_not_claim_semantic_understanding(self):
        result = pilot.baseline_git_grep(self.case, self.repo)
        self.assertIn("src/worker.c", result["paths"])
        self.assertIn("test/worker.test", result["paths"])
        self.assertEqual(len(result["paths"]), len(set(result["paths"])))
        proxy = pilot.discovery_proxy(result["paths"], self.case)
        self.assertTrue(proxy["all_required_found"])
        self.assertEqual(proxy["meaning"],
                         "PATH_RETRIEVAL_PROXY_NOT_BEHAVIORAL_UNDERSTANDING")
        missing = pilot.discovery_proxy(["src/worker.c"], self.case)
        self.assertFalse(missing["all_required_found"])
        self.assertEqual(missing["missing_required"], ["test/worker.test"])

    def test_keyword_selection_and_refusals(self):
        self.assertEqual(pilot.question_terms("How does return lease recovery work?"),
                         ["return", "lease", "recovery", "work"])
        with self.assertRaises(pilot.Refusal):
            pilot.question_terms("what is the it")
        self.case["oracle"][0]["path"] = "../private"
        with self.assertRaisesRegex(pilot.Refusal, "invalid oracle path"):
            pilot.oracle_validate(self.case, self.repo)


if __name__ == "__main__":
    unittest.main()
