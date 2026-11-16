"""GitHub artifact evidence must preserve Git identity and unresolved uncertainty."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("github_evidence", ROOT / "tools" / "github_evidence.py")
evidence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evidence)


def run(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args]).decode().strip()


class GithubEvidence(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name) / "source"
        self.repo.mkdir()
        subprocess.run(["git", "-C", str(self.repo), "init", "-q", "-b", "main"], check=True)
        run(self.repo, "config", "user.email", "test@example.invalid")
        run(self.repo, "config", "user.name", "Evidence test")
        (self.repo / "src").mkdir()
        (self.repo / "tests").mkdir()
        (self.repo / "src" / "module.py").write_text("value = 1\n")
        (self.repo / "tests" / "test_module.py").write_text("def test_value(): assert True\n")
        run(self.repo, "add", ".")
        run(self.repo, "commit", "-qm", "base")
        self.base = run(self.repo, "rev-parse", "HEAD")
        (self.repo / "src" / "module.py").write_text("value = 2\n")
        run(self.repo, "add", ".")
        run(self.repo, "commit", "-qm", "candidate")
        self.head = run(self.repo, "rev-parse", "HEAD")
        self.base_tree = run(self.repo, "rev-parse", self.base + "^{tree}")
        self.head_tree = run(self.repo, "rev-parse", self.head + "^{tree}")

    def verification(self):
        return {
            "schema": "excession.agent-verification.v1",
            "base_commit": self.base,
            "base_tree": self.base_tree,
            "candidate_commit": self.head,
            "candidate_tree": self.head_tree,
            "change_count": 1,
            "report_id": "a" * 64,
            "coverage": {"status": "PARTIAL", "truncated": False},
            "status": "OPEN_OBLIGATIONS",
            "replay": {"semantic_correctness": "NOT_EVALUATED"},
            "obligations": ["SEMANTIC_CORRECTNESS_NOT_EVALUATED"],
            "ready_to_merge": False,
            "safe_to_skip_unlisted_tests": False,
        }

    def context(self):
        return {
            "schema": "excession.agent-context.v1",
            "source_commit": self.head,
            "source_tree": self.head_tree,
            "report_id": "b" * 64,
            "sources": [
                {
                    "source_commit": self.head,
                    "source_tree": self.head_tree,
                    "path": "src/module.py",
                    "blob": run(self.repo, "rev-parse", self.head + ":src/module.py"),
                    "mode": "100644",
                }
            ],
            "coverage": {"status": "PARTIAL", "gaps": [{"reason": "TEST_SELECTION_NOT_PROVEN"}], "truncated": False},
            "test_candidates": [
                {"source": {"path": "tests/test_module.py"}, "reason": "PATH_NAME_HEURISTIC", "selection_verified": False}
            ],
            "safe_to_skip_unlisted_tests": False,
        }

    def produce(self):
        with patch.object(evidence, "run_excession", side_effect=[self.verification(), self.context()]):
            return evidence.make_report(
                self.repo, Path("/excession"), self.base, self.head,
                "example/project", "c" * 40, "", "123456",
            )

    def test_envelope_preserves_source_witness_and_nonapproval(self):
        report = self.produce()
        self.assertEqual(report["schema"], "excession.github-evidence.v1")
        self.assertEqual(report["base"]["tree"], self.base_tree)
        self.assertEqual(report["head"]["tree"], self.head_tree)
        self.assertEqual(report["source_witnesses"][0]["blob"], self.context()["sources"][0]["blob"])
        self.assertEqual(report["coverage"]["changed_paths"], 1)
        self.assertEqual(report["coverage"]["focus_complete"], True)
        self.assertEqual(report["suggested_checks"][0]["selection_verified"], False)
        self.assertIn("SEMANTIC_CORRECTNESS_NOT_EVALUATED", report["unresolved_obligations"])
        self.assertIn("TEST_SELECTION_NOT_PROVEN", report["unresolved_obligations"])
        self.assertEqual(report["producer"]["source_commit"], "c" * 40)
        self.assertEqual(report["provenance"]["github_run_id"], "123456")
        self.assertFalse(report["ready_to_merge"])
        self.assertFalse(report["safe_to_skip_unlisted_tests"])
        self.assertEqual(len(report["report_sha256"]), 64)
        self.assertIn("not a correctness or merge approval", evidence.summary(report))

    def test_rejects_mismatched_excession_tree(self):
        wrong = self.verification()
        wrong["candidate_tree"] = "0" * 40
        with patch.object(evidence, "run_excession", return_value=wrong):
            with self.assertRaisesRegex(evidence.EvidenceError, "candidate_tree"):
                evidence.make_report(self.repo, Path("/excession"), self.base, self.head, "example/project", "c" * 40)

    def test_rejects_fabricated_blob_witness(self):
        wrong = self.context()
        wrong["sources"][0]["blob"] = "0" * 40
        with patch.object(evidence, "run_excession", side_effect=[self.verification(), wrong]):
            with self.assertRaisesRegex(evidence.EvidenceError, "Source witness blob"):
                evidence.make_report(self.repo, Path("/excession"), self.base, self.head, "example/project", "c" * 40)

    def test_rejects_claim_of_semantic_approval(self):
        wrong = self.verification()
        wrong["ready_to_merge"] = True
        with patch.object(evidence, "run_excession", return_value=wrong):
            with self.assertRaisesRegex(evidence.EvidenceError, "ready-to-merge"):
                evidence.make_report(self.repo, Path("/excession"), self.base, self.head, "example/project", "c" * 40)

    def test_deletion_only_preserves_no_context_obligation(self):
        # Deletion-only revisions cannot be passed as a non-existent head source.
        run(self.repo, "rm", "src/module.py")
        run(self.repo, "commit", "-qm", "delete source")
        deleted_head = run(self.repo, "rev-parse", "HEAD")
        verify = self.verification()
        verify["base_commit"] = self.head
        verify["base_tree"] = self.head_tree
        verify["candidate_commit"] = deleted_head
        verify["candidate_tree"] = run(self.repo, "rev-parse", "HEAD^{tree}")
        with patch.object(evidence, "run_excession", return_value=verify) as runner:
            report = evidence.make_report(
                self.repo, Path("/excession"), self.head, deleted_head,
                "example/project", "c" * 40,
            )
        runner.assert_called_once()
        self.assertEqual(report["source_witnesses"], [])
        self.assertEqual(report["suggested_checks"], [])
        self.assertIsNone(report["context"])
        self.assertIn("GITHUB_EVIDENCE_NO_CONTEXT_WITNESSES", report["unresolved_obligations"])

    def test_fail_closed_writes_diagnostic_not_report(self):
        out = Path(self.temporary.name) / "output"
        argv = [
            "github_evidence.py", "--repo", str(self.repo), "--bin", "/excession",
            "--base", self.base, "--head", self.head, "--repository", "example/project",
            "--tool-commit", "c" * 40, "--out", str(out),
        ]
        wrong = self.verification()
        wrong["ready_to_merge"] = True
        with patch.object(sys, "argv", argv), patch.object(evidence, "run_excession", return_value=wrong):
            self.assertEqual(evidence.main(), 1)
        self.assertFalse((out / "github-evidence.v1.json").exists())
        self.assertEqual(json.loads((out / "failure.json").read_text())["schema"], "excession.github-evidence-failure.v1")
        self.assertIn("failed", (out / "summary.md").read_text())


if __name__ == "__main__":
    unittest.main()
