"""Typed evidence identity contract: active negative-first shape tests."""
import copy
import json
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = json.loads((ROOT / "contracts/evidence-core/identity-receipt.v1.schema.json").read_text())
H = "a" * 40
T = "b" * 40
D = "c" * 64
E = "d" * 64
SOURCE = {"commit": H, "tree": T, "path": "src/lib.rs", "blob": H, "mode": "100644"}
RECEIPT = {"producer": "scip-rust", "version": "1.0", "payload_sha256": D, "input_sha256": E,
           "provenance": "SELF_HASHED_UNATTESTED"}

def report(subject, receipt=None):
    return {"schema": "excession.identity-receipt.v1", "subject": subject, "receipt": receipt or RECEIPT,
            "source_identity_assessment": "SOURCE_BINDING_REQUIRES_INDEPENDENT_GIT_REPLAY"}

class IdentityReceiptContracts(unittest.TestCase):
    def setUp(self):
        self.validator = Draft202012Validator(SCHEMA)

    def reject(self, value):
        with self.assertRaises(ValidationError):
            self.validator.validate(value)

    def test_shape_and_discriminated_subjects(self):
        Draft202012Validator.check_schema(SCHEMA)
        subjects = [
            {"kind": "SOURCE_FILE", "source": SOURCE},
            {"kind": "SYMBOL", "source": SOURCE, "scip_symbol": "rust-analyzer cargo pkg::fun",
             "span": {"first_line": 1, "last_line": 4, "encoding": "UTF16"}},
            {"kind": "ARTIFACT", "sha256": D, "size_bytes": 101},
            {"kind": "EXECUTION", "artifact_sha256": D, "execution_receipt_sha256": E},
            {"kind": "CONFIGURATION", "sha256": D},
        ]
        for subject in subjects:
            with self.subTest(kind=subject["kind"]):
                self.validator.validate(report(subject))

    def test_artifacts_cannot_claim_git_source_by_shape(self):
        self.reject(report({"kind": "ARTIFACT", "sha256": D, "size_bytes": 101, "source": SOURCE}))
        self.reject(report({"kind": "ARTIFACT", "source": SOURCE}))

    def test_invalid_variants_and_missing_symbol_identity(self):
        self.reject(report({"kind": "SOURCE_FILE", "source": SOURCE, "sha256": D}))
        self.reject(report({"kind": "SYMBOL", "source": SOURCE}))
        self.reject(report({"kind": "MADE_UP", "source": SOURCE}))
        self.reject(report({"kind": "EXECUTION", "artifact_sha256": D, "execution_receipt_sha256": "bad"}))

    def test_invalid_git_paths_modes_and_hashes(self):
        for path in ["../escape", "src/../lib.rs", "src//lib.rs", "src/./lib.rs", "/tmp/data", "src\\lib.rs"]:
            bad = copy.deepcopy(SOURCE)
            bad["path"] = path
            with self.subTest(path=path):
                self.reject(report({"kind": "SOURCE_FILE", "source": bad}))
        for mode in ["120000", "160000", "0100644"]:
            bad = copy.deepcopy(SOURCE)
            bad["mode"] = mode
            with self.subTest(mode=mode):
                self.reject(report({"kind": "SOURCE_FILE", "source": bad}))
        bad = copy.deepcopy(SOURCE)
        bad["blob"] = "A" * 40
        self.reject(report({"kind": "SOURCE_FILE", "source": bad}))

    def test_attestation_claim_requires_separate_receipt(self):
        subject = {"kind": "ARTIFACT", "sha256": D, "size_bytes": 0}
        missing = {**RECEIPT, "provenance": "EXTERNAL_ATTESTATION_CLAIM"}
        self.reject(report(subject, missing))
        self.validator.validate(report(subject, {**missing, "attestation_sha256": E}))
        self.reject(report(subject, {**RECEIPT, "attestation_sha256": E}))
        self.reject(report(subject, {**RECEIPT, "provenance": "INDEPENDENT_ATTESTATION"}))

    def test_unknown_fields_and_missing_required(self):
        value = report({"kind": "SOURCE_FILE", "source": SOURCE})
        for key in SCHEMA["required"]:
            bad = copy.deepcopy(value)
            bad.pop(key)
            with self.subTest(required=key):
                self.reject(bad)
        bad = copy.deepcopy(value)
        bad["assumed_authority"] = "VERIFIED"
        self.reject(bad)

    @unittest.skip("TDD pending: real upstream SCIP normalization and independent native comparison")
    def test_upstream_scip_normalization_end_to_end(self):
        self.fail("Needs real upstream SCIP and independent source oracle")

if __name__ == "__main__":
    unittest.main()
