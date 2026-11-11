"""Proposed read-only concept query and answer contracts."""
import copy
import json
import os
import subprocess
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts/concepts"
QUERY = {"schema": "excession.concepts-query.v1", "query_id": "a" * 64,
         "snapshot_id": "b" * 64, "operation": "EXPLAIN_CLAIM",
         "selector": {"claim_id": "c" * 64}, "limits": {"max_depth": 4, "max_results": 100}}
ANSWER = {"schema": "excession.concepts-answer.v1", "query_id": "a" * 64,
          "snapshot_id": "b" * 64, "source_commit": "d" * 40,
          "answerability": "UNKNOWN", "claim_ids": [], "evidence_receipts": [],
          "gaps": [{"reason": "Missing producer evidence"}], "limits_applied": False}


class ConceptQueryContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.query_schema = json.loads((CONTRACTS / "query.v1.schema.json").read_text())
        cls.answer_schema = json.loads((CONTRACTS / "answer.v1.schema.json").read_text())
        for schema in [cls.query_schema, cls.answer_schema]:
            Draft202012Validator.check_schema(schema)

    def test_valid_query_and_unknown_answer(self):
        self.assertEqual(list(Draft202012Validator(self.query_schema).iter_errors(QUERY)), [])
        self.assertEqual(list(Draft202012Validator(self.answer_schema).iter_errors(ANSWER)), [])

    def test_unbounded_query_rejected(self):
        value = copy.deepcopy(QUERY)
        value["limits"]["max_depth"] = 100000
        self.assertTrue(list(Draft202012Validator(self.query_schema).iter_errors(value)))

    def test_query_must_have_selector(self):
        value = copy.deepcopy(QUERY)
        value["selector"] = {}
        self.assertTrue(list(Draft202012Validator(self.query_schema).iter_errors(value)))

    def test_answer_cannot_claim_llm_verified(self):
        value = copy.deepcopy(ANSWER)
        value["answerability"] = "CERTIFIED_BY_LLM"
        self.assertTrue(list(Draft202012Validator(self.answer_schema).iter_errors(value)))

    @unittest.skipUnless(os.getenv("EXCESSION_CONCEPTS_TDD") == "1", "Semantic query CLI not implemented")
    def test_unsupported_question_reports_machine_readable_status(self):
        binary = ROOT / "core/target/debug/excession"
        proc = subprocess.run([str(binary), "concepts", "answerable", "--query", "/missing-query.json"],
                              capture_output=True, timeout=10)
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(proc.stdout)


if __name__ == "__main__":
    unittest.main()
