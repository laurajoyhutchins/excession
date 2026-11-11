"""Typed query v1: static candidates cannot masquerade as native/runtime truth."""
from __future__ import annotations

import copy
from pathlib import Path
import sys
import unittest

from jsonschema import ValidationError

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import validate_evidence_contract as contract  # noqa: E402
from validate_semantic_contract import digest  # noqa: E402

OID = "a" * 40
TREE = "b" * 40
SHA = "f" * 64
LIMITS = [
    "SOURCE_REPLAY_DOES_NOT_PROVE_ALL_DEPENDENCIES",
    "COMPILER_DECLARATIONS_ARE_UNATTESTED_INPUT_SETS_NOT_IMPORT_EDGES",
    "RUNTIME_OBSERVATIONS_ARE_NOT_COMPLETE_OR_INDEPENDENTLY_ATTESTED",
    "ABSENCE_OF_OBSERVATION_IS_NEVER_ABSENCE_OF_DEPENDENCY",
]


def receipt(*, compiler=True, runtime=True):
    candidate = {
        "source_path": "src/app.py", "source_blob": OID,
        "target_path": "src/plugin.py", "target_blob": TREE,
        "language": "python", "specifier": "plugin", "line": 2,
        "resolution": "PYTHON_MODULE_CANDIDATE",
    }
    unresolved = {
        "source_path": "src/app.py", "source_blob": OID,
        "language": "python", "specifier": "external_package", "line": 3,
        "reason": "NO_UNIQUE_TRACKED_MODULE",
    }
    inputs = [{"path": "src/app.py", "blob": OID, "mode": "100644", "role": "source"}] if compiler else []
    native = {
        "declaration_id": SHA, "subject": "src/app.py",
        "language": "rust", "coverage": "DECLARED_INPUTS_ONLY",
        "provenance": "UNATTESTED_NATIVE_DECLARATION", "selected_inputs": 1,
        "assumptions": {
            "source-commit": OID, "source-tree": TREE,
            "dependency-discovery": "excession-toolchain-native-v1",
            "toolchain-sha256": SHA, "toolchain-version": "compiler fixture",
            "platform": "linux-x86_64",
        },
    } if compiler else None
    witnessed = [{
        "selection": "PINNED_TARGET_PATH",
        "witness": {
            "importer": "app", "module": "plugin",
            "mechanism": "importlib.import_module",
            "observation": "MODULE_PRESENT", "count": 1,
            "source": {
                "status": "GIT_BOUND", "path": "src/app.py",
                "mode": "100644", "blob_oid": OID,
            },
        },
    }] if runtime else []
    runtime_receipt = {
        "receipt_id": SHA,
        "entrypoint": {"path": "src/app.py", "blob_oid": OID, "mode": "100644"},
        "execution": {"exit_code": 0, "failure": None, "elapsed_wall_ms": 1.25},
        "interpreter": {"version": "3.13.5", "executable": "/usr/bin/python3", "sha256": SHA},
        "claim": "OBSERVED_RUNTIME_ONLY",
        "provenance": "SELF_HASHED_UNATTESTED_EXECUTION",
    } if runtime else None
    gaps = [{"kind": "COVERAGE_UNPROVEN", "reason": "audit hooks are bypassable"}] if runtime else []
    body = {
        "schema": "excession.evidence-query.v1",
        "source_commit": OID, "source_tree": TREE,
        "focus": {"path": "src/app.py", "blob": OID, "mode": "100644"},
        "static_replay": "REGENERATED_FROM_PINNED_GIT",
        "static_resolution_id": SHA,
        "static_candidates": [candidate],
        "unresolved_imports": [unresolved],
        "source_gaps": [],
        "compiler_inputs": inputs,
        "compiler_receipt": native,
        "runtime_witnesses": witnessed,
        "runtime_gaps": gaps,
        "runtime_receipt": runtime_receipt,
        "counts": {
            "static_candidates": 1, "unresolved_static_imports": 1,
            "scoped_source_gaps": 0,
            "compiler_declared_inputs": len(inputs),
            "runtime_witness_records": len(witnessed),
            "runtime_gaps": len(gaps),
        },
        "limits": LIMITS,
    }
    return {**body, "query_id": digest(body)}


