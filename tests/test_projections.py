"""Real-Git, source-bound repository layout projections. No mocked Git identities."""
from __future__ import annotations

import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"
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
        run(BIN, "research", "index", cls.catalog, "h11", cls.source, "HEAD", "h11,tests")

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temp.cleanup()

    def setUp(self) -> None:
        self.temp_case = tempfile.TemporaryDirectory(prefix="excession-case-", dir=self.root)
        self.addCleanup(self.temp_case.cleanup)
        self.case = Path(self.temp_case.name)

    def plan_deps(self, seed: str = "h11._connection") -> tuple[dict, Path]:
        result = run(BIN, "research", "view-plan-deps", self.catalog, "h11", "HEAD", seed)
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
        receipt = json.loads(run(BIN, "research", "security-ingest", self.source, report, catalog, engine).stdout)
        assert receipt["status"] == "UNVERIFIED_STATIC_CANDIDATES"
        run(BIN, "research", "security-ingest-audit", self.source, report, catalog, engine)
        return report, catalog, receipt["run_id"]

    def plan_security(self, engine: str = "codeql") -> tuple[dict, Path, Path, str]:
        _, db, run_id = self.create_report(engine)
        output = run(BIN, "research", "view-plan-security", db, self.source, run_id).stdout
        manifest = self.case / "security-view-plan.json"
        manifest.write_text(output)
        return json.loads(output), manifest, db, run_id

    def test_dependency_view_is_deterministic_and_source_identical(self):
        first, manifest = self.plan_deps()
        again = run(BIN, "research", "view-plan-deps", self.catalog, "h11", "HEAD", "h11._connection").stdout
        self.assertEqual(manifest.read_text(), again)
        self.assertEqual(first["schema"], "excession.repository-view.v1")
        self.assertEqual((first["source_commit"], first["source_tree"]), (self.commit, self.tree))
        self.assertTrue(any(x["role"] == "indexed-test" for x in first["entries"]))
        self.assertEqual(first["entries"], sorted(first["entries"], key=lambda x: x["virtual_path"]))
        self.assertNotEqual(len({x["role"] for x in first["entries"]}), 1)
        view_dir = self.case / "read-only-view"
        receipt = json.loads(run(BIN, "research", "view-materialize", self.source, manifest, view_dir).stdout)
        self.assertEqual(receipt["entries_verified"], len(first["entries"]))
        self.assertEqual(receipt["status"], "SOURCE_BLOBS_VERIFIED_NOT_EXECUTABLE")
        for entry in first["entries"]:
            view = view_dir / entry["virtual_path"]
            self.assertEqual(view.read_bytes(), run("git", "-C", self.source, "show", f"{self.commit}:{entry['source_path']}").stdout.encode())
            self.assertEqual(view.stat().st_mode & 0o777, 0o444)
        self.assertEqual(json.loads(run(BIN, "research", "view-audit", self.source, manifest, view_dir).stdout), receipt)
        self.assertEqual(run("git", "-C", self.source, "status", "--porcelain").stdout, "")

    def test_focus_changes_layout_not_source_revision(self):
        a, _ = self.plan_deps("h11._connection")
        b, _ = self.plan_deps("h11._headers")
        self.assertNotEqual(a["view_id"], b["view_id"])
        self.assertEqual(a["source_commit"], b["source_commit"])
        self.assertNotEqual([e["virtual_path"] for e in a["entries"]], [e["virtual_path"] for e in b["entries"]])

    def test_dependency_query_replay_refuses_missing_relations(self):
        first, manifest = self.plan_deps()
        receipt = json.loads(run(BIN, "research", "view-plan-replay-deps", self.catalog, "h11", "HEAD", "h11._connection", manifest).stdout)
        self.assertEqual(receipt["view_id"], first["view_id"])
        with sqlite3.connect(self.catalog) as db:
            rows = db.execute("SELECT source_name,target_name,line,syntax FROM dependency WHERE source_name='h11._connection'").fetchall()
            self.assertTrue(rows)
            db.execute("DELETE FROM dependency WHERE source_name='h11._connection'")
        try:
            self.assertNotEqual(run(BIN, "research", "view-plan-replay-deps", self.catalog, "h11", "HEAD", "h11._connection", manifest, ok=False).returncode, 0)
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
        output = json.loads(run(BIN, "research", "view-materialize", self.source, manifest, view_dir).stdout)
        self.assertEqual(output["entries_verified"], 4)
        run(BIN, "research", "view-audit", self.source, manifest, view_dir)
        replay = json.loads(run(BIN, "research", "view-plan-replay-security", catalog, self.source, run_id, manifest).stdout)
        self.assertEqual(replay["view_id"], planned["view_id"])

    def test_semgrep_candidate_without_flow_stays_unverified(self):
        planned, manifest, _, _ = self.plan_security("semgrep")
        self.assertEqual(len(planned["entries"]), 1)
        self.assertEqual(planned["entries"][0]["role"], "unverified-static-candidate")
        run(BIN, "research", "view-materialize", self.source, manifest, self.case / "semgrep-layout")

    def test_audit_refuses_modified_or_missing_file(self):
        planned, manifest = self.plan_deps()
        dest = self.case / "view"
        run(BIN, "research", "view-materialize", self.source, manifest, dest)
        target = dest / planned["entries"][0]["virtual_path"]
        target.chmod(0o644)
        target.write_text("tampered\n")
        self.assertNotEqual(run(BIN, "research", "view-audit", self.source, manifest, dest, ok=False).returncode, 0)
        target.unlink()
        self.assertNotEqual(run(BIN, "research", "view-audit", self.source, manifest, dest, ok=False).returncode, 0)

    def test_audit_refuses_extra_files_and_symlink(self):
        planned, manifest = self.plan_deps()
        dest = self.case / "view"
        run(BIN, "research", "view-materialize", self.source, manifest, dest)
        extra = dest / "views" / "untracked.txt"
        extra.write_text("not in manifest")
        self.assertNotEqual(run(BIN, "research", "view-audit", self.source, manifest, dest, ok=False).returncode, 0)
        extra.unlink()
        target = dest / planned["entries"][0]["virtual_path"]
        target.unlink()
        target.symlink_to(self.source / planned["entries"][0]["source_path"])
        self.assertNotEqual(run(BIN, "research", "view-audit", self.source, manifest, dest, ok=False).returncode, 0)

    def test_audit_refuses_manifest_tampering(self):
        planned, manifest = self.plan_deps()
        dest = self.case / "view"
        run(BIN, "research", "view-materialize", self.source, manifest, dest)
        planned["entries"][0]["source_path"] = "h11/_abnf.py"
        forged = self.case / "forged.json"
        forged.write_text(json.dumps(planned))
        self.assertNotEqual(run(BIN, "research", "view-audit", self.source, forged, dest, ok=False).returncode, 0)

    def test_materializer_refuses_existing_and_in_tree_output(self):
        _, manifest = self.plan_deps()
        dest = self.case / "view"
        dest.mkdir()
        self.assertNotEqual(run(BIN, "research", "view-materialize", self.source, manifest, dest, ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-materialize", self.source, manifest, self.source / "new-view", ok=False).returncode, 0)
        self.assertFalse((self.source / "new-view").exists())
        self.assertNotEqual(run(BIN, "research", "view-materialize", self.source, manifest, "relative-view", ok=False).returncode, 0)

    def test_security_query_replay_refuses_evidence_row_deletion(self):
        _, manifest, catalog, run_id = self.plan_security()
        with sqlite3.connect(catalog) as db:
            db.execute("DELETE FROM security_ingest_flow_step WHERE run_id=? AND step_index=1", (run_id,))
        self.assertNotEqual(run(BIN, "research", "view-plan-replay-security", catalog, self.source, run_id, manifest, ok=False).returncode, 0)

    def test_does_not_require_clean_working_tree_to_render_pinned_git(self):
        # Materialization reads from the commit object, not the mutable working tree.
        _, manifest = self.plan_deps()
        dirty = self.source / "untracked-temporary.txt"
        dirty.write_text("not in Git")
        try:
            target = self.case / "pinned"
            run(BIN, "research", "view-materialize", self.source, manifest, target)
            run(BIN, "research", "view-audit", self.source, manifest, target)
        finally:
            dirty.unlink()

    def test_virtual_navigation_reads_exact_git_objects_without_copies(self):
        dependency, dep_manifest = self.plan_deps()
        listing = json.loads(run(BIN, "research", "view-ls", self.source, dep_manifest).stdout)
        self.assertEqual([c["name"] for c in listing["children"]], ["views"])
        groups = json.loads(run(BIN, "research", "view-ls", self.source, dep_manifest, "views").stdout)
        self.assertIn("00-focus", {c["name"] for c in groups["children"]})
        self.assertTrue(all(c["type"] == "directory" for c in groups["children"]))

        entry = next(e for e in dependency["entries"] if e["role"] == "focus")
        parent = str(Path(entry["virtual_path"]).parent)
        leaf = Path(entry["virtual_path"]).name
        children = json.loads(run(BIN, "research", "view-ls", self.source, dep_manifest, parent).stdout)
        self.assertEqual(children["children"], [{"name": leaf, "type": "file", "references": 1}])
        describe = json.loads(run(BIN, "research", "view-stat", self.source, dep_manifest, entry["virtual_path"]).stdout)
        self.assertEqual(describe["entry"]["source_path"], entry["source_path"])
        self.assertEqual(describe["entry"]["blob"], entry["blob"])
        self.assertEqual(describe["source_commit"], self.commit)
        contents = run(BIN, "research", "view-open", self.source, dep_manifest, entry["virtual_path"]).stdout
        original = run("git", "-C", self.source, "show", f"{self.commit}:{entry['source_path']}").stdout
        self.assertEqual(contents, original)
        self.assertEqual(run("git", "-C", self.source, "status", "--porcelain").stdout, "")
        self.assertFalse(any(self.case.glob("*view")))

    def test_composed_views_are_deterministic_and_replayable(self):
        dependency, dep_manifest = self.plan_deps()
        security, sec_manifest, _, _ = self.plan_security()
        output = run(BIN, "research", "view-compose", self.source, "union", dep_manifest, sec_manifest).stdout
        self.assertEqual(output, run(BIN, "research", "view-compose", self.source, "union", dep_manifest, sec_manifest).stdout)
        composed = json.loads(output)
        self.assertEqual(composed["kind"], "composed")
        self.assertEqual(composed["source_commit"], self.commit)
        self.assertEqual(len(composed["entries"]), len(dependency["entries"]) + len(security["entries"]))
        path = self.case / "union.json"
        path.write_text(output)
        replay = json.loads(run(BIN, "research", "view-compose-replay", self.source, "union", dep_manifest, sec_manifest, path).stdout)
        self.assertEqual(replay["view_id"], composed["view_id"])
        # The union can be selected and queried repeatedly without ever being copied.
        selected = json.loads(run(BIN, "research", "view-select", self.source, path, "role", "scanner-flow-start").stdout)
        self.assertEqual(len(selected["entries"]), 1)
        self.assertEqual(selected["entries"][0]["role"], "scanner-flow-start")
        filtered = self.case / "filtered.json"
        filtered.write_text(json.dumps(selected))
        run(BIN, "research", "view-select-replay", self.source, path, "role", "scanner-flow-start", filtered)
        self.assertEqual(json.loads(run(BIN, "research", "view-ls", self.source, filtered).stdout)["children"][0]["name"], "views")
        plan = self.case / "union-physical"
        run(BIN, "research", "view-materialize", self.source, path, plan)
        run(BIN, "research", "view-audit", self.source, path, plan)

    def test_intersection_and_difference_use_source_identity(self):
        dep, manifest = self.plan_deps()
        focus = json.loads(run(BIN, "research", "view-select", self.source, manifest, "role", "focus").stdout)
        focus_file = self.case / "focus.json"
        focus_file.write_text(json.dumps(focus))
        overlap = json.loads(run(BIN, "research", "view-compose", self.source, "intersection", manifest, focus_file).stdout)
        self.assertEqual({(e["source_path"], e["blob"]) for e in overlap["entries"]},
                         {(e["source_path"], e["blob"]) for e in focus["entries"]})
        rest = json.loads(run(BIN, "research", "view-compose", self.source, "difference", manifest, focus_file).stdout)
        focus_sources = {(e["source_path"], e["blob"]) for e in focus["entries"]}
        self.assertTrue(all((e["source_path"], e["blob"]) not in focus_sources for e in rest["entries"]))
        self.assertEqual(
            {(e["virtual_path"], e["source_path"]) for e in dep["entries"]},
            {(e["virtual_path"], e["source_path"]) for e in overlap["entries"] + rest["entries"]},
        )
        narrowed = json.loads(run(BIN, "research", "view-select", self.source, manifest, "source-prefix", "h11").stdout)
        self.assertTrue(all(entry["source_path"].startswith("h11/") for entry in narrowed["entries"]))
        self.assertTrue(0 < len(narrowed["entries"]) < len(dep["entries"]))
        narrowed = json.loads(run(BIN, "research", "view-select", self.source, manifest, "virtual-prefix", "views/00-focus").stdout)
        self.assertEqual(len(narrowed["entries"]), 1)

    def test_query_replay_rejects_changed_inputs_and_output(self):
        _, dep = self.plan_deps()
        _, sec, _, _ = self.plan_security()
        expected = json.loads(run(BIN, "research", "view-compose", self.source, "union", dep, sec).stdout)
        out = self.case / "combined.json"
        out.write_text(json.dumps(expected))
        self.assertNotEqual(run(BIN, "research", "view-compose-replay", self.source, "difference", dep, sec, out, ok=False).returncode, 0)
        edited = json.loads(dep.read_text())
        edited["entries"][0]["role"] = "fabricated-role"
        edited.pop("view_id")
        import hashlib
        edited["view_id"] = hashlib.sha256(json.dumps(edited, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        forged = self.case / "forged-input.json"
        forged.write_text(json.dumps(edited))
        self.assertNotEqual(run(BIN, "research", "view-compose-replay", self.source, "union", forged, sec, out, ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-select-replay", self.source, dep, "role", "focus", out, ok=False).returncode, 0)

    def test_virtual_view_fail_closed_controls(self):
        dep, manifest = self.plan_deps()
        self.assertNotEqual(run(BIN, "research", "view-compose", self.source, "xor", manifest, manifest, ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-select", self.source, manifest, "role", "does-not-exist", ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-select", self.source, manifest, "source-prefix", "../bad", ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-ls", self.source, manifest, "views/../bad", ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-ls", self.source, manifest, "views/nonexistent", ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-open", self.source, manifest, "views/../../etc/passwd", ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-stat", self.source, manifest, "views/not-a-file", ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-compose", self.source, "difference", manifest, manifest, ok=False).returncode, 0)
        focus = json.loads(run(BIN, "research", "view-select", self.source, manifest, "role", "focus").stdout)
        tests = json.loads(run(BIN, "research", "view-select", self.source, manifest, "role", "indexed-test").stdout)
        focus_path = self.case / "focus-only.json"
        tests_path = self.case / "tests-only.json"
        focus_path.write_text(json.dumps(focus))
        tests_path.write_text(json.dumps(tests))
        self.assertNotEqual(run(BIN, "research", "view-compose", self.source, "intersection", focus_path,
                                tests_path, ok=False).returncode, 0)

    def test_composition_refuses_mismatched_git_revision(self):
        _, first = self.plan_deps()
        alternate = self.case / "different-revision-repo"
        run("git", "clone", "-q", BUNDLE, alternate)
        changed = alternate / "h11/_util.py"
        changed.write_bytes(changed.read_bytes() + b"\n# alternate revision\n")
        run("git", "-C", alternate, "add", "h11/_util.py")
        run("git", "-C", alternate, "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "alternate source")
        database = self.case / "alternate-index.sqlite"
        run(BIN, "research", "index", database, "alternate", alternate, "HEAD", "h11,tests")
        result = run(BIN, "research", "view-plan-deps", database, "alternate", "HEAD", "h11._connection")
        manifest = self.case / "different.json"
        manifest.write_text(result.stdout)
        self.assertNotEqual(run(BIN, "research", "view-compose", self.source, "union", first, manifest, ok=False).returncode, 0)
        self.assertNotEqual(run(BIN, "research", "view-compose", alternate, "union", first, manifest, ok=False).returncode, 0)

    def test_union_conflict_and_file_directory_collision_are_refused(self):
        _, manifest = self.plan_deps()
        modified = json.loads(manifest.read_text())
        modified["entries"][0]["role"] = "different-role"
        modified.pop("view_id")
        import hashlib
        modified["view_id"] = hashlib.sha256(json.dumps(modified, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        changed = self.case / "changed-role.json"
        changed.write_text(json.dumps(modified))
        self.assertNotEqual(run(BIN, "research", "view-compose", self.source, "union", manifest, changed, ok=False).returncode, 0)
        # Add a valid source reference with a virtual pathname that is itself the
        # parent directory of another virtual file, and recompute its view ID.
        malformed = json.loads(manifest.read_text())
        extra = dict(malformed["entries"][0])
        extra["virtual_path"] = "views/00-focus"
        malformed["entries"].append(extra)
        malformed["entries"].sort(key=lambda e: e["virtual_path"])
        malformed.pop("view_id")
        malformed["view_id"] = hashlib.sha256(json.dumps(malformed, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        bad = self.case / "file-directory.json"
        bad.write_text(json.dumps(malformed))
        self.assertNotEqual(run(BIN, "research", "view-ls", self.source, bad, "views", ok=False).returncode, 0)

    def test_virtual_browser_navigates_without_materialization(self):
        _, manifest = self.plan_deps()
        commands = "ls\ncd views\nls\ncd 00-focus\npwd\nls\nquit\n"
        completed = subprocess.run([
            "python3", ROOT / "tools/view_browser.py", self.source, manifest,
            "--rust", BIN,
        ], input=commands, text=True, capture_output=True, check=False, timeout=15)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("'views'/", completed.stdout)
        self.assertIn("'00-focus'/", completed.stdout)
        self.assertIn("/views/00-focus", completed.stdout)
        self.assertFalse((self.source / "views").exists())
        invalid = subprocess.run([
            "python3", ROOT / "tools/view_browser.py", self.source, manifest,
            "--rust", BIN,
        ], input="cd ..\n", text=True, capture_output=True, check=False, timeout=15)
        self.assertNotEqual(invalid.returncode, 0)
        self.assertIn("above virtual root", invalid.stderr)


    def test_namespaced_mounts_allow_distinct_focus_layouts_to_coexist(self):
        left, left_manifest = self.plan_deps("h11._connection")
        right, right_manifest = self.plan_deps("h11._headers")
        left_view = json.loads(run(BIN, "research", "view-mount", self.source, left_manifest, "connection").stdout)
        right_view = json.loads(run(BIN, "research", "view-mount", self.source, right_manifest, "headers").stdout)
        self.assertEqual(len(left_view["entries"]), len(left["entries"]))
        self.assertEqual(len(right_view["entries"]), len(right["entries"]))
        self.assertTrue(all(e["virtual_path"].startswith("views/connection/") for e in left_view["entries"]))
        self.assertTrue(all(e["virtual_path"].startswith("views/headers/") for e in right_view["entries"]))
        a = self.case / "connection.json"
        b = self.case / "headers.json"
        a.write_text(json.dumps(left_view))
        b.write_text(json.dumps(right_view))
        union = json.loads(run(BIN, "research", "view-compose", self.source, "union", a, b).stdout)
        self.assertEqual(len(union["entries"]), len(left["entries"]) + len(right["entries"]))
        composed = self.case / "both.json"
        composed.write_text(json.dumps(union))
        run(BIN, "research", "view-compose-replay", self.source, "union", a, b, composed)
        run(BIN, "research", "view-mount-replay", self.source, left_manifest, "connection", a)
        branches = json.loads(run(BIN, "research", "view-ls", self.source, composed, "views").stdout)
        self.assertEqual({item["name"] for item in branches["children"]}, {"connection", "headers"})
        for namespace in ("../bad", "slash/not-allowed", "", "control\nchar"):
            self.assertNotEqual(
                run(BIN, "research", "view-mount", self.source, left_manifest, namespace, ok=False).returncode,
                0,
            )



if __name__ == "__main__":
    unittest.main()
