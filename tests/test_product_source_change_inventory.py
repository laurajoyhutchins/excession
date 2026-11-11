"""Wire-shape tests only; native behavior remains pending."""
import json
import unittest
from jsonschema import ValidationError
from product_contract_support import validate_schema

class Product5SchemaTests(unittest.TestCase):
    def report(self):
        return json.loads(r'''{"schema":"excession.change-inventory.v1","report_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","base_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","base_tree":"cccccccccccccccccccccccccccccccccccccccc","head_commit":"cccccccccccccccccccccccccccccccccccccccc","head_tree":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","changes":[{"change_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","kind":"DELETED","before":{"source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_tree":"cccccccccccccccccccccccccccccccccccccccc","path":"src/gone.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"},"after":null},{"change_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","kind":"ADDED","before":null,"after":{"source_commit":"cccccccccccccccccccccccccccccccccccccccc","source_tree":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","path":"src/new.py","blob":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":"100644"}}]}''')

    def test_valid_wire_shape(self):
        validate_schema("change-inventory.v1.schema.json", self.report())

    def test_reject_deleted_without_old_source(self):
        report = self.report()
        report["changes"][0]["before"] = None
        with self.assertRaises(ValidationError):
            validate_schema("change-inventory.v1.schema.json", report)

    def test_reject_unproved_rename_status(self):
        report = self.report()
        report["changes"][1]["kind"] = "RENAMED_VERIFIED"
        with self.assertRaises(ValidationError):
            validate_schema("change-inventory.v1.schema.json", report)

    def test_reject_missing_head_tree(self):
        report = self.report()
        report.pop("head_tree")
        with self.assertRaises(ValidationError):
            validate_schema("change-inventory.v1.schema.json", report)

    @unittest.skip("TDD pending independent Git/native implementation")
    def test_end_to_end_behavior(self):
        self.fail("not implemented")

if __name__ == "__main__":
    unittest.main()