def resign(document):
    document["query_id"] = digest({k: v for k, v in document.items() if k != "query_id"})
    return document


class EvidenceContractTests(unittest.TestCase):
    def test_catalogue_is_odcs_32_and_reuses_semantic_v1(self):
        contract.validate_catalog_evidence()

    def test_all_three_evidence_families_preserve_distinct_types(self):
        value = receipt()
        contract.validate_evidence_query(value)
        static = contract.semantic_record(value["static_candidates"][0], True)
        self.assertEqual(static["assessment"]["status"], "CANDIDATE")
        self.assertEqual(static["kind"], "IMPORTS")
        missing = contract.semantic_record(value["unresolved_imports"][0], False)
        self.assertEqual(missing["assessment"]["status"], "UNRESOLVED")
        self.assertNotIn("target", missing)
        self.assertEqual(value["compiler_receipt"]["provenance"],
                         "UNATTESTED_NATIVE_DECLARATION")
        self.assertEqual(value["runtime_receipt"]["claim"], "OBSERVED_RUNTIME_ONLY")

    def test_not_supplied_is_null_without_implied_completeness(self):
        contract.validate_evidence_query(receipt(compiler=False, runtime=False))

    def test_prohibited_promotions_fail_even_after_resigning(self):
        changes = [
            lambda x: x["compiler_receipt"].update(provenance="COMPILER_VERIFIED"),
            lambda x: x["runtime_receipt"].update(claim="COMPLETE_RUNTIME"),
            lambda x: x.update(static_replay="NOT_REPLAYED"),
            lambda x: x.update(limits=["ALL_DEPENDENCIES_COMPLETE"]),
            lambda x: x["static_candidates"][0].update(assessment={"status": "NATIVE_VERIFIED"}),
            lambda x: x["focus"].update(path="../escape.py"),
            lambda x: x["runtime_witnesses"][0].update(selection="COMPILER_CONFIRMED"),
            lambda x: x["compiler_inputs"][0].update(role="COMPILER_IMPORT_EDGE"),
            lambda x: x["runtime_witnesses"][0]["witness"]["source"].update(status="ATTESTED"),
        ]
        for i, change in enumerate(changes):
            value = receipt()
            change(value)
            resign(value)
            with self.subTest(i=i), self.assertRaises((ValueError, ValidationError)):
                contract.validate_evidence_query(value)

    def test_cross_field_invalidity_is_not_fixed_by_self_hash(self):
        cases = [
            lambda x: x["counts"].update(static_candidates=10),
            lambda x: x["compiler_receipt"].update(selected_inputs=20),
            lambda x: x["compiler_receipt"]["assumptions"].update(**{"source-tree": "c" * 40}),
            lambda x: x["runtime_witnesses"][0]["witness"]["source"].update(path="src/other.py"),
            lambda x: x["runtime_witnesses"][0].update(selection="EXECUTION_CONTEXT_ONLY"),
        ]
        for i, change in enumerate(cases):
            value = receipt()
            change(value)
            resign(value)
            if i == 4:
                # The entrypoint matches the focus; this selection is legitimate.
                contract.validate_evidence_query(value)
            else:
                with self.subTest(i=i), self.assertRaises((ValueError, ValidationError)):
                    contract.validate_evidence_query(value)

    def test_optional_receipt_null_cannot_hide_populated_records(self):
        for which in ("compiler_receipt", "runtime_receipt"):
            value = receipt()
            value[which] = None
            resign(value)
            with self.subTest(which=which), self.assertRaises(ValidationError):
                contract.validate_evidence_query(value)

    def test_query_id_cannot_hide_tampering(self):
        value = receipt()
        value["static_candidates"][0]["source_blob"] = "e" * 40
        with self.assertRaisesRegex(ValueError, "canonical query digest"):
            contract.validate_evidence_query(value)


if __name__ == "__main__":
    unittest.main()
