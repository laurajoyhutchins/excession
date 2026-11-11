"""Executable contracts for independently specified relationship workloads."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "benchmarks/experiments/relationships.py"
spec = importlib.util.spec_from_file_location("relational_workload", PATH)
assert spec and spec.loader
relationships = importlib.util.module_from_spec(spec)
spec.loader.exec_module(relationships)


class IndependentRelationTests(unittest.TestCase):
    def test_graph_structure_and_revision_mutation(self):
        old = relationships.graph_spec(8, 2, 0)
        newer = relationships.graph_spec(8, 2, 1)
        self.assertIn("bench.m0001", old["bench.m0000"])
        self.assertIn("bench.m0002", old["bench.m0000"])
        self.assertEqual(len(old), 11)
        self.assertNotIn("bench.m0000", old["bench.m0001"])
        self.assertIn("bench.m0000", newer["bench.m0001"])
        self.assertEqual(old["tests.test_links"], {"bench.m0000", "bench.m0004"})

    def test_oracle_separate_from_index(self):
        graph = relationships.graph_spec(8, 1, 0)
        self.assertEqual(relationships.oracle(graph, "bench.m0000", "focus"), ["bench/m0000.py"])
        self.assertEqual(relationships.oracle(graph, "bench.m0000", "dependencies_direct"), ["bench/m0001.py"])
        self.assertEqual(relationships.oracle(graph, "bench.m0000", "dependents_direct"), ["tests/test_links.py"])
        self.assertEqual(relationships.oracle(graph, "bench.m0004", "tests"), ["tests/test_links.py"])
        self.assertEqual(relationships.oracle(graph, "bench.m0007", "dependencies_direct"), [])
        with self.assertRaises(ValueError):
            relationships.oracle(graph, "nonexistent", "focus")

    def test_generated_git_revision_history_is_reproducible(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "source"
            history = relationships.fixture(folder, 8, 2, 3)
            self.assertEqual(len(history), 3)
            self.assertEqual(len(set(item["commit"] for item in history)), 3)
            self.assertEqual(len(set(item["tree"] for item in history)), 3)
            self.assertEqual(
                (folder / "bench/m0001.py").read_text(),
                relationships.source_for(history[-1]["graph"], "bench.m0001"),
            )
            edge_counts = [sum(len(v) for v in s["graph"].values()) for s in history]
            self.assertEqual(edge_counts, [15, 16, 16])
            self.assertEqual(
                subprocess.check_output(["git", "-C", str(folder), "status", "--porcelain"]),
                b"",
            )

    def test_invalid_dimensions_fail_before_indexing(self):
        for size, fanout, revision in [(0, 1, 0), (3, 1, 0), (8, 8, 0), (8, 0, 0), (8, 1, -1)]:
            with self.subTest(size=size, fanout=fanout, revision=revision), self.assertRaises(ValueError):
                relationships.graph_spec(size, fanout, revision)


class RealBinaryWorkload(unittest.TestCase):
    def test_one_workload_against_compiled_rust(self):
        binary = ROOT / "core/target/debug/excession"
        if not binary.is_file():
            self.skipTest("build the Rust binary first")
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp) / "output"
            run = subprocess.run([
                sys.executable, str(PATH), "--bin", str(binary),
                "--out", str(out), "--sizes", "8", "--fanouts", "1",
                "--revisions", "2", "--repetitions", "1",
            ], capture_output=True, text=True, timeout=90)
            self.assertEqual(run.returncode, 0, run.stderr + run.stdout)
            summary = json.loads((out / "summary.json").read_text())
            self.assertEqual(summary["query_count_per_arm"], 24)
            self.assertEqual(summary["status_counts"]["graph"]["ok"], 24)
            self.assertEqual(summary["status_counts"]["view"]["ok"], 24)
            self.assertTrue(all(
                record["independently_specified_edges_equal"]
                for item in summary["workloads"] for record in item["revisions"]
            ))
            self.assertTrue((out / "modules8_fanout1.bundle").is_file())
            self.assertTrue((out / "report.json").is_file())
            validated = subprocess.run([
                str(binary), "benchmark", "validate",
                "--report", str(out / "report.json"),
            ], capture_output=True, text=True, timeout=20)
            self.assertEqual(validated.returncode, 0, validated.stderr)
            self.assertEqual(json.loads((out / "report.json").read_text())["status"], "passed")


if __name__ == "__main__":
    unittest.main()
