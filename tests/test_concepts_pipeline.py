"""Git-backed end-to-end acceptance for the conservative concepts pipeline.

This independently authenticates the source and receipt bytes. The receipt is
assembled for this fixture, not independently emitted by a language indexer.
Consequently none of these tests claims upstream parser correctness.
"""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def digest(value):
    return hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def run(*args, check=True):
    result = subprocess.run([str(x) for x in args], capture_output=True, timeout=30)
    if check and result.returncode:
        raise AssertionError(result.stderr.decode(errors="replace"))
    return result


def git(root, *args):
    return run("git", "-C", root, *args).stdout.decode().strip()


@unittest.skipUnless(BIN.is_file(), "Requires compiled Excession CLI")
class GitBoundConceptPipeline(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-concepts-tdd-")
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Concept acceptance")
        git(self.repo, "config", "user.email", "concepts@example.invalid")
        (self.repo / "a.py").write_text("from . import b\n")
        (self.repo / "b.py").write_text("from . import a\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "Pinned source")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        self.tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        self.db = Path(self.tmp.name) / "model.sqlite"
        self.report_path = Path(self.tmp.name) / "observations.json"

    def make_report(self, pairs=(("a.py", "b.py"), ("b.py", "a.py"))):
        records = []
        for source, target in pairs:
            record = {
                "kind": "IMPORTS",
                "subject": {
                    "path": source,
                    "blob": git(self.repo, "rev-parse", f"HEAD:{source}"),
                    "line": 1,
                    "language": "python",
                },
                "specifier": "." + target.removesuffix(".py"),
                "assessment": {
                    "status": "CANDIDATE",
                    "method": "SYNTAX_RESOLUTION",
                    "rule": "RELATIVE_PATH_CANDIDATE",
                },
                "condition_evaluation": "NOT_EVALUATED",
                "target": {
                    "path": target,
                    "blob": git(self.repo, "rev-parse", f"HEAD:{target}"),
                },
            }
            record["observation_id"] = digest(record)
            records.append(record)
        records.sort(key=canonical)
        report = {
            "schema": "excession.semantic-observations.v1",
            "source_commit": self.commit,
            "source_tree": self.tree,
            "syntax_report_id": "a" * 64,
            "resolution_report_id": "b" * 64,
            "completeness": "PARTIAL_CANDIDATES_ONLY",
            "record_count": len(records),
            "gap_count": 0,
            "records": records,
            "gaps": [],
        }
        report["report_id"] = digest(report)
        self.report_path.write_text(canonical(report))
        return report

    def cli(self, subcommand, *extra, check=True):
        result = run(
            BIN, "concepts", subcommand,
            "--repo", self.repo, "--report", self.report_path,
            *extra, check=check,
        )
        return json.loads(result.stdout) if check else result

    def test_reciprocal_imports_remain_unverified_proposals(self):
        self.make_report()
        batch = self.cli("normalize")
        self.assertEqual(batch["completeness"], "PARTIAL_CANDIDATES_ONLY")
        self.assertEqual(len(batch["observations"]), 2)
        model = self.cli("preview")
        self.assertEqual(len(model["concepts"]), 1)
        self.assertEqual(len(model["claims"]), 3)
        self.assertEqual(model["coverage"]["status"], "PARTIAL")
        self.assertEqual({c["assessment"] for c in model["claims"]}, {"PROPOSED"})
        self.assertEqual({c["behavioral_verification"] for c in model["claims"]}, {"UNTESTED"})
        self.assertEqual(
            len({e["observation_id"] for c in model["claims"] for e in c["support"]}), 2
        )
        manifest = self.cli("compile", "--db", self.db)
        same = self.cli("compile", "--db", self.db)
        self.assertEqual(same, manifest)
        audit = self.cli("audit", "--db", self.db, "--snapshot", manifest["snapshot_id"])
        self.assertEqual(audit["status"], "SOURCE_BINDINGS_RECHECKED")
        self.assertFalse(audit["producer_reexecuted"])
        with sqlite3.connect(self.db) as db:
            db.execute(
                "INSERT INTO concept_subject VALUES (?,?,?)",
                (manifest["snapshot_id"], "f" * 64, "{}"),
            )
        bad = self.cli("audit", "--db", self.db, "--snapshot", manifest["snapshot_id"], check=False)
        self.assertNotEqual(bad.returncode, 0)
        self.assertEqual(bad.stdout, b"")

    def test_no_false_concept_from_one_way_import(self):
        self.make_report(pairs=(("a.py", "b.py"),))
        model = self.cli("preview")
        self.assertEqual(model["concepts"], [])
        self.assertEqual(model["claims"], [])
        self.assertEqual(model["coverage"]["status"], "PARTIAL")

    def test_stale_blob_refused_even_with_new_canonical_receipts(self):
        report = self.make_report()
        tampered = copy.deepcopy(report)
        tampered["records"][0]["target"]["blob"] = "e" * 40
        record = tampered["records"][0]
        record.pop("observation_id")
        record["observation_id"] = digest(record)
        tampered.pop("report_id")
        tampered["report_id"] = digest(tampered)
        self.report_path.write_text(canonical(tampered))
        result = self.cli("normalize", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main()
