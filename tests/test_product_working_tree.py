"""Schema contract tests are active; implementation acceptance is deliberately skipped."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema


class WorkingTreeContractTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.working-tree.v1","source_kind":"COMMITTED_GIT","source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","worktree_status":"DIRTY","changes":[{"path":"src/main.rs","state":"UNSTAGED"}],"analysis_scope":"COMMITTED_OBJECTS_ONLY"}''')

    def test_accepts_valid_draft_shape(self):
        validate_schema("working-tree.v1.schema.json", self.report())

    def test_rejects_worktree_as_commit(self):
        report = self.report()
        report["source_kind"] = "UNCOMMITTED_GIT_COMMIT"
        with self.assertRaises(ValidationError):
            validate_schema("working-tree.v1.schema.json", report)

    def test_rejects_fabricated_scope(self):
        report = self.report()
        report["analysis_scope"] = "COMMITTED_PLUS_UNSTAGED_VERIFIED"
        with self.assertRaises(ValidationError):
            validate_schema("working-tree.v1.schema.json", report)

    def test_rejects_unknown_change_state(self):
        report = self.report()
        report["changes"][0]["state"] = "SAVED_EDITOR_BUFFER"
        with self.assertRaises(ValidationError):
            validate_schema("working-tree.v1.schema.json", report)

    @unittest.skip("Behavioral TDD pending: same-commit clean-vs-dirty parity, scanner trust unchanged")
    def test_pending_native_behavior(self):
        self.fail("native implementation not yet present")


if __name__ == "__main__":
    unittest.main()
