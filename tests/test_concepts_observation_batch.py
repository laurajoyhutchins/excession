"""Draft observation-batch contract tests. Runtime tests intentionally pending."""
import copy
import json
import os
import subprocess
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/observation-batch.v1.schema.json"
SAMPLE = {
    "schema": "excession.concepts-observation-batch.v1",
    "batch_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "source_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "source_tree": "cccccccccccccccccccccccccccccccccccccccc",
    "completeness": "PARTIAL_CANDIDATES_ONLY",
    "subjects": [{
        "subject_id": "f" * 64, "kind": "SOURCE_FILE",
        "source_commit": "b" * 40, "path": "core/src/lib.rs",
        "blob": "1" * 40, "producer": "excession.semantic-observations.v1",
        "producer_receipt_id": "d" * 64
    }],
    "inputs": [
        {
            "receipt_id": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "producer": "rust-analyzer",
            "version": "pinned-v1",
            "kind": "STATIC_SYNTAX",
            "scope": [
                "core/src"
            ]
        }
    ],
    "observations": [
        {
            "observation_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "receipt_id": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "source_observation_id": "a" * 64,
            "native_relation": "IMPORTS",
            "specifier": ".helpers",
            "condition_evaluation": "NOT_EVALUATED",
            "rule": "TEST_RULE",
            "language": "rust",
            "kind": "STATIC_IMPORT_CANDIDATE",
            "source_subject_id": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "status": "UNRESOLVED"
        }
    ],
    "gaps": [
        {
            "reason": "UNRESOLVED_SYMBOL"
        }
    ]
}


class ContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        schema = json.loads(SCHEMA.read_text())
        Draft202012Validator.check_schema(schema)
        cls.validator = Draft202012Validator(schema)

    def test_positive_fixture(self):
        self.assertEqual(list(self.validator.iter_errors(SAMPLE)), [])

    def test_reject_unknown_fields(self):
        value = copy.deepcopy(SAMPLE)
        value["unreviewed_authority"] = "LLM_CONFIRMED"
        self.assertNotEqual(list(self.validator.iter_errors(value)), [])

    def test_reject_invalid_domain_state(self):
        value = copy.deepcopy(SAMPLE)
        value["inputs"][0]["kind"] = "UNTRUSTED_PARSED_RUNTIME"
        self.assertNotEqual(list(self.validator.iter_errors(value)), [])

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "Rust normalization not implemented")
    def test_missing_input_receipt_is_refused_by_cli(self):
        cmd = [str(ROOT / "core/target/debug/excession"), "concepts", "normalize", "--repo", "/missing-repo", "--report", "/missing-receipt.json"]
        proc = subprocess.run(cmd, capture_output=True, timeout=10)
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(proc.stdout)


if __name__ == "__main__":
    unittest.main()
