"""Real-Git, source-bound repository layout projections. No mocked Git identities."""
from __future__ import annotations

import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/relgit"
BUNDLE = ROOT / "core/fixtures/h11-crossrepo.bundle"


def run(*args: object, ok: bool = True) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run([str(a) for a in args], text=True, capture_output=True, check=False)
    if ok and proc.returncode:
        raise AssertionError(f"{args}: exit {proc.returncode}; stderr={proc.stderr}")
    return proc


def source_step(path: str, line: int) -> dict:
    return {"physicalLocation": {"artifactLocation": {"uri": path}, "region": {"startLine": line}}}


class ProjectionIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not BIN.is_file():
            raise unittest.SkipTest("build Rust CLI before integration tests")
        cls.temp = tempfile.TemporaryDirectory(prefix="excession-projection-")
        cls.root = Path(cls.temp.name)
        cls.source = cls.root / "h11"
        run("git", "clone", "-q", BUNDLE, cls.source)
        cls.commit = run("git", "-C", cls.source, "rev-parse", "HEAD").stdout.strip()
        cls.tree = run("git", "-C", cls.source, "rev-parse", "HEAD^{tree}").stdout.strip()
        cls.catalog = cls.root / "index.sqlite"
        run(BIN, "index", cls.catalog, "h11", cls.source, "HEAD", "h11,tests")

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temp.cleanup()

    def setUp(self) -> None:
        self.temp_case = tempfile.TemporaryDirectory(prefix="excession-case-", dir=self.root)
        self.addCleanup(self.temp_case.cleanup)
        self.case = Path(self.temp_case.name)

    def plan_deps(self, seed: str = "h11._connection") -> tuple[dict, Path]:
        result = run(BIN, "view-plan-deps", self.catalog, "h11", "HEAD", seed)
        path = self.case / f"deps-{seed}.json"
        path.write_text(result.stdout)
        return json.loads(result.stdout), path

    def create_report(self, engine: str = "codeql") -> tuple[Path, Path, str]:
        report = self.case / "scanner.json"
        catalog = self.case / "security.sqlite"
        if engine == "codeql":
            first = source_step("h11/_connection.py", 20)
            flow_paths = [("h11/_connection.py", 20), ("h11/_events.py", 25), ("h11/_util.py", 20)]
            locations = [{"location": source_step(path, line)} for path, line in flow_paths]
            result = {
                "ruleId": "controlled/test-flow",
                "locations": [first],
                "codeFlows": [{"threadFlows": [{"locations": locations}]}],
            }
            data = {"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "CodeQL"}}, "results": [result]}]}
        else:
            data = {"version": "1.0.0", "errors": [], "results": [{
                "check_id": "controlled/review", "path": "h11/_connection.py", "start": {"line": 20},
            }]}
        report.write_text(json.dumps(data))
        receipt = json.loads(run(BIN, "security-ingest", self.source, report, catalog, engine).stdout)
        assert receipt["status"] == "UNVERIFIED_STATIC_CANDIDATES"
        run(BIN, "security-ingest-audit", self.source, report, catalog, engine)
        return report, catalog, receipt["run_id"]

    def plan_security(self, engine: str = "codeql") -> tuple[dict, Path, Path, str]:
        _, db, run_id = self.create_report(engine)
        output = run(BIN, "view-plan-security", db, self.source, run_id).stdout
        manifest = self.case / "security-view-plan.json"
        manifest.write_text(output)
        return json.loads(output), manifest, db, run_id

    def test_dependency_view_is_deterministic_and_source_identical(self):
        first, manifest = self.plan_deps()
        again = run(BIN, "view-plan-deps", self.catalog, "h11", "HEAD", "h11._connection").stdout
        self.assertEqual(manifest.read_text(), again)
        self.assertEqual(first["schema"], "excession.repository-view.v1")
        self.assertEqual((first["source_commit"], first["source_tree"]), (self.commit, self.tree))
        self.assertTrue(any(x["role"] == "indexed-test" for x in first["entries"]))
        self.assertEqual(first["entries"], sorted(first["entries"], key=lambda x: x["virtual_path"]))
        self.assertNotEqual(len({x["role"] for x in first["entries"]}), 1)
        view_dir = self.case / "read-only-view"
        receipt = json.loads(run(BIN, "view-materialize", self.source, manifest, view_dir).stdout)
        self.assertEqual(receipt["entries_verified"], len(first["entries"]))
        self.assertEqual(receipt["status"], "SOURCE_BLOBS_VERIFIED_NOT_EXECUTABLE")
        for entry in first["entries"]:
            view = view_dir / entry["virtual_path"]
            self.assertEqual(view.read_bytes(), run("git", "-C", self.source, "show", f"{self.commit}:{entry['source_path']}").stdout.encode())
            self.assertEqual(view.stat().st_mode & 0o777, 0o444)
        self.assertEqual(json.loads(run(BIN, "view-audit", self.source, manifest, view_dir).stdout), receipt)
        self.assertEqual(run("git", "-C", self.source, "status", "--porcelain").stdout, "")

    def test_focus_changes_layout_not_source_revision(self):
        a, _ = self.plan_deps("h11._connection")
        b, _ = self.plan_deps("h11._headers")
        self.assertNotEqual(a["view_id"], b["view_id"])
        self.assertEqual(a["source_commit"], b["source_commit"])
        self.assertNotEqual([e["virtual_path"] for e in a["entries"]], [e["virtual_path"] for e in b["entries"]])

    def test_dependency_query_replay_refuses_missing_relations(self):
        first, manifest = self.plan_deps()
        receipt = json.loads(run(BIN, "view-plan-replay-deps", self.catalog, "h11", "HEAD", "h11._connection", manifest).stdout)
        self.assertEqual(receipt["view_id"], first["view_id"])
        with sqlite3.connect(self.catalog) as db:
            rows = db.execute("SELECT source_name,target_name,line,syntax FROM dependency WHERE source_name='h11._connection'").fetchall()
            self.assertTrue(rows)
            db.execute("DELETE FROM dependency WHERE source_name='h11._connection'")
        try:
            self.assertNotEqual(run(BIN, "view-plan-replay-deps", self.catalog, "h11", "HEAD", "h11._connection", manifest, ok=False).returncode, 0)
        finally:
            # Restore the indexed relations so the following tests see the same source.
            with sqlite3.connect(self.catalog) as db:
                db.executemany("INSERT INTO dependency(repo_id,commit_oid,source_name,target_name,line,syntax) VALUES((SELECT id FROM repository WHERE name='h11'),?,?,?,?,?)", [(self.commit,*row) for row in rows])

    def test_codeql_layout_uses_reported_flow_positions(self):
        planned, manifest, catalog, run_id = self.plan_security()
        self.assertEqual(planned["kind"], "security-flow")
        self.assertEqual(len(planned["entries"]), 4)
        self.assertEqual({e["role"] for e in planned["entries"]}, {
            "unverified-static-candidate", "scanner-flow-start", "scanner-flow-intermediate", "scanner-flow-end",
        })
        self.assertEqual(planned["source_commit"], self.commit)
        view_dir = self.case / "security-tree"
        output = json.loads(run(BIN, "view-materialize", self.source, manifest, view_dir).stdout)
        self.assertEqual(output["entries_verified"], 4)
        run(BIN, "view-audit", self.source, manifest, view_dir)
        replay = json.loads(run(BIN, "view-plan-replay-security", catalog, self.source, run_id, manifest).stdout)
        self.assertEqual(replay["view_id"], planned["view_id"])

    def test_semgrep_candidate_without_flow_stays_unverified(self):
        planned, manifest, _, _ = self.plan_security("semgrep")
        self.assertEqual(len(planned["entries"]), 1)
        self.assertEqual(planned["entries"][0]["role"], "unverified-static-candidate")
        run(BIN, "view-materialize", self.source, manifest, self.case / "semgrep-layout")

    def test_audit_refuses_modified_or_missing_file(self):
        planned, manifest = self.plan_deps()
        dest = self.case / "view"
        run(BIN, "view-materialize", self.source, manifest, dest)
        target = dest / planned["entries"][0]["virtual_path"]
        target.chmod(0o644)
        target.write_text("tampered\n")
        self.assertNotEqual(run(BIN, "view-audit", self.source, manifest, dest, ok=False).returncode, 0)
        target.unlink()
        self.assertNotEqual(run(BIN, "view-audit", self.source, manifest, dest, ok=False).returncode, 0)

    def test_audit_refuses_extra_files_and_symlink(self):
        planned, manifest = self.plan_deps()
        dest = self.case / "view"
        run(BIN, "view-materialize", self.source, manifest, dest)
        extra = dest / "views" / "untracked.txt"
        extra.write_text("not in manifest")
        self.assertNotEqual(run(BIN, "view-audit", self.source, manifest, dest, ok=False).returncode, 0)
        extra.unlink()
        target = dest / planned["entries"][0]["virtual_path"]
        target.unlink()
        target.symlink_to(self.source / planned["entries"][0]["source_path"])
        self.assertNotEqual(run(BIN, "view-audit", self.source, manifest, dest, ok=False).returncode, 0)

    def test_audit_refuses_manifest_tampering(self):
        planned, manifest = self.plan_deps()
        dest = self.case / "view"
        run(BIN, "view-materialize", self.source, manifest, dest)
        planned["entries"][0]["source_path"] = "h11/_abnf.py"
        forged = self.case / "forged.json"
        forged.write_text(json.dumps(planned))
        self.assertNotEqual(run(BIN, "view-audit", self.source, forged, dest, ok=False).returncode, 0)

    def test_materializer_refuses_existing_and_in_tree_output(self):
        _, manifest = self.plan_deps()
        dest = self.case / "view"
        dest.mkdir()
        self.assertNotEqual(run(BIN, "view-materialize", self.source, manifest, dest, ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "view-materialize", self.source, manifest, self.source / "new-view", ok=False).returncode, 0)
        self.assertFalse((self.source / "new-view").exists())
        self.assertNotEqual(run(BIN, "view-materialize", self.source, manifest, "relative-view", ok=False).returncode, 0)

    def test_security_query_replay_refuses_evidence_row_deletion(self):
        _, manifest, catalog, run_id = self.plan_security()
        with sqlite3.connect(catalog) as db:
            db.execute("DELETE FROM security_ingest_flow_step WHERE run_id=? AND step_index=1", (run_id,))
        self.assertNotEqual(run(BIN, "view-plan-replay-security", catalog, self.source, run_id, manifest, ok=False).returncode, 0)

    def test_does_not_require_clean_working_tree_to_render_pinned_git(self):
        # Materialization reads from the commit object, not the mutable working tree.
        _, manifest = self.plan_deps()
        dirty = self.source / "untracked-temporary.txt"
        dirty.write_text("not in Git")
        try:
            target = self.case / "pinned"
            run(BIN, "view-materialize", self.source, manifest, target)
            run(BIN, "view-audit", self.source, manifest, target)
        finally:
            dirty.unlink()


if __name__ == "__main__":
    unittest.main()
