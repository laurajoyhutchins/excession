"""ODCS metadata matches the wire schema; behavioral proof is deliberately pending."""
import json
from pathlib import Path
import unittest
from jsonschema import Draft201909Validator, Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "contracts/evidence-core/rea-import.v1.schema.json"
ODCS_PATH = ROOT / "contracts/evidence-core/rea-import.v1.odcs.json"
UPSTREAM = ROOT / "contracts/vendor/odcs-json-schema-v3.2.0.json"

class OdcsContractTests(unittest.TestCase):
    def test_catalogue_matches_wire_and_upstream_odcs(self):
        wire = json.loads(SCHEMA_PATH.read_text())
        metadata = json.loads(ODCS_PATH.read_text())
        upstream = json.loads(UPSTREAM.read_text())
        Draft202012Validator.check_schema(wire)
        Draft201909Validator.check_schema(upstream)
        Draft201909Validator(upstream).validate(metadata)
        self.assertEqual(metadata["apiVersion"], "v3.2.0")
        names = [p["name"] for p in metadata["schema"][0]["properties"]]
        self.assertEqual(set(names), set(wire["properties"]))
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual([p["value"] for p in metadata["customProperties"]
                          if p["property"] == "jsonSchemaPath"],
                         [SCHEMA_PATH.relative_to(ROOT).as_posix()])
        self.assertEqual({x["id"] for x in metadata["context"]["constraints"]},
                         {"no_semantic_promotion", "no_absence_inference", "no_self_hash_attestation"})

    @unittest.skip("TDD pending: Rust/producer evidence and independent accuracy audit")
    def test_real_quality_obligations(self):
        self.fail("ODCS quality text alone is not a verifier")

if __name__ == "__main__":
    unittest.main()
