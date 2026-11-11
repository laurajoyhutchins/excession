"""SCIP-first source indexing, with no Python or JavaScript parser step.

SCIP records here are deliberately controlled claims, not an indexer acceptance
test or evidence that a Python-to-Rust import is executable.
"""
from __future__ import annotations

import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"
RUST = "scip rust cargo example 0.1.0 api/emit()."
PYTHON = "scip python pypi example 0.1.0 app/parse()."


def run(*args: object, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        [str(x) for x in args], capture_output=True, check=False,
    )
    if check and result.returncode:
        raise AssertionError(
            f"{args!r}: {result.stderr.decode(errors='replace')}"
        )
    return result


class ScipIndex(unittest.TestCase):
    def setUp(self) -> None:
        if not BIN.is_file():
            self.skipTest("build Excession before running SCIP indexing fixtures")
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-scip-index-")
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        self.repo = self.work / "source"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q")
        (self.repo / "lib.rs").write_text("pub fn emit() {}\n")
        (self.repo / "app.py").write_text("def parse():\n    return 1\n")
        (self.repo / "unused.ts").write_text("export const unused = 1\n")
        run("git", "-C", self.repo, "add", ".")
        run(
            "git", "-C", self.repo,
            "-c", "user.name=Fixture",
            "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "-m", "SCIP source fixtures",
        )
        self.commit = run(
            "git", "-C", self.repo, "rev-parse", "HEAD",
        ).stdout.decode().strip()
        self.db = self.work / "index.sqlite"
        self.input = self.work / "scip.json"

    def scip(self) -> dict:
        return {
            "metadata": {
                "tool_info": {"name": "fixture-indexer", "version": "1.2.3"},
            },
            "documents": [
                {
                    "relative_path": "lib.rs", "language": "rust",
                    "position_encoding": 1,
                    "occurrences": [
                        {"range": [0, 7, 11], "symbol": RUST, "symbol_roles": 1},
                        {"range": [0, 7, 11], "symbol": PYTHON},
                    ],
                },
                {
                    "relative_path": "app.py", "language": "python",
                    "position_encoding": 1,
                    "occurrences": [
                        {"range": [0, 4, 9], "symbol": PYTHON, "symbol_roles": 1},
                        {"range": [0, 4, 9], "symbol": RUST, "symbol_roles": 2},
                    ],
                },
                {
                    "relative_path": "unused.ts", "language": "typescript",
                    "occurrences": [],
                },
            ],
        }

    def index(self, payload: dict | None = None, check: bool = True):
        self.input.write_text(json.dumps(self.scip() if payload is None else payload))
        result = run(
            BIN, "index", "--repo", self.repo, "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
            "--scip-index", self.input, check=check,
        )
        return json.loads(result.stdout) if check else result

    def audit(self, check: bool = True):
        result = run(
            BIN, "inspect", "scip-audit", "--repo", self.repo, "--db", self.db,
            "--name", "fixture", "--rev", self.commit, "--index", self.input,
            check=check,
        )
        return json.loads(result.stdout) if check else result

    def test_index_and_replay_import_role_only(self) -> None:
        result = self.index()
        self.assertEqual(result["status"], "SCIP_IMPORT_RELATIONS_INDEXED")
        self.assertEqual(result["source_commit"], self.commit)
        self.assertEqual(result["coverage"]["indexed_documents"], 3)
        self.assertEqual(result["coverage"]["indexed_import_role_edges"], 1)
        self.assertEqual(result["coverage"]["non_import_references_not_promoted"], 1)
        self.assertEqual(
            result["coverage"]["languages"], ["python", "rust", "typescript"],
        )

        with sqlite3.connect(self.db) as con:
            modules = con.execute(
                "SELECT name,path,blob_oid FROM module ORDER BY path"
            ).fetchall()
            self.assertEqual([m[0] for m in modules],
                             ["app.py", "lib.rs", "unused.ts"])
            for name, path, blob in modules:
                self.assertEqual(name, path)
                self.assertEqual(
                    blob, run("git", "-C", self.repo, "rev-parse",
                              f"{self.commit}:{path}").stdout.decode().strip(),
                )
            self.assertEqual(con.execute(
                "SELECT source_name,target_name,line,syntax FROM dependency"
            ).fetchall(), [("app.py", "lib.rs", 1, "scip:import-role")])
            self.assertEqual(con.execute(
                "SELECT index_sha256,indexer_name,indexer_version FROM scip_index_metadata"
            ).fetchone()[1:], ("fixture-indexer", "1.2.3"))

        self.assertEqual(self.audit()["status"], "REPLAYED_SCIP_INDEXED_RELATIONS")
        inspection = json.loads(run(
            BIN, "inspect", "dependencies", "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
        ).stdout)
        self.assertTrue(inspection)
        hypotheses = json.loads(run(
            BIN, "refactor", "suggest", "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
        ).stdout)
        self.assertIn("SCIP", hypotheses["coverage"]["scope"])
        self.assertEqual(hypotheses["coverage"]["indexed_symbols"], 0)

    def test_audit_rejects_forged_edges_and_modules(self) -> None:
        self.index()
        with sqlite3.connect(self.db) as con:
            con.execute("DELETE FROM dependency")
        bad = self.audit(check=False)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn(b"REFUSED:", bad.stderr)

    def test_index_failure_and_no_silent_fallback(self) -> None:
        payload = self.scip()
        payload["documents"][0]["relative_path"] = "../outside.rs"
        self.assertNotEqual(self.index(payload, check=False).returncode, 0)
        self.input.write_text(json.dumps(self.scip()))
        # Valid SCIP input is explicitly opt-in and is not silently mixed
        # with the old native parser index under the same source identity.
        run(
            BIN, "index", "--repo", self.repo, "--db", self.db,
            "--name", "fixture", "--rev", self.commit,
            "--scip-index", self.input,
        )
        self.assertNotEqual(
            run(
                BIN, "index", "--repo", self.repo, "--db", self.db,
                "--name", "fixture", "--rev", self.commit,
                "--scip-index", self.input, check=False,
            ).returncode,
            0,
        )
        self.assertNotEqual(
            run(
                BIN, "index", "--repo", self.repo, "--db", self.db,
                "--name", "other", "--rev", self.commit,
                "--scip-index", self.input, "--roots", "src",
                check=False,
            ).returncode,
            0,
        )


if __name__ == "__main__":
    unittest.main()
