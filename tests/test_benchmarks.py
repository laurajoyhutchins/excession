"""Adversarial tests for the independent graph/view calibration harness."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("excession_benchmark_paired", ROOT / "benchmarks/paired.py")
assert spec and spec.loader
paired = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paired)


class GraphAndManifestUnitTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.db = Path(self.temp.name) / "graph.sqlite"
        with sqlite3.connect(self.db) as conn:
            conn.executescript("""
                CREATE TABLE repository (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                CREATE TABLE module(repo_id INTEGER, commit_oid TEXT, name TEXT, path TEXT, is_test INTEGER);
                CREATE TABLE dependency(repo_id INTEGER, commit_oid TEXT, source_name TEXT, target_name TEXT);
                INSERT INTO repository VALUES (1, 'fixture');
                INSERT INTO module VALUES (1,'pinned','p.a','p/a.py',0);
                INSERT INTO module VALUES (1,'pinned','p.b','p/b.py',0);
                INSERT INTO module VALUES (1,'pinned','p.c','p/c.py',0);
                INSERT INTO module VALUES (1,'pinned','p.d','p/d.py',0);
                INSERT INTO module VALUES (1,'pinned','tests.t','tests/t.py',1);
                INSERT INTO dependency VALUES (1,'pinned','p.a','p.b');
                INSERT INTO dependency VALUES (1,'pinned','p.b','p.c');
                INSERT INTO dependency VALUES (1,'pinned','p.c','p.a');
                INSERT INTO dependency VALUES (1,'pinned','p.d','p.a');
                INSERT INTO dependency VALUES (1,'pinned','tests.t','p.b');
            """)

    def paths(self, kind: str, seed: str = "p.a") -> list[str]:
        return paired.graph_paths(self.db, "fixture", "pinned", seed, kind)

    def test_cycles_dedup_and_shortest_distance(self) -> None:
        self.assertEqual(self.paths("focus"), ["p/a.py"])
        self.assertEqual(self.paths("dependencies_direct"), ["p/b.py"])
        self.assertEqual(self.paths("dependencies_all"), ["p/b.py", "p/c.py"])
        self.assertEqual(self.paths("dependents_direct"), ["p/c.py", "p/d.py"])
        self.assertEqual(self.paths("dependents_all"), ["p/b.py", "p/c.py", "p/d.py", "tests/t.py"])
        self.assertEqual(self.paths("tests"), ["tests/t.py"])

    def test_strict_revision_and_seed_boundaries(self) -> None:
        for repo, rev, seed in [("wrong", "pinned", "p.a"), ("fixture", "other", "p.a"), ("fixture", "pinned", "p.notreal")]:
            with self.subTest(repo=repo, rev=rev, seed=seed), self.assertRaises(ValueError):
                paired.graph_paths(self.db, repo, rev, seed, "focus")

    def test_unresolved_graph_edge_refused(self) -> None:
        with sqlite3.connect(self.db) as conn:
            conn.execute("INSERT INTO dependency VALUES(1,'pinned','p.a','external.invalid')")
        with self.assertRaisesRegex(ValueError, "unknown module"):
            self.paths("dependencies_all")

    def test_view_role_filtering_deduplicates_tests(self) -> None:
        manifest = {
            "schema": "excession.repository-view.v1", "kind": "dependencies",
            "source_commit": "pinned", "focus": "p.a", "entries": [
                {"role": "focus", "source_path": "p/a.py"},
                {"role": "direct-dependencies", "source_path": "p/b.py"},
                {"role": "transitive-dependencies", "source_path": "p/c.py"},
                {"role": "direct-dependents", "source_path": "p/c.py"},
                {"role": "transitive-dependents", "source_path": "tests/t.py"},
                {"role": "indexed-test", "source_path": "tests/t.py"},
                {"role": "indexed-test", "source_path": "tests/t.py"},
            ],
        }
        self.assertEqual(paired.view_paths(manifest, "pinned", "p.a", "dependencies_all"), ["p/b.py", "p/c.py"])
        self.assertEqual(paired.view_paths(manifest, "pinned", "p.a", "tests"), ["tests/t.py"])
        for key, invalid in [("focus", "p.other"), ("source_commit", "different"), ("kind", "security-flow")]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                paired.view_paths({**manifest, key: invalid}, "pinned", "p.a", "focus")

    def test_task_manifest_validated_not_silently_repaired(self) -> None:
        original = json.loads((ROOT / "benchmarks/calibration-h11.json").read_text())
        self.assertEqual(len(original["tasks"]), 24)
        file = Path(self.temp.name) / "tasks.json"
        file.write_text(json.dumps(original))
        self.assertEqual(len(paired.load_tasks(file)), 24)
        original["tasks"].append(original["tasks"][0])
        file.write_text(json.dumps(original))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            paired.load_tasks(file)
        original["tasks"].pop()
        original["tasks"][0]["expected_source_paths"] = ["../escape"]
        file.write_text(json.dumps(original))
        with self.assertRaisesRegex(ValueError, "invalid expected"):
            paired.load_tasks(file)

    def test_graph_worker_cli(self) -> None:
        run = subprocess.run([
            sys.executable, str(ROOT / "benchmarks/paired.py"), "graph-query",
            "--db", str(self.db), "--name", "fixture", "--revision", "pinned",
            "--seed", "p.a", "--kind", "dependencies_all",
        ], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(json.loads(run.stdout)["source_paths"], ["p/b.py", "p/c.py"])


    def test_measured_output_amplification_and_p95(self) -> None:
        graph = {"source_paths": ["p/b.py", "p/c.py"]}
        manifest = {"entries": [
            {"role": "focus", "source_path": "p/a.py"},
            {"role": "direct-dependencies", "source_path": "p/b.py"},
            {"role": "direct-dependents", "source_path": "p/c.py"},
            {"role": "indexed-test", "source_path": "p/c.py"},
        ]}
        self.assertEqual(paired.output_counts(graph, "graph", "dependencies_all"), {
            "emitted_references": 2, "emitted_unique_files": 2,
            "emitted_duplicate_references": 0, "selected_references": 2,
            "selected_duplicate_references": 0,
        })
        self.assertEqual(paired.output_counts(manifest, "view", "dependencies_all"), {
            "emitted_references": 4, "emitted_unique_files": 3,
            "emitted_duplicate_references": 1, "selected_references": 1,
            "selected_duplicate_references": 0,
        })
        self.assertEqual(paired.nearest_rank_p95([1.0, 5.0, 3.0, 2.0]), 5.0)

    def test_command_timeout_is_measured_and_not_silent(self) -> None:
        proc, timing = paired.command([sys.executable, "-c", "import time; time.sleep(0.5)"], 0.03)
        self.assertEqual(proc.returncode, 124)
        self.assertGreaterEqual(timing["wall_ms"], 25)


class RealExcessionSmoke(unittest.TestCase):
    """Runs the actual binary when built; never substitutes a fake view provider."""

    def test_two_arms_on_pinned_h11(self) -> None:
        binary = ROOT / "core/target/debug/excession"
        bundle = ROOT / "core/fixtures/h11-crossrepo.bundle"
        if not (binary.is_file() and bundle.is_file()):
            self.skipTest("real Excession binary or pinned Git bundle unavailable")
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            repo = temp / "h11"
            result = subprocess.run(["git", "clone", "-q", str(bundle), str(repo)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            corpus = json.loads((ROOT / "benchmarks/calibration-h11.json").read_text())
            corpus["tasks"] = corpus["tasks"][:2]
            tasks = temp / "small.json"
            tasks.write_text(json.dumps(corpus))
            out = temp / "receipts"
            result = subprocess.run([
                sys.executable, str(ROOT / "benchmarks/paired.py"), "run",
                "--repo", str(repo), "--bin", str(binary), "--tasks", str(tasks),
                "--out", str(out), "--name", "h11", "--roots", "h11,tests",
                "--repetitions", "1",
            ], capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 0, result.stderr)
            summary = json.loads((out / "summary.json").read_text())
            self.assertEqual(summary["source_parity_pairs"], 2)
            self.assertEqual(summary["total_pairs"], 2)
            self.assertEqual(len((out / "trials.jsonl").read_text().splitlines()), 6)
            self.assertEqual(summary["counts_by_status"]["graph"]["ok"], 2)
            self.assertEqual(summary["counts_by_status"]["rust_graph"]["ok"], 2)
            self.assertEqual(summary["counts_by_status"]["view"]["ok"], 2)
            self.assertIn("median_raw_stdout_bytes", summary)
            self.assertIn("median_duplicate_references", summary)
            trials = [json.loads(line) for line in (out / "trials.jsonl").read_text().splitlines()]
            self.assertTrue(all("raw_stdout_sha256" in r for r in trials))
            self.assertTrue(all("candidate_source_bytes" in r for r in trials))


if __name__ == "__main__":
    unittest.main()
