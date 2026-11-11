"""Draft snapshot contract tests. Runtime tests intentionally pending."""
import copy
import json
import os
import subprocess
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "contracts/concepts/snapshot.v1.schema.json"
SAMPLE = {
    "schema": "excession.concepts-snapshot.v1",
    "snapshot_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "model_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "source_commit": "cccccccccccccccccccccccccccccccccccccccc",
    "source_tree": "dddddddddddddddddddddddddddddddddddddddd",
    "input_batch_ids": [
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    ],
    "compiler_version": "v1",
    "rule_set_id": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "record_counts": {
        "subjects": 2,
        "concepts": 1,
        "claims": 1,
        "support_links": 1
    },
    "coverage_status": "PARTIAL"
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
        value["record_counts"]["claims"] = -1
        self.assertNotEqual(list(self.validator.iter_errors(value)), [])

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "SQLite snapshot implementation not added")
    def test_snapshot_cli_rejects_missing_model(self):
        cmd = [str(ROOT / "core/target/debug/excession"), "concepts", "snapshot", "import", "--model", "/missing.json"]
        proc = subprocess.run(cmd, capture_output=True, timeout=10)
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(proc.stdout)


if __name__ == "__main__":
    unittest.main()
