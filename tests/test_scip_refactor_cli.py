"""Controlled SCIP-decoder fixtures for cross-language refactor evidence.

These are intentionally synthetic SCIP indexer claims, not proof that FFI
exists. A real indexer/decoder end-to-end test is a separate acceptance gate.
"""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"

SYM_RUST = "scip rust cargo example 0.1.0 api/emit()."
SYM_PY = "scip python pypi example 0.1.0 app/parse()."


def run(*args: object, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        [str(x) for x in args], capture_output=True, check=False,
    )
    if check and result.returncode:
        raise AssertionError(
            f"exit {result.returncode}: {args!r}\n"
            + result.stderr.decode(errors="replace")
        )
    return result


class ScipRefactor(unittest.TestCase):
    def setUp(self) -> None:
        if not BIN.is_file():
            self.skipTest("build Excession CLI first")
        self.temp = tempfile.TemporaryDirectory(prefix="excession-scip-")
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.repo = self.work / "source"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q")
        (self.repo / "lib.rs").write_text("pub fn emit() {}\n")
        (self.repo / "app.py").write_text("def parse():\n    return 1\n")
        run("git", "-C", self.repo, "add", ".")
        run(
            "git", "-C", self.repo, "-c", "user.name=Fixture",
            "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "-m", "controlled multi-language source",
        )
        self.commit = run(
            "git", "-C", self.repo, "rev-parse", "HEAD",
        ).stdout.decode().strip()
        self.source = self.work / "index.json"
        self.report = self.work / "report.json"

    def index(self) -> dict:
        # scip print --json uses the Go bindings' encoding/json field names.
        return {
            "metadata": {
                "tool_info": {"name": "controlled-scip-fixture", "version": "0.1"}
            },
            "documents": [
                {
                    "relative_path": "lib.rs",
                    "language": "rust",
                    "position_encoding": 1,
                    "occurrences": [
                        {"range": [0, 7, 11], "symbol": SYM_RUST, "symbol_roles": 1},
                        {"range": [0, 7, 11], "symbol": SYM_PY},
                    ],
                },
                {
                    "relative_path": "app.py",
                    "language": "python",
                    "position_encoding": 1,
                    "occurrences": [
                        {"range": [0, 4, 9], "symbol": SYM_PY, "symbol_roles": 1},
                        {"range": [0, 4, 9], "symbol": SYM_RUST},
                    ],
                },
            ],
        }

    def suggest(self, index: dict | None = None, check: bool = True):
        self.source.write_text(json.dumps(self.index() if index is None else index))
        result = run(
            BIN, "refactor", "scip-suggest", "--repo", self.repo,
            "--rev", self.commit, "--index", self.source,
            check=check,
        )
        if check:
            return json.loads(result.stdout)
        return result

    def audit(self, check: bool = True):
        return run(
            BIN, "refactor", "scip-audit", "--repo", self.repo,
            "--rev", self.commit, "--index", self.source,
            "--report", self.report, check=check,
        )

    def test_source_bound_reciprocal_and_cross_language(self) -> None:
        report = self.suggest()
        self.assertEqual(report["schema"], "excession.scip-refactor.v1")
        self.assertEqual(report["source_commit"], self.commit)
        self.assertEqual(
            report["coverage"]["languages"], ["python", "rust"],
        )
        self.assertEqual(
            report["coverage"]["matched_cross_file_references"], 2,
        )
        self.assertEqual(
            report["coverage"]["unresolved_references"], 0,
        )
        kinds = [candidate["kind"] for candidate in report["candidates"]]
        self.assertEqual(kinds.count("reciprocal_symbol_references"), 1)
        self.assertEqual(kinds.count("cross_language_symbol_boundary"), 2)
        for candidate in report["candidates"]:
            self.assertEqual(
                candidate["status"], "UNVERIFIED_REFACTOR_HYPOTHESIS",
            )
        self.report.write_text(json.dumps(report))
        receipt = json.loads(self.audit().stdout)
        self.assertEqual(receipt["report_id"], report["report_id"])
        self.assertFalse(receipt["indexer_independently_executed"])
        self.assertEqual(self.suggest(), report)

    def test_no_name_based_cross_language_invention(self) -> None:
        index = self.index()
        index["documents"][1]["occurrences"][1]["symbol"] = (
            "scip rust cargo different 0.1.0 api/emit()."
        )
        report = self.suggest(index)
        self.assertEqual(
            report["coverage"]["unresolved_references"], 1,
        )
        self.assertEqual(
            sum(c["kind"] == "cross_language_symbol_boundary"
                for c in report["candidates"]), 1,
        )
        self.assertFalse(any(
            c["kind"] == "reciprocal_symbol_references"
            for c in report["candidates"]
        ))

    def test_tampering_and_invalid_source_are_refused(self) -> None:
        report = self.suggest()
        self.report.write_text(json.dumps(report))
        report["coverage"]["matched_cross_file_references"] = 200
        self.report.write_text(json.dumps(report))
        self.assertNotEqual(self.audit(check=False).returncode, 0)

        index = self.index()
        index["documents"][0]["relative_path"] = "../escape.rs"
        self.assertNotEqual(self.suggest(index, check=False).returncode, 0)
        index["documents"][0]["relative_path"] = "lib.rs"
        index["documents"][0]["occurrences"][0]["range"] = [90, 0, 1]
        self.assertNotEqual(self.suggest(index, check=False).returncode, 0)

        index = self.index()
        index["documents"][0]["text"] = "forged source"
        self.assertNotEqual(self.suggest(index, check=False).returncode, 0)

    def test_index_changes_invalidate_audit(self) -> None:
        self.report.write_text(json.dumps(self.suggest()))
        index = self.index()
        index["documents"][1]["occurrences"][1]["symbol"] = SYM_PY
        self.source.write_text(json.dumps(index))
        self.assertNotEqual(self.audit(check=False).returncode, 0)


if __name__ == "__main__":
    unittest.main()
