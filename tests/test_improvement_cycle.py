"""Acceptance and adversarial fixtures for reproducible automatic native edits."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SRC = Path(__file__).resolve().parents[1] / "tools" / "ci" / "improvement_cycle.py"
spec = importlib.util.spec_from_file_location("improvement_cycle", SRC)
cycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cycle)


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def catalog():
    return {
        "schema": cycle.SCHEMA,
        "recipes": [
            {"id": "native-rustfmt", "producer": "cargo-fmt-pinned",
             "integration": "automatic-after-exact-head-verification",
             "paths": ["core/**/*.rs"], "rationale": "Native compiler-format invariant",
             "evidence": "https://example.invalid/native-formatter",
             "independent_checks": ["formatter-replay", "quality-ratchet", "cargo-tests",
                                    "verify-workflow", "github-evidence-workflow"]},
            {"id": "native-clippy", "producer": "cargo-clippy-fix-pinned",
             "integration": "review-required", "paths": ["core/**/*.rs"],
             "rationale": "Compiler transformations require review",
             "evidence": "https://example.invalid/independent-oracles",
             "independent_checks": ["cargo-tests", "human-review"]},
            {"id": "native-cargo-lock-refresh", "producer": "cargo-update-locked",
             "integration": "automatic-after-exact-head-verification",
             "paths": ["core/Cargo.lock"], "rationale": "Patch-only graph-preserving updates",
             "evidence": "https://example.invalid/graph-audit",
             "independent_checks": ["cargo-lock-graph-audit", "cargo-tests",
                                    "quality-ratchet", "verify-workflow",
                                    "github-evidence-workflow"]},
        ],
    }


class NativeImprovementCycle(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Fixture")
        git(self.repo, "config", "user.email", "fixture@example.invalid")
        self.write("quality/improvement-recipes.json", json.dumps(catalog()) + "\n")
        self.write("core/src/lib.rs", "pub  fn answer() -> u32 { 42 }\n")
        self.base = self.commit()
        self.write("core/src/lib.rs", "pub fn answer() -> u32 { 42 }\n")
        self.candidate = self.commit()
        self.formatter = [sys.executable, "-c",
                          "from pathlib import Path; p=Path('core/src/lib.rs'); "
                          "p.write_text(p.read_text().replace('pub  fn', 'pub fn'))"]

    def write(self, path, value):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)

    def commit(self):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "fixture")
        return git(self.repo, "rev-parse", "HEAD")

    def fork_from_base(self):
        git(self.repo, "reset", "--hard", self.base)

    def verify(self, candidate=None, formatter=None):
        return cycle.verify(self.repo, self.base, candidate or self.candidate,
                            formatter=formatter or self.formatter)

    def test_native_reproduction_is_exact_and_still_pending_independent_ci(self):
        receipt = self.verify()
        self.assertEqual(receipt["native_replay"], "EXACT_GIT_BLOB_MATCH")
        self.assertEqual(receipt["status"], "REPRODUCED_PENDING_INDEPENDENT_CI")
        self.assertFalse(receipt["ready_to_merge"])
        self.assertFalse(receipt["semantic_equivalence_proved"])
        self.assertEqual(receipt["changed_paths"], ["core/src/lib.rs"])
        self.assertEqual(receipt["candidate_tree"], git(self.repo, "rev-parse", self.candidate + "^{tree}"))
        self.assertEqual(len(git(self.repo, "worktree", "list").splitlines()), 1)

    def test_proposed_change_different_from_native_replay_fails(self):
        self.fork_from_base()
        self.write("core/src/lib.rs", "pub fn answer() -> u32 { 43 }\n")
        bad = self.commit()
        with self.assertRaisesRegex(cycle.Refused, "differs from candidate"):
            self.verify(candidate=bad)
        self.assertEqual(len(git(self.repo, "worktree", "list").splitlines()), 1)

    def test_native_producer_unavailable_refuses(self):
        with self.assertRaisesRegex(cycle.Refused, "unavailable or timed out"):
            self.verify(formatter=["unavailable-format-producer-nothing"])

    def test_candidate_changes_unapproved_path_refuses(self):
        self.fork_from_base()
        self.write("docs/INTRO.md", "unreviewed changes\n")
        bad = self.commit()
        with self.assertRaisesRegex(cycle.Refused, "must not add"):
            self.verify(candidate=bad)

    def test_candidate_changes_a_python_file_refuses(self):
        self.fork_from_base()
        self.write("tools/example.py", "print('not native rustfmt')\n")
        bad = self.commit()
        with self.assertRaisesRegex(cycle.Refused, "must not add"):
            self.verify(candidate=bad)

    def test_candidate_changes_own_recipe_authority_refuses(self):
        self.fork_from_base()
        obj = catalog()
        obj["recipes"][0]["rationale"] = "Changed in a generated commit"
        self.write("quality/improvement-recipes.json", json.dumps(obj))
        bad = self.commit()
        with self.assertRaisesRegex(cycle.Refused, "own recipe authority"):
            self.verify(candidate=bad)

    def test_candidate_must_be_one_commit_from_base(self):
        self.write("core/src/lib.rs", "pub fn answer() -> u32 { 44 }\n")
        beyond = self.commit()
        with self.assertRaisesRegex(cycle.Refused, "directly atop"):
            self.verify(candidate=beyond)

    def test_missing_diff_refuses(self):
        git(self.repo, "commit", "--allow-empty", "-qm", "no source change")
        empty = git(self.repo, "rev-parse", "HEAD")
        with self.assertRaisesRegex(cycle.Refused, "1-128"):
            cycle.verify(self.repo, self.candidate, empty, formatter=self.formatter)

    def test_path_mode_changes_refuse(self):
        self.fork_from_base()
        (self.repo / "core/src/lib.rs").chmod(0o755)
        bad = self.commit()
        with self.assertRaises(cycle.Refused):
            self.verify(candidate=bad)

    def test_native_tool_modifying_nonrust_file_refuses(self):
        malformed = [sys.executable, "-c",
                     "from pathlib import Path; Path('another.txt').write_text('bad')"]
        with self.assertRaisesRegex(cycle.Refused, "unsupported tracked/untracked source"):
            self.verify(formatter=malformed)

    def test_catalog_accepts_new_review_only_recipe(self):
        obj = catalog()
        obj["recipes"].append(
            {"id": "future-native", "producer": "some-reviewed-compiler",
             "integration": "review-required", "paths": ["core/**/*.rs"],
             "rationale": "New reviewed finding", "evidence": "link",
             "independent_checks": ["human-review"]}
        )
        self.assertEqual(len(cycle.validated_catalog(json.dumps(obj).encode())["recipes"]), 4)

    def test_catalog_refuses_new_automatic_authority(self):
        obj = catalog()
        obj["recipes"].append(
            {"id": "unsafe-auto", "producer": "arbitrary-script",
             "integration": "automatic-after-exact-head-verification",
             "paths": ["core/**/*.rs"], "rationale": "Impossible safety",
             "evidence": "link", "independent_checks": ["human-review"]}
        )
        with self.assertRaises(cycle.Refused):
            cycle.validated_catalog(json.dumps(obj).encode())

    def test_catalog_refuses_removal_of_native_replay(self):
        obj = catalog()
        obj["recipes"][0]["independent_checks"].remove("formatter-replay")
        with self.assertRaises(cycle.Refused):
            cycle.validated_catalog(json.dumps(obj).encode())

    def test_catalog_refuses_promoted_clippy(self):
        obj = catalog()
        obj["recipes"][1]["integration"] = "automatic-after-exact-head-verification"
        with self.assertRaises(cycle.Refused):
            cycle.validated_catalog(json.dumps(obj).encode())

    def test_catalog_refuses_duplicate_recipe(self):
        obj = catalog()
        obj["recipes"].append(obj["recipes"][0])
        with self.assertRaises(cycle.Refused):
            cycle.validated_catalog(json.dumps(obj).encode())

    def test_cli_verification_requires_two_exact_commits(self):
        with self.assertRaises(cycle.Refused):
            self.verify(candidate="HEAD")


if __name__ == "__main__":
    unittest.main()
