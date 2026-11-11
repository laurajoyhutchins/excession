"""Schema, provenance and negative tests for benchmark reporting without a Rust build."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("benchmark_reporting", ROOT / "benchmarks/reporting.py")
assert SPEC and SPEC.loader
reporting = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reporting)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def native_fixture() -> dict:
    stats = {
        "wall_ms": {"median": 1.25, "nearest_rank_p95": 1.5},
        "child_cpu_ms": {"median": 0.75, "nearest_rank_p95": 1.0},
        "stdout_bytes": {"median": 240, "nearest_rank_p95": 320},
    }
    return {
        "schema": "excession.native-paired-calibration.v1",
        "binary_sha256": "a" * 64, "bundle_sha256": "b" * 64,
        "source_commit": "c" * 40, "source_tree": "d" * 40,
        "paired_queries": 2, "source_set_matches": 2, "repetitions": 2,
        "summary": {"graph": stats, "paths": stats},
        "samples": [{"arm": arm} for arm in ("graph", "paths", "graph", "paths")],
        "limitations": "This synthetic report is only a protocol fixture and cannot establish correctness.",
    }


class BenchmarkSchemaTests(unittest.TestCase):
    def test_schema_is_valid_draft_2020_12(self) -> None:
        schema = json.loads(reporting.SCHEMA_PATH.read_text())
        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        Draft202012Validator.check_schema(schema)

    def build_native(self, out: Path) -> Path:
        (out / "native.json").write_text(json.dumps(native_fixture()) + "\n")
        return reporting.build_native(out, "native.json")

    def mutate_report(self, path: Path, mutate) -> None:
        data = json.loads(path.read_text())
        mutate(data)
        data["run_id"] = reporting.run_id(data)
        path.write_text(json.dumps(data) + "\n")

    def test_native_roundtrip_retains_raw_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            path = self.build_native(out)
            report = reporting.validate(path)
            self.assertEqual(report["schema"], reporting.SCHEMA_NAME)
            self.assertEqual(report["status"], "passed")
            self.assertFalse(report["oracle"]["independent"])
            self.assertEqual(report["subject"]["binary_sha256"], "a" * 64)
            self.assertEqual(len(report["measurements"]), 12)
            self.assertEqual(report["artifacts"][0]["sha256"], sha((out / "native.json").read_bytes()))
            self.assertEqual(report["source_revisions"][0]["tree"], "d" * 40)

    def test_artifact_mutation_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            path = self.build_native(out)
            (out / "native.json").write_text("{}")
            with self.assertRaisesRegex(ValueError, "digest or size mismatch"):
                reporting.validate(path)

    def test_unauthenticated_report_modification_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = self.build_native(Path(temp))
            value = json.loads(path.read_text())
            value["measurements"][0]["value"] = 999999
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "run_id"):
                reporting.validate(path)

    def test_unknown_fields_negative_counts_and_unsafe_paths_fail(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = self.build_native(Path(temp))
            original = json.loads(path.read_text())
            for change in (
                lambda d: d.update({"invented_success": True}),
                lambda d: d["measurements"][0].update({"value": -10}),
                lambda d: d["artifacts"][0].update({"path": "../native.json"}),
                lambda d: d["subject"].update({"binary_sha256": "not-sha256"}),
                lambda d: d["measurements"][0].update({"unit": "tokens-guessed"}),
            ):
                data = copy.deepcopy(original)
                change(data)
                data["run_id"] = reporting.run_id(data)
                path.write_text(json.dumps(data))
                with self.subTest(data=data), self.assertRaisesRegex(ValueError, "invalid benchmark report"):
                    reporting.validate(path)

    def test_missing_measurement_requires_explicit_reason(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = self.build_native(Path(temp))
            report = json.loads(path.read_text())
            metric = report["measurements"][0]
            metric["status"] = "not_measured"
            metric.pop("value")
            metric["reason"] = "No compatible peak-RSS measurement exists in this runner."
            report["run_id"] = reporting.run_id(report)
            path.write_text(json.dumps(report))
            self.assertEqual(reporting.validate(path)["measurements"][0]["status"], "not_measured")
            del metric["reason"]
            report["run_id"] = reporting.run_id(report)
            path.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, "invalid benchmark report"):
                reporting.validate(path)

    def test_duplicate_metrics_and_false_pass_are_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = self.build_native(Path(temp))
            original = json.loads(path.read_text())
            for modify, expected in (
                (lambda r: r["measurements"].append(r["measurements"][0]), "duplicate measurement identity"),
                (lambda r: r["checks"][0].update({"status": "failed"}), "passed report contains"),
            ):
                record = copy.deepcopy(original)
                modify(record)
                record["run_id"] = reporting.run_id(record)
                path.write_text(json.dumps(record))
                with self.assertRaisesRegex(ValueError, expected):
                    reporting.validate(path)

    def test_relational_conversion_counts_checks_and_source_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            bundle = out / "source.bundle"
            bundle.write_bytes(b"fixed source fixture")
            commit, tree = "e" * 40, "f" * 40
            summary = {
                "schema": "excession.relational-retrieval.v1",
                "binary_sha256": "a" * 64,
                "sizes": [8], "fanouts": [1], "revision_count": 1,
                "repetitions": 1, "query_count_per_arm": 1,
                "median_wall_ms": {"graph": 2.1, "view": 3.2},
                "median_raw_stdout_bytes": {"graph": 18, "view": 26},
                "total_query_wall_ms": {"graph": 2.1, "view": 3.2},
                "status_counts": {"graph": {"ok": 1}, "view": {"ok": 1}},
                "limitations": "Synthetic independent source fixture only. No real codebase accuracy is claimed.",
                "workloads": [{
                    "name": "modules8_fanout1", "source_bundle": "source.bundle",
                    "bundle_sha256": sha(bundle.read_bytes()),
                    "revisions": [{
                        "commit": commit, "tree": tree, "index_ms": 4.0,
                        "index_cpu_ms": 3.5, "independently_specified_edges_equal": True,
                    }],
                }],
            }
            (out / "summary.json").write_text(json.dumps(summary))
            trials = [{
                "workload": "modules8_fanout1", "revision": 0, "commit": commit,
                "seed": "bench.m0000", "kind": "focus", "repetition": 0,
                "arm": arm, "status": "ok", "exact_oracle_match": True,
                "source_paths": ["bench/m0000.py"],
            } for arm in ("graph", "view")]
            (out / "trials.jsonl").write_text("".join(json.dumps(row) + "\n" for row in trials))
            report = reporting.validate(reporting.build_relational(out))
            self.assertEqual(report["status"], "passed")
            self.assertEqual(len(report["source_revisions"]), 1)
            self.assertEqual(len(report["artifacts"]), 3)
            self.assertEqual(len(report["checks"]), 2)
            self.assertEqual(len(report["measurements"]), 10)

    def test_non_finite_receipt_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            (out / "native.json").write_text('{"value":NaN}')
            with self.assertRaisesRegex(ValueError, "non-finite"):
                reporting.build_native(out, "native.json")


if __name__ == "__main__":
    unittest.main()
