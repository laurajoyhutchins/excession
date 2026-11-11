"""End-to-end refactor hypotheses on a pinned, controlled Git checkout."""
from __future__ import annotations

import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"


def run(*args: object, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run([str(arg) for arg in args], capture_output=True, check=False)
    if check and result.returncode:
        raise AssertionError(
            f"{args!r} failed: {result.stderr.decode(errors='replace')}"
        )
    return result


class RefactorCandidates(unittest.TestCase):
    def setUp(self) -> None:
        if not BIN.is_file():
            self.skipTest("build Excession CLI first")
        self.temp = tempfile.TemporaryDirectory(prefix="excession-refactor-")
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.repo = self.work / "repo"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q")
        (self.repo / "pkg").mkdir()
        (self.repo / "pkg" / "__init__.py").write_text("")
        (self.repo / "pkg" / "a.py").write_text(
            "from pkg import b\n\ndef shared(value):\n    return value + 1\n"
        )
        (self.repo / "pkg" / "b.py").write_text(
            "from pkg import a\n\ndef shared(value):\n    return value + 1\n"
        )
        run("git", "-C", self.repo, "add", ".")
        run(
            "git", "-C", self.repo, "-c", "user.name=Fixture",
            "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "-m", "controlled cycle and duplicate",
        )
        self.commit = run(
            "git", "-C", self.repo, "rev-parse", "HEAD"
        ).stdout.decode().strip()
        self.db = self.work / "index.sqlite"
        run(
            BIN, "index", "--repo", self.repo, "--db", self.db,
            "--name", "fixture", "--rev", self.commit, "--roots", "pkg",
        )

    def suggest(self) -> dict:
        return json.loads(run(
            BIN, "refactor", "suggest", "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
        ).stdout)

    def audit(self, path: Path, check: bool = True) -> subprocess.CompletedProcess[bytes]:
        return run(
            BIN, "refactor", "audit", "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
            "--report", path, check=check,
        )

    def test_exact_sources_cycle_duplicates_and_replay(self) -> None:
        report = self.suggest()
        self.assertEqual(report["schema"], "excession.refactor-candidates.v1")
        self.assertEqual(report["source_commit"], self.commit)
        self.assertEqual(report["status"], "UNVERIFIED_REFACTOR_HYPOTHESES")
        self.assertEqual(
            sorted(x["kind"] for x in report["candidates"]),
            ["dependency_cycle", "duplicate_body_fingerprint"],
        )
        cycle = next(
            x for x in report["candidates"] if x["kind"] == "dependency_cycle"
        )
        self.assertEqual(
            {x["name"] for x in cycle["modules"]}, {"pkg.a", "pkg.b"}
        )
        self.assertEqual(len(cycle["edges"]), 2)
        duplicate = next(
            x for x in report["candidates"]
            if x["kind"] == "duplicate_body_fingerprint"
        )
        self.assertEqual(duplicate["language"], "python")
        self.assertEqual(
            {x["path"] for x in duplicate["symbols"]},
            {"pkg/a.py", "pkg/b.py"},
        )
        for symbol in duplicate["symbols"]:
            self.assertEqual(
                symbol["blob"],
                run(
                    "git", "-C", self.repo, "rev-parse",
                    f"{self.commit}:{symbol['path']}",
                ).stdout.decode().strip(),
            )
        self.assertEqual(report, self.suggest(), "deterministic report")
        path = self.work / "report.json"
        path.write_text(json.dumps(report))
        receipt = json.loads(self.audit(path).stdout)
        self.assertEqual(receipt["report_id"], report["report_id"])
        self.assertFalse(receipt["semantic_reparse_performed"])

    def test_report_and_relation_tampering_are_refused(self) -> None:
        report = self.suggest()
        path = self.work / "report.json"
        report["candidates"][0]["action"] = "forged advice"
        path.write_text(json.dumps(report))
        self.assertNotEqual(self.audit(path, check=False).returncode, 0)
        path.write_text(json.dumps(self.suggest()))
        with sqlite3.connect(self.db) as connection:
            connection.execute(
                "DELETE FROM dependency WHERE source_name=? AND target_name=?",
                ("pkg.a", "pkg.b"),
            )
        self.assertNotEqual(self.audit(path, check=False).returncode, 0)

    def test_refuses_unindexed_revision_and_duplicate_flags(self) -> None:
        (self.repo / "extra.txt").write_text("new commit\n")
        run("git", "-C", self.repo, "add", ".")
        run(
            "git", "-C", self.repo, "-c", "user.name=Fixture",
            "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "-m", "next",
        )
        self.assertEqual(self.suggest()["source_commit"], self.commit)
        bad = run(
            BIN, "refactor", "suggest", "--db", self.db,
            "--name", "fixture", "--rev", "HEAD", check=False,
        )
        self.assertNotEqual(bad.returncode, 0)
        self.assertEqual(bad.stdout, b"")
        bad = run(
            BIN, "refactor", "suggest", "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
            "--rev", self.commit, check=False,
        )
        self.assertNotEqual(bad.returncode, 0)


if __name__ == "__main__":
    unittest.main()
