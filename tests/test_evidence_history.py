"""Native Git-history evidence transport: no agent, checkout reset, or merge required."""
from __future__ import annotations

import importlib.util
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BIN = Path(os.environ.get("EXCESSION_BIN", Path(__file__).resolve().parents[1] / "core/target/debug/excession"))
MODULE = Path(__file__).resolve().parents[1] / "tools/excession_slice.py"
spec = importlib.util.spec_from_file_location("excession_slice_history", MODULE)
assert spec is not None and spec.loader is not None
slices = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = slices
spec.loader.exec_module(slices)


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


@unittest.skipUnless(BIN.is_file(), "build Excession with cargo build --offline before history tests")
class HistoricalTransport(unittest.TestCase):
    def setUp(self) -> None:
        temp = tempfile.TemporaryDirectory(prefix="excession-history-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.name", "Historical Tests")
        git(self.repo, "config", "user.email", "test@example.invalid")
        self.write("pkg/__init__.py", "")
        self.write("pkg/math.py", "def value():\n    return 7\n")
        self.write("tests/__init__.py", "")
        self.write("tests/check.py", "from pkg.math import value\nassert value() == 7\n")
        self.write("irrelevant.py", "print('does not enter the slice')\n")
        self.commit("baseline")
        self.base = git(self.repo, "rev-parse", "HEAD")
        self.db = self.root / "evidence.sqlite"

    def write(self, path: str, content: str) -> None:
        dest = self.repo / path
        dest.parent.mkdir(exist_ok=True, parents=True)
        dest.write_text(content)

    def commit(self, message: str) -> str:
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", message)
        return git(self.repo, "rev-parse", "HEAD")

    def manifest(self, name: str, revision: str) -> tuple[Path, dict]:
        plan = slices.plan(self.repo, "tests/check.py", revision, history=True)
        dest = self.root / f"{name}.json"
        dest.write_text(json.dumps(plan))
        return dest, plan

    def exec(self, *args: str, ok: bool = True) -> dict:
        cmd = [str(BIN), *map(str, args)]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if ok:
            self.assertEqual(result.returncode, 0, f"{cmd!r}\n{result.stderr}")
            return json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0, f"incorrectly succeeded: {cmd!r}")
        self.assertIn("REFUSED", result.stderr)
        return {}

    def establish(self) -> tuple[Path, Path, dict]:
        manifest, source = self.manifest("source", self.base)
        materialized = self.root / "source-slice"
        slices.materialize(self.repo, source, materialized)
        receipt = slices.execute(self.repo, source, materialized, 10)
        self.assertEqual(receipt["status"], "OBSERVED_PASS")
        receipt_path = self.root / "receipt.json"
        receipt_path.write_text(json.dumps(receipt))
        self.exec("slice", "ingest", "--repo", self.repo, "--manifest", manifest,
                  "--receipt", receipt_path, "--db", self.db)
        self.exec("slice", "audit", "--repo", self.repo, "--manifest", manifest,
                  "--receipt", receipt_path, "--db", self.db)
        return manifest, receipt_path, receipt

    def transport(self, source: Path, target: Path, receipt: Path, audit: bool = False) -> dict:
        return self.exec("evidence", "transport-audit" if audit else "transport",
                         "--repo", self.repo, "--source-manifest", source,
                         "--target-manifest", target, "--receipt", receipt,
                         "--db", self.db)

    def test_history_and_divergent_branches_transport_without_agents(self) -> None:
        git(self.repo, "switch", "-qc", "docs-branch")
        self.write("docs/note.md", "words\n")
        docs = self.commit("historical docs only")
        git(self.repo, "switch", "-qc", "other-branch", self.base)
        self.write("irrelevant.py", "print('change')\n")
        other = self.commit("historical unrelated change")
        merged = git(self.repo, "merge", "-q", "--no-edit", "docs-branch")
        head = git(self.repo, "rev-parse", "HEAD")
        self.assertNotEqual(self.base, head)
        source, receipt, _ = self.establish()  # Establish historical evidence AFTER integration.
        for index, revision in enumerate((docs, other, head)):
            target, _ = self.manifest(f"target-{index}", revision)
            result = self.transport(source, target, receipt)
            self.assertEqual(result["decision"], "SOURCE_MATCH_ONLY")
            self.assertFalse(result["target_behavior_verified"])
            self.assertEqual(result["changed_source_paths"], [])
            self.assertEqual(result["source_commit"], self.base)
            self.assertEqual(result["target_commit"], revision)
            self.assertTrue(self.transport(source, target, receipt, audit=True)["independently_audited"])
        trace = self.exec("evidence", "trace", "--repo", self.repo,
                          "--receipt-id", json.loads(receipt.read_text())["receipt_id"],
                          "--db", self.db)
        self.assertEqual(trace["edge_count"], 3)
        self.assertTrue(trace["independently_audited"])
        self.assertEqual({edge["decision"] for edge in trace["edges"]}, {"SOURCE_MATCH_ONLY"})
        self.assertEqual(git(self.repo, "rev-parse", "HEAD"), head)

    def test_dependency_regression_invalidates_historical_evidence(self) -> None:
        self.write("pkg/math.py", "def value():\n    return 8\n")
        changed = self.commit("introduce regression")
        source, receipt, _ = self.establish()
        target, target_plan = self.manifest("regressed", changed)
        outcome = self.transport(source, target, receipt)
        self.assertEqual(outcome["decision"], "REVERIFY_REQUIRED")
        self.assertEqual(outcome["changed_source_paths"], ["pkg/math.py"])
        self.assertTrue(self.transport(source, target, receipt, audit=True)["independently_audited"])
        dest = self.root / "regressed-slice"
        slices.materialize(self.repo, target_plan, dest)
        fresh = slices.execute(self.repo, target_plan, dest, 10)
        self.assertEqual(fresh["status"], "OBSERVED_FAIL")
        self.assertFalse(outcome["target_behavior_verified"])

    def test_forged_target_and_sql_tampering_refused(self) -> None:
        self.write("docs/note.md", "unrelated\n")
        revision = self.commit("docs")
        source, receipt, _ = self.establish()
        target, target_plan = self.manifest("target", revision)
        outcome = self.transport(source, target, receipt)
        self.assertEqual(outcome["decision"], "SOURCE_MATCH_ONLY")
        forged = json.loads(json.dumps(target_plan))
        forged["entries"][0]["blob"] = "0" * 40
        forged["view_id"] = hashlib.sha256(json.dumps({k: v for k, v in forged.items() if k != "view_id"}, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        fake_path = self.root / "forged.json"
        fake_path.write_text(json.dumps(forged))
        self.exec("evidence", "transport", "--repo", self.repo, "--source-manifest", source,
                  "--target-manifest", fake_path, "--receipt", receipt, "--db", self.db, ok=False)
        import sqlite3
        with sqlite3.connect(self.db) as db:
            db.execute("UPDATE evidence_transport SET comparison_json='{}' WHERE transport_id=?",
                       (outcome["transport_id"],))
        self.exec("evidence", "transport-audit", "--repo", self.repo, "--source-manifest", source,
                  "--target-manifest", target, "--receipt", receipt, "--db", self.db, ok=False)
        self.exec("evidence", "trace", "--repo", self.repo,
                  "--receipt-id", json.loads(receipt.read_text())["receipt_id"],
                  "--db", self.db, ok=False)

    def test_absent_original_receipt_cannot_be_inherited(self) -> None:
        source, manifest = self.manifest("source", self.base)
        target, _ = self.manifest("target", self.base)
        fabricated_receipt = self.root / "never-ingested.json"
        fabricated_receipt.write_text(json.dumps({"schema": slices.RECEIPT, "commit": self.base,
                                                  "view_id": manifest["view_id"], "receipt_id": "0" * 64}))
        self.exec("evidence", "transport", "--repo", self.repo, "--source-manifest", source,
                  "--target-manifest", target, "--receipt", fabricated_receipt,
                  "--db", self.db, ok=False)


if __name__ == "__main__":
    unittest.main()
