"""End-to-end production Rust/SQLite incremental Python indexing acceptance.

Never infer correctness from an incremental database's own view: compare the
entire public analysis report and stored derived rows against a fresh database
for the exact same Git revision. The expected relationship fixture is also
independently determined from the tracked source code.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"


def call(*args, check=True, env=None):
    process = subprocess.run(
        [str(a) for a in args], check=False, text=True, capture_output=True,
        timeout=90, env=env,
    )
    if check and process.returncode:
        raise AssertionError(
            f"{args!r}\nstdout: {process.stdout[-2000:]}\nstderr: {process.stderr[-2000:]}"
        )
    return process


def git(repo, *args):
    return call("git", "-C", repo, *args).stdout.strip()


def recorded(db, commit):
    with sqlite3.connect(db) as con:
        return {
            table: con.execute(
                f"SELECT {fields} FROM {table} WHERE commit_oid=? ORDER BY {sort}",
                (commit,),
            ).fetchall()
            for table, fields, sort in [
                ("module", "name,path,blob_oid,is_test", "name,path"),
                ("dependency", "source_name,target_name,line,syntax",
                 "source_name,target_name,line,syntax"),
                ("symbol", "module_name,qualname,kind,signature,body_fingerprint,line",
                 "module_name,qualname"),
                ("api_binding", "module_name,public_name,target,explicitly_exported",
                 "module_name,public_name"),
                ("hazard", "module_name,line,reason", "module_name,line,reason"),
            ]
        }


@unittest.skipUnless(BIN.is_file() or os.environ.get("EXCESSION_REQUIRE_BIN") == "1",
                     "native Excession binary is required in CI")
class ProductionPythonIncremental(unittest.TestCase):
    def setUp(self):
        if not BIN.is_file():
            self.fail("EXCESSION_REQUIRE_BIN was set but native binary was not built")
        tmp = tempfile.TemporaryDirectory(prefix="excession-prod-inc-")
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)
        self.repo = self.home / "repo"
        self.repo.mkdir()
        call("git", "init", "-q", "-b", "main", self.repo)
        git(self.repo, "config", "user.name", "Production incremental acceptance")
        git(self.repo, "config", "user.email", "incremental@example.invalid")
        for path, content in {
            "pkg/__init__.py": "",
            "pkg/core.py": "VALUE = 41\n",
            "pkg/mid.py": "import pkg.core\n",
            "pkg/top.py": "import pkg.mid\n",
            "pkg/unrelated.py": "VALUE = 9\n",
            "README.md": "Independent two-revision Git fixture.\n",
        }.items():
            self.write(path, content)
        self.base = self.commit("base")
        self.incremental_db = self.home / "incremental.sqlite"
        self.full_db = self.home / "fresh.sqlite"

    def write(self, path, content):
        dest = self.repo / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(content, encoding="utf-8")

    def commit(self, message):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", message)
        return git(self.repo, "rev-parse", "HEAD")

    def analyze(self, rev, db, env=None):
        run = call(
            BIN, "analyze", self.repo, "--rev", rev,
            "--roots", "pkg", "--db", db, "--json", env=env,
        )
        return json.loads(run.stdout)

    def snapshot(self, db, commit):
        with sqlite3.connect(db) as con:
            result = con.execute(
                "SELECT parsed_modules,reused_modules,producer_sha256,catalog_sha256,rows_sha256 "
                "FROM python_index_snapshot WHERE commit_oid=?", (commit,),
            ).fetchone()
        return result

    def compare_fresh(self, head, expected_parsed, expected_reused, env=None):
        base_report = self.analyze(self.base, self.incremental_db)
        self.assertEqual(base_report["source_commit"], self.base)
        old = self.snapshot(self.incremental_db, self.base)
        self.assertIsNotNone(old)
        self.assertEqual(old[:2], (5, 0))

        observed = self.analyze(head, self.incremental_db, env)
        fresh = self.analyze(head, self.full_db, env)
        self.assertEqual(observed, fresh)
        self.assertEqual(recorded(self.incremental_db, head),
                         recorded(self.full_db, head))
        with sqlite3.connect(self.incremental_db) as con:
            self.assertFalse(con.execute("PRAGMA foreign_key_check").fetchall())
        receipt = self.snapshot(self.incremental_db, head)
        self.assertIsNotNone(receipt)
        self.assertEqual(receipt[:2], (expected_parsed, expected_reused))
        self.assertEqual(receipt[2:], self.snapshot(self.full_db, head)[2:])
        self.assertEqual(observed, self.analyze(head, self.incremental_db, env))
        self.assertEqual(receipt, self.snapshot(self.incremental_db, head))
        self.assertEqual(git(self.repo, "status", "--porcelain"), "")
        return observed

    def test_changed_module_reuses_unchanged_symbols_imports_and_hazards(self):
        self.write("pkg/core.py", "VALUE = 42\n")
        head = self.commit("change Python value")
        result = self.compare_fresh(head, 1, 4)
        edges = {(e["source"]["path"], e["target"]["path"])
                 for e in result["relationships"]}
        self.assertIn(("pkg/mid.py", "pkg/core.py"), edges)
        self.assertIn(("pkg/top.py", "pkg/mid.py"), edges)
        self.assertNotIn(("pkg/unrelated.py", "pkg/core.py"), edges)

    def test_changed_importer_resolves_unchanged_target_from_full_catalog(self):
        self.write("pkg/mid.py", "from . import core\n")
        head = self.commit("change import syntax")
        report = self.compare_fresh(head, 1, 4)
        self.assertIn(
            ("pkg/mid.py", "pkg/core.py"),
            {(edge["source"]["path"], edge["target"]["path"])
             for edge in report["relationships"]},
        )

    def test_nonsource_commit_reuses_entire_catalog(self):
        self.write("README.md", "No Python change.\n")
        head = self.commit("change README")
        self.compare_fresh(head, 0, 5)

    def test_module_addition_invalidates_import_resolution(self):
        self.write("pkg/new.py", "import pkg.core\n")
        head = self.commit("new importable module")
        result = self.compare_fresh(head, 6, 0)
        self.assertTrue(any(
            edge["source"]["path"] == "pkg/new.py"
            and edge["target"]["path"] == "pkg/core.py"
            for edge in result["relationships"]
        ))

    def test_mode_only_change_forces_module_reparse(self):
        (self.repo / "pkg/core.py").chmod(0o755)
        git(self.repo, "add", "pkg/core.py")
        git(self.repo, "commit", "-qm", "change Python source mode")
        head = git(self.repo, "rev-parse", "HEAD")
        self.compare_fresh(head, 1, 4)

    def test_corrupted_parent_observation_is_refused(self):
        self.analyze(self.base, self.incremental_db)
        with sqlite3.connect(self.incremental_db) as con:
            con.execute(
                "DELETE FROM dependency WHERE source_name='pkg.mid'"
            )
            self.assertEqual(con.total_changes, 1)
        self.write("pkg/core.py", "VALUE = 43\n")
        head = self.commit("edit core after cache corruption")
        result = call(
            BIN, "analyze", self.repo, "--rev", head, "--roots", "pkg",
            "--db", self.incremental_db, "--json", check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stored Python index rows disagree", result.stderr)
        self.assertIsNone(self.snapshot(self.incremental_db, head))

    @unittest.skipUnless(os.name == "posix", "requires a POSIX interpreter wrapper")
    def test_interpreter_version_change_forces_reparse(self):
        wrapper = self.home / "python-wrapper"
        wrapper.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = \"--version\" ]; then echo 'Python 99.0'; "
            "else exec python3 \"$@\"; fi\n", encoding="utf-8",
        )
        wrapper.chmod(0o755)
        self.write("pkg/core.py", "VALUE = 44\n")
        head = self.commit("changed source after producer drift")
        env = dict(os.environ, EXCESSION_PYTHON=str(wrapper))
        self.compare_fresh(head, 5, 0, env=env)

    def test_same_revision_reanalysis_repairs_derived_row_tamper(self):
        original = self.analyze(self.base, self.incremental_db)
        with sqlite3.connect(self.incremental_db) as con:
            con.execute(
                "DELETE FROM dependency WHERE source_name='pkg.mid'"
            )
            self.assertEqual(con.total_changes, 1)
        # The refreshed main-branch architecture deliberately reruns the
        # parser outside a write transaction on repeated same-revision calls.
        self.assertEqual(
            original, self.analyze(self.base, self.incremental_db)
        )
        self.assertEqual(len(recorded(self.incremental_db, self.base)["dependency"]), 2)


if __name__ == "__main__":
    unittest.main()
