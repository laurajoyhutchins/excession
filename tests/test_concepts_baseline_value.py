"""TDD for source-bound baseline novelty, blinded review packets, and no false enablement."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
from concepts_baseline_value import (  # noqa: E402
    baseline_pairs, concept_pairs, evaluate, fixture_report, digest, attach_measured_costs,
)


def seal(item: dict, key: str) -> dict:
    item[key] = digest({k: v for k, v in item.items() if k != key})
    return item


def subject(path: str, letter: str) -> dict:
    return seal({
        "path": path, "blob": letter * 40,
        "kind": "SOURCE_FILE", "source_commit": "f" * 40,
    }, "subject_id")


def candidate(a: dict, b: dict) -> dict:
    return {
        "candidate_id": "e" * 64,
        "kind": "reciprocal_symbol_references",
        "status": "UNVERIFIED_REFACTOR_HYPOTHESIS",
        "sources": [a, b],
    }


class ValueGateTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="concept-value-gate-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.prefix = "fixture-python"
        self.a, self.b, self.c = (
            subject("pkg/a.py", "a"), subject("pkg/b.py", "b"),
            subject("pkg/c.py", "c"),
        )
        self.subj = {x["subject_id"]: x for x in [self.a, self.b, self.c]}
        membership = []
        for s in [self.a, self.b]:
            membership.append(seal({
                "predicate": "REALIZES_CONCEPT",
                "assessment": "PROPOSED",
                "behavioral_verification": "UNTESTED",
                "concept_id": "d" * 64,
                "subjects": [s["subject_id"]],
            }, "claim_id"))
        self.baseline = seal({
            "schema": "excession.scip-refactor.v1",
            "status": "UNVERIFIED_SCIP_RELATIONS",
            "source_commit": "f" * 40,
            "source_tree": "d" * 40,
            "index_sha256": "1" * 64,
            "candidates": [candidate(self.a, self.b)],
        }, "report_id")
        self.model = seal({
            "schema": "excession.concepts-model.v1",
            "source_commit": "f" * 40,
            "source_tree": "d" * 40,
            "subjects": [self.a, self.b, self.c],
            "claims": membership,
            "concepts": [{
                "concept_id": "d" * 64, "review_status": "UNREVIEWED",
                "membership_claims": [x["claim_id"] for x in membership],
            }],
        }, "model_id")
        self.batch = seal({
            "schema": "excession.concepts-observation-batch.v1",
            "completeness": "PARTIAL_CANDIDATES_ONLY",
            "source_commit": "f" * 40,
            "source_tree": "d" * 40,
            "subjects": [self.a, self.b, self.c],
            "inputs": [{"configuration_id": "1" * 64, "receipt_id": "2" * 64}],
            "observations": [{
                "source_subject_id": self.a["subject_id"],
                "target_subject_id": self.b["subject_id"],
                "kind": "SCIP_REFERENCE", "native_relation": "SCIP_SYMBOL_REFERENCE",
                "status": "OBSERVED",
            }, {
                "source_subject_id": self.b["subject_id"],
                "target_subject_id": self.a["subject_id"],
                "kind": "SCIP_REFERENCE", "native_relation": "SCIP_SYMBOL_REFERENCE",
                "status": "OBSERVED",
            }, {
                "source_subject_id": self.a["subject_id"],
                "target_subject_id": self.c["subject_id"],
                "kind": "SCIP_REFERENCE", "native_relation": "SCIP_SYMBOL_REFERENCE",
                "status": "OBSERVED",
            }],
        }, "batch_id")
        self.snapshot = seal({
            "source_commit": "f" * 40,
            "source_tree": "d" * 40,
            "model_id": self.model["model_id"],
            "input_batch_ids": [self.batch["batch_id"]],
        }, "snapshot_id")
        self.oracle = {
            "status": "SOURCE_BOUND_NATIVE_INTERPRETATION_AGREES",
            "source_commit": "f" * 40,
            "source_tree": "d" * 40,
            "indexer_receipt": "2" * 64,
            "indexer_semantic_correctness_independently_verified": False,
        }
        self.write()

    def write(self):
        fixtures = {
            "report": self.baseline, "concepts.model": self.model,
            "concepts.batch": self.batch, "concepts.snapshot": self.snapshot,
            "concepts.oracle": self.oracle,
        }
        for part, content in fixtures.items():
            (self.root / f"{self.prefix}.{part}.json").write_text(
                json.dumps(content), encoding="utf-8"
            )

    def test_source_pair_baseline_contains_every_concept_pair(self):
        row, cases = fixture_report(self.root, self.prefix)
        self.assertEqual(row["baseline_reciprocal_pairs"], 1)
        self.assertEqual(row["proposed_concept_pairs"], 1)
        self.assertEqual(row["unchanged_pairs"], 1)
        self.assertEqual(row["concept_only_pairs"], 0)
        self.assertEqual(row["one_way_negative_controls"], 1)
        self.assertEqual(len(cases), 2)
        self.assertTrue(all(c["assessment"] == "UNJUDGED" for c in cases))
        self.assertTrue(all(c["reviewer"] is None for c in cases))
        self.assertTrue(all("predicted" not in c for c in cases))

    def test_no_label_never_means_verified_quality(self):
        report, packets = evaluate([self.root])
        self.assertEqual(report["decision"], "DO_NOT_ENABLE_BY_DEFAULT")
        self.assertIsNone(report["architectural_precision"])
        self.assertIsNone(report["user_usefulness_gain"])
        self.assertEqual(report["totals"]["architectural_labels_independently_adjudicated"], 0)
        self.assertEqual(report["totals"]["unchanged_pairs"], 1)
        self.assertFalse(packets["held_out"])
        self.assertFalse(packets["reviewer_identity_authenticated"])
        self.assertEqual(packets["packet_id"], digest({
            k: v for k, v in packets.items() if k != "packet_id"
        }))

    def test_forged_verified_concept_membership_is_rejected(self):
        self.model["claims"][0]["behavioral_verification"] = "PROVEN"
        seal(self.model, "model_id")
        self.snapshot["model_id"] = self.model["model_id"]
        seal(self.snapshot, "snapshot_id")
        self.write()
        with self.assertRaisesRegex(ValueError, "verified or differently typed"):
            fixture_report(self.root, self.prefix)

    def test_tampered_baseline_digest_is_rejected(self):
        self.baseline["candidates"][0]["sources"][0]["path"] = "stale.py"
        self.write()
        with self.assertRaisesRegex(ValueError, "report_id"):
            fixture_report(self.root, self.prefix)

    def test_new_concept_pair_is_reported_but_not_declared_useful(self):
        self.model["claims"][1]["subjects"] = [self.c["subject_id"]]
        seal(self.model["claims"][1], "claim_id")
        self.model["concepts"][0]["membership_claims"][1] = self.model["claims"][1]["claim_id"]
        seal(self.model, "model_id")
        self.snapshot["model_id"] = self.model["model_id"]
        seal(self.snapshot, "snapshot_id")
        self.write()
        result, _ = evaluate([self.root])
        self.assertEqual(result["totals"]["concept_only_pairs"], 1)
        self.assertEqual(result["totals"]["baseline_only_pairs"], 1)
        self.assertEqual(result["decision"], "DO_NOT_ENABLE_BY_DEFAULT")

    def test_duplicate_reciprocal_baseline_pair_is_rejected(self):
        self.baseline["candidates"].append(copy.deepcopy(self.baseline["candidates"][0]))
        seal(self.baseline, "report_id")
        self.write()
        with self.assertRaisesRegex(ValueError, "duplicate baseline"):
            fixture_report(self.root, self.prefix)

    def test_measured_costs_are_bound_to_the_same_git_revision(self):
        report, _ = evaluate([self.root])
        stages = [
            {
                "stage": stage,
                "wall_ms": 1.0,
                "peak_rss_bytes": 1024,
                "stdout_bytes": 128,
                "measurement": "GNU_TIME_LINUX_SINGLE_PROCESS",
            }
            for stage in [
                "baseline-suggest", "baseline-audit",
                "concept-scip-normalize", "concept-scip-preview",
                "concept-scip-compile", "concept-scip-audit",
            ]
        ]
        fixture = {
            "name": "fixture", "source_commit": "f" * 40,
            "source_tree": "d" * 40, "status": "SOURCE_BOUND",
            "languages": [{
                "language_family": "python",
                "index_sha256": "1" * 64,
                "execution_cost": {
                    "sample_count": 1,
                    "native_index_generation_wall_ms": 7.5,
                    "total_excession_wall_ms": 6.0,
                    "per_process": stages,
                },
            }],
        }
        path = self.root / "native-run-summary.json"
        path.write_text(json.dumps({
            "schema": "excession.scip-fixture-evidence.v2",
            "fixtures": [fixture],
        }))
        attach_measured_costs(report, [path])
        self.assertEqual(report["measured_performance"]["sum_cli_process_wall_ms"], 6.0)
        self.assertEqual(report["measured_performance"]["sum_native_indexer_wall_ms"], 7.5)
        self.assertIsNone(report["measured_performance"]["indexer_peak_rss_bytes"])
        self.assertEqual(report["report_id"], digest({
            k: v for k, v in report.items() if k != "report_id"
        }))
        fixture["source_commit"] = "a" * 40
        path.write_text(json.dumps({
            "schema": "excession.scip-fixture-evidence.v2", "fixtures": [fixture],
        }))
        with self.assertRaisesRegex(ValueError, "another Git revision"):
            attach_measured_costs(evaluate([self.root])[0], [path])

    def test_partial_invalid_native_observation_classification_rejected(self):
        self.batch["observations"][0]["native_relation"] = "CALLS"
        seal(self.batch, "batch_id")
        self.snapshot["input_batch_ids"] = [self.batch["batch_id"]]
        seal(self.snapshot, "snapshot_id")
        self.write()
        with self.assertRaisesRegex(ValueError, "unexpected native relation"):
            fixture_report(self.root, self.prefix)


if __name__ == "__main__":
    unittest.main()
