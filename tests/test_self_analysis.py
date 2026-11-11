"""Adversarial tests for the independent Git-bound self-analysis summary."""
from __future__ import annotations

import copy
import subprocess
import tempfile
from pathlib import Path
import unittest

from tools.self_analysis import AnalysisError, independently_verify, report_markdown


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True,
        capture_output=True, text=True,
    ).stdout.strip()


class SelfAnalysisTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="excession-dogfood-")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.name", "Self-analysis test")
        git(self.repo, "config", "user.email", "test@example.invalid")
        for path, content in {
            "core/src/lib.rs": "pub fn answer() -> u32 { 42 }\n",
            "core/src/bin/excession.rs": "use crate::answer;\n",
            "tools/helper.py": "def helper(): return 42\n",
        }.items():
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "baseline")
        self.base = git(self.repo, "rev-parse", "HEAD")
        (self.repo / "core/src/lib.rs").write_text(
            "pub fn answer() -> u32 { 43 }\n", encoding="utf-8",
        )
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "modify library")
        self.head = git(self.repo, "rev-parse", "HEAD")
        tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        self.roots = ("core/src", "tools")
        self.sources = [
            {
                "source_path": path,
                "source_mode": "100644",
                "source_blob": git(self.repo, "rev-parse", f"HEAD:{path}"),
                "language": "python" if path.endswith(".py") else "rust",
                "status": "PARSED",
                "structure": [],
                "imports": [],
                "diagnostics": [],
            }
            for path in (
                "core/src/lib.rs", "core/src/bin/excession.rs", "tools/helper.py",
            )
        ]
        self.syntax = {
            "schema": "excession.syntax-observations.v1",
            "source_commit": self.head,
            "source_tree": tree,
            "roots": list(self.roots),
            "completeness": "CONSERVATIVE_SYNTAX_ONLY",
            "report_id": "a" * 64,
            "source_count": len(self.sources),
            "sources": self.sources,
        }
        self.resolved = {
            "schema": "excession.syntax-dependencies.v1",
            "source_commit": self.head,
            "source_tree": tree,
            "syntax_report_id": self.syntax["report_id"],
            "completeness": "CONSERVATIVE_CANDIDATES_ONLY",
            "resolution_id": "b" * 64,
            "candidate_count": 1,
            "unresolved_count": 1,
            "candidate_edges": [{
                "source_path": "core/src/bin/excession.rs",
                "source_blob": self.sources[1]["source_blob"],
                "target_path": "core/src/lib.rs",
                "target_blob": self.sources[0]["source_blob"],
            }],
            "unresolved_imports": [{
                "source_path": "core/src/bin/excession.rs",
                "source_blob": self.sources[1]["source_blob"],
                "reason": "UNRESOLVED_OR_NONLOCAL_RUST_IMPORT",
            }],
            "skipped_sources": [],
        }
        self.syntax_receipt = {
            "status": "REPARSED_MATCH", "report_id": "a" * 64,
        }
        self.resolution_receipt = {
            "status": "REGENERATED_MATCH", "resolution_id": "b" * 64,
        }

    def summarize(self):
        return independently_verify(
            self.repo, self.head, self.roots, self.syntax, self.resolved,
            self.syntax_receipt, self.resolution_receipt, self.base,
        )

    def test_genuine_git_binding_and_advisory_change_impact(self) -> None:
        report = self.summarize()
        self.assertEqual(report["syntax"]["source_count"], 3)
        self.assertEqual(report["syntax"]["essential_rust_python_files"], 3)
        self.assertEqual(report["dependencies"]["unresolved_imports"], 1)
        self.assertEqual(report["dependencies"]["completeness"], "NOT_ESTABLISHED")
        self.assertEqual(report["change_impact"]["changed_paths"], ["core/src/lib.rs"])
        self.assertEqual(report["change_impact"]["candidate_affected_sources"],
                         ["core/src/bin/excession.rs"])
        self.assertFalse(report["assurance"]["test_skipping_authorized"])
        self.assertIn("Not established", report_markdown(report))

    def test_forged_git_blob_is_refused(self) -> None:
        self.syntax["sources"][0]["source_blob"] = "0" * 40
        with self.assertRaisesRegex(AnalysisError, "independent Git tree"):
            self.summarize()

    def test_omitted_rust_source_is_refused(self) -> None:
        self.syntax["sources"] = self.syntax["sources"][:-1]
        self.syntax["source_count"] = 2
        self.syntax["sources"] = [self.sources[0], self.sources[2]]
        with self.assertRaisesRegex(AnalysisError, "entry points missing"):
            self.summarize()

    def test_forged_replay_receipt_is_refused(self) -> None:
        self.resolution_receipt["resolution_id"] = "0" * 64
        with self.assertRaisesRegex(AnalysisError, "dependency replay"):
            self.summarize()

    def test_forged_candidate_target_is_refused(self) -> None:
        self.resolved["candidate_edges"][0]["target_blob"] = "0" * 40
        with self.assertRaisesRegex(AnalysisError, "candidate target Git binding"):
            self.summarize()

    def test_missing_unresolved_import_binding_is_refused(self) -> None:
        self.resolved["unresolved_imports"][0]["source_blob"] = "0" * 40
        with self.assertRaisesRegex(AnalysisError, "unresolved import"):
            self.summarize()

    def test_dirty_worktree_is_refused(self) -> None:
        (self.repo / "core/src/lib.rs").write_text("changed, not committed\n")
        with self.assertRaisesRegex(AnalysisError, "dirty"):
            self.summarize()

    def test_false_completeness_claim_is_refused(self) -> None:
        self.resolved["completeness"] = "COMPLETE"
        with self.assertRaisesRegex(AnalysisError, "unsupported completeness"):
            self.summarize()

    def test_nonmatching_git_commit_is_refused(self) -> None:
        with self.assertRaisesRegex(AnalysisError, "checked-out Git commit"):
            independently_verify(
                self.repo, self.base, self.roots, self.syntax, self.resolved,
                self.syntax_receipt, self.resolution_receipt,
            )


if __name__ == "__main__":
    unittest.main()
