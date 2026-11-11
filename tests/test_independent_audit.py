"""Independent Git/SQLite dependency query replay and corruption controls."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "benchmarks/experiments/independent_audit.py"
spec = importlib.util.spec_from_file_location("independent_audit", SCRIPT)
assert spec and spec.loader
auditor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auditor)


def command(*argv: object) -> str:
    return subprocess.check_output([str(x) for x in argv], text=True).strip()


class FullQueryReplayReferenceTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.repo = Path(self.scratch.name) / "repo"
        self.repo.mkdir()
        command("git", "init", "-q", self.repo)
        (self.repo / "pkg").mkdir()
        for name, value in {"__init__.py": "\n", "a.py": "import pkg.b\n",
                            "b.py": "answer=1\n"}.items():
            (self.repo / "pkg" / name).write_text(value)
        command("git", "-C", self.repo, "add", "pkg")
        command("git", "-C", self.repo, "-c", "user.name=Benchmark",
                "-c", "user.email=benchmark@example.invalid",
                "commit", "-qm", "fixture")
        self.commit = command("git", "-C", self.repo, "rev-parse", "HEAD")
        self.tree = command("git", "-C", self.repo, "rev-parse", "HEAD^{tree}")
        self.db = Path(self.scratch.name) / "index.sqlite"
        self.manifest = Path(self.scratch.name) / "manifest.json"
        entries = auditor.git_entries(self.repo, self.commit)
        with sqlite3.connect(self.db) as con:
            con.executescript("""
                CREATE TABLE repository(id INTEGER PRIMARY KEY,name TEXT,git_dir TEXT);
                CREATE TABLE revision(repo_id INTEGER,commit_oid TEXT,tree_oid TEXT);
                CREATE TABLE tree_entry(repo_id INTEGER,commit_oid TEXT,path TEXT,
                                        mode TEXT,kind TEXT,oid TEXT);
                CREATE TABLE module(repo_id INTEGER,commit_oid TEXT,name TEXT,
                                    path TEXT,blob_oid TEXT,is_test INTEGER);
                CREATE TABLE dependency(repo_id INTEGER,commit_oid TEXT,
                                        source_name TEXT,target_name TEXT);
            """)
            con.execute("INSERT INTO repository VALUES(1,'pkg',?)", (str(self.repo),))
            con.execute("INSERT INTO revision VALUES(1,?,?)", (self.commit, self.tree))
            for path, (mode, typ, oid) in entries.items():
                con.execute("INSERT INTO tree_entry VALUES(1,?,?,?,?,?)",
                            (self.commit, path, mode, typ, oid))
                if typ == "blob" and path.endswith(".py"):
                    name = "pkg" if path.endswith("/__init__.py") else "pkg." + Path(path).stem
                    con.execute("INSERT INTO module VALUES(1,?,?,?,?,0)",
                                (self.commit, name, path, oid))
            con.execute("INSERT INTO dependency VALUES(1,?,'pkg.a','pkg.b')", (self.commit,))
        focus = entries["pkg/a.py"]
        dependency = entries["pkg/b.py"]
        payload = {
            "schema": auditor.FORMAT, "kind": "dependencies",
            "source_commit": self.commit, "source_tree": self.tree,
            "focus": "pkg.a",
            "entries": [
                {"virtual_path": "views/00-focus/pkg/a.py",
                 "source_path": "pkg/a.py", "mode": focus[0], "blob": focus[2],
                 "role": "focus", "detail": "module=pkg.a"},
                {"virtual_path": "views/10-dependencies/direct/pkg/b.py",
                 "source_path": "pkg/b.py", "mode": dependency[0], "blob": dependency[2],
                 "role": "direct-dependencies", "detail": "module=pkg.b;depth=1"},
            ],
        }
        self.payload = payload
        self.manifest.write_text(json.dumps({
            **payload, "view_id": auditor.signature(payload)
        }))

    def check(self, db=None, manifest=None):
        return auditor.independently_replay(
            self.repo, db or self.db, "pkg", self.commit, "pkg.a",
            manifest or self.manifest,
        )

    def test_valid_full_query(self):
        report = self.check()
        self.assertEqual(report["entries_replayed"], 2)
        self.assertEqual(report["status"], "QUERY_REPLAY_MATCHED")

    def test_modified_full_query_with_recomputed_hash_is_refused(self):
        for mutation in ("different_blob", "missing_entry", "wrong_role",
                         "stale_commit", "invalid_commitment"):
            with self.subTest(mutation=mutation):
                target = Path(self.scratch.name) / (mutation + ".json")
                auditor.altered_manifest(
                    json.loads(self.manifest.read_text()), target, mutation
                )
                with self.assertRaises(auditor.Refused):
                    self.check(manifest=target)

    def test_unrelated_git_tree_row_is_also_verified(self):
        with sqlite3.connect(self.db) as con:
            con.execute("UPDATE tree_entry SET mode='100755' WHERE path='pkg/__init__.py'")
        with self.assertRaisesRegex(auditor.Refused, "indexed tree"):
            self.check()

    def test_sql_module_binding_and_dependency_are_checked(self):
        with sqlite3.connect(self.db) as con:
            con.execute(
                "UPDATE module SET blob_oid='0000000000000000000000000000000000000000' "
                "WHERE name='pkg.a'")
        with self.assertRaisesRegex(auditor.Refused, "module binding"):
            self.check()

    def test_duplicate_json_keys_refused(self):
        path = Path(self.scratch.name) / "duplicate.json"
        value = self.manifest.read_text()
        path.write_text(value.replace('"focus":', '"focus":"pkg.a","focus":', 1))
        with self.assertRaises(auditor.Refused):
            auditor.read_manifest(path)


if __name__ == "__main__":
    unittest.main()
