"""Contract tests for bounded source-bound SCIP reference receipts."""
import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/concepts/scip-references.v1.schema.json").read_text())
Draft202012Validator.check_schema(SCHEMA)
VALIDATOR = Draft202012Validator(SCHEMA)

A = {"path": "src/alpha.rs", "blob": "a" * 40, "language": "rust"}
B = {"path": "pkg/beta.py", "blob": "b" * 40, "language": "python"}
SAMPLE = {
    "schema": "excession.scip-references.v1",
    "status": "GIT_BOUND_INDEX_NOT_REEXECUTED",
    "source_commit": "c" * 40,
    "source_tree": "d" * 40,
    "index_sha256": "e" * 64,
    "report_id": "f" * 64,
    "indexer": {"name": "synthetic-scip-fixture", "version": "0.1"},
    "documents": [A, B],
    "references": [{"from": A, "to": B, "symbol": "scip rust cargo example 0.1.0 alpha().",
                    "line": 2, "relation": "SCIP_SYMBOL_REFERENCE"}],
    "coverage": {
        "indexed_occurrences": 4, "unresolved_references": 1,
        "matched_cross_file_references": 1, "line_only_position_documents": 0,
        "indexed_documents": 2, "languages": ["rust", "python"],
        "completeness": "PARTIAL_INDEX_SCOPE_ONLY"
    }
}


def test_source_bound_scip_reference_shape():
    assert not list(VALIDATOR.iter_errors(SAMPLE))


def test_scip_cannot_be_promoted_to_runtime_import_or_call():
    for fraudulent in ["IMPORTS", "CALLS", "FFI_BOUNDARY", "BEHAVIORALLY_EQUIVALENT"]:
        value = copy.deepcopy(SAMPLE)
        value["references"][0]["relation"] = fraudulent
        assert list(VALIDATOR.iter_errors(value))


def test_scip_cannot_claim_complete_coverage_or_attestation():
    value = copy.deepcopy(SAMPLE)
    value["coverage"]["completeness"] = "COMPLETE"
    assert list(VALIDATOR.iter_errors(value))
    value = copy.deepcopy(SAMPLE)
    value["indexer"]["authenticated"] = True
    assert list(VALIDATOR.iter_errors(value))


def test_scip_requires_explicit_index_and_git_identity():
    for key in ["index_sha256", "source_commit", "source_tree", "documents", "coverage"]:
        value = copy.deepcopy(SAMPLE)
        value.pop(key)
        assert list(VALIDATOR.iter_errors(value))


def test_observation_batch_limit_is_measured_and_bounded():
    batch = json.loads((ROOT / "contracts/concepts/observation-batch.v1.schema.json").read_text())
    assert batch["properties"]["observations"]["maxItems"] == 20000
    assert batch["properties"]["subjects"]["maxItems"] == 20000
