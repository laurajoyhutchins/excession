"""Draft compilation contract tests. Runtime tests intentionally pending."""
import copy
import json
import os
import subprocess
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/compilation.v1.schema.json"
SAMPLE = {
    "schema": "excession.concepts-compilation.v1",
    "compilation_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "source_commit": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "source_tree": "cccccccccccccccccccccccccccccccccccccccc",
    "input_batch_ids": [
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    ],
    "rule_set_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    "compiler_version": "v1",
    "model_id": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "snapshot_id": "1111111111111111111111111111111111111111111111111111111111111111",
    "decisions": {
        "proposed": 1,
        "unresolved": 2,
        "rejected": 0
    },
    "diagnostics": [
        {
            "status": "UNRESOLVED",
            "reason": "Symbol cannot be tied to source"
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
        value["decisions"]["proposed"] = -1
        self.assertNotEqual(list(self.validator.iter_errors(value)), [])

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "Rust compiler not implemented")
    def test_compile_refuses_missing_receipts(self):
        cmd = [str(ROOT / "core/target/debug/excession"), "concepts", "compile", "--batch", "/missing.json"]
        proc = subprocess.run(cmd, capture_output=True, timeout=10)
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(proc.stdout)


if __name__ == "__main__":
    unittest.main()
