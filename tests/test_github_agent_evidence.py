"""Independent Git and fail-closed contract tests for the Actions evidence bridge."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


MODULE = Path(__file__).resolve().parents[1] / "tools" / "github_agent_evidence.py"
SPEC = importlib.util.spec_from_file_location("github_agent_evidence", MODULE)
assert SPEC is not None and SPEC.loader is not None
bridge = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bridge)


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), *args], text=True
    ).strip()


class GitHubEvidenceTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="excession-evidence-")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Fixture")
        git(self.repo, "config", "user.email", "fixture@example.invalid")
        (self.repo / "deleted.py").write_text("from os import path\n")
        (self.repo / "retained.py").write_text("answer = 1\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "base")
        self.base = git(self.repo, "rev-parse", "HEAD")
        (self.repo / "deleted.py").unlink()
        (self.repo / "retained.py").write_text("answer = 2\n")
        (self.repo / "added.py").write_text("answer = 3\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "candidate")
        self.head = git(self.repo, "rev-parse", "HEAD")

    def test_real_git_commit_and_tree_identities(self):
        item = bridge.identity(self.repo, self.head)
        self.assertEqual(item["commit"], self.head)
        self.assertEqual(item["tree"], git(self.repo, "rev-parse", "HEAD^{tree}"))
        for bad in (self.head[:12], self.head.upper(), "HEAD", "f" * 40 + " --exec"):
            with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                bridge.identity(self.repo, bad)

    def test_context_only_uses_tracked_candidate_regular_sources(self):
        paths, count = bridge.choose_focus(self.repo, self.base, self.head, "")
        self.assertEqual(paths, ["added.py", "retained.py"])
        self.assertEqual(count, 2)
        paths, count = bridge.choose_focus(
            self.repo, self.base, self.head, "retained.py"
        )
        self.assertEqual(paths, ["retained.py"])
        self.assertEqual(count, 2)
        for bad in ("deleted.py", "../retained.py", "/etc/passwd",
                    "retained.py,retained.py", ".git/config"):
            with self.assertRaises(ValueError):
                bridge.choose_focus(self.repo, self.base, self.head, bad)

    def test_mismatched_or_overstated_native_verification_is_refused(self):
        base = bridge.identity(self.repo, self.base)
        head = bridge.identity(self.repo, self.head)
        good = {
            "schema": "excession.agent-verification.v1",
            "base_commit": base["commit"], "base_tree": base["tree"],
            "candidate_commit": head["commit"], "candidate_tree": head["tree"],
            "ready_to_merge": False, "safe_to_skip_unlisted_tests": False,
            "replay": {"semantic_correctness": "NOT_EVALUATED"},
            "coverage": {"status": "PARTIAL", "truncated": False},
            "obligations": ["SEMANTIC_CORRECTNESS_NOT_EVALUATED"],
        }
        bridge.validate_verification(good, base, head)
        for field, value in (
            ("candidate_tree", base["tree"]), ("ready_to_merge", True),
            ("safe_to_skip_unlisted_tests", True), ("obligations", None)
        ):
            fake = dict(good, **{field: value})
            with self.assertRaises(ValueError):
                bridge.validate_verification(fake, base, head)
        fake = dict(good, replay={"semantic_correctness": "VERIFIED"})
        with self.assertRaises(ValueError):
            bridge.validate_verification(fake, base, head)

    def test_context_rejects_stale_sources(self):
        head = bridge.identity(self.repo, self.head)
        base = bridge.identity(self.repo, self.base)
        valid = {
            "schema": "excession.agent-context.v1",
            "source_commit": head["commit"], "source_tree": head["tree"],
            "safe_to_skip_unlisted_tests": False, "test_candidates": [],
            "sources": [{"source_commit": head["commit"], "source_tree": head["tree"]}],
        }
        bridge.validate_context(valid, head)
        valid["sources"][0]["source_tree"] = base["tree"]
        with self.assertRaises(ValueError):
            bridge.validate_context(valid, head)


if __name__ == "__main__":
    unittest.main()
