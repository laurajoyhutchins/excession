"""Cheap preflight contract tests without Tree-sitter or a Git checkout.

Preflight must catch local report corruption before expensive parser/Git replay;
a self-consistent forgery still requires the full source-bound audit.
"""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"


def fingerprint(body):
    return hashlib.sha256(json.dumps(
        body, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()


def seal(body, field):
    record = copy.deepcopy(body)
    record[field] = fingerprint(body)
    return record


def syntax_report():
    source = {
        "source_path": "src/entry.rs",
        "source_blob": "a" * 40,
        "source_mode": "100644",
        "language": "rust",
        "status": "PARSED",
        "structure": [],
        "imports": [
            {"source": "use crate::value;", "line": 1, "wildcard": False}
        ],
        "diagnostics": [],
    }
    return seal({
        "schema": "excession.syntax-observations.v1",
        "parser": "tree-sitter-language-pack",
        "parser_version": "1.25.0",
        "completeness": "CONSERVATIVE_SYNTAX_ONLY",
        "source_commit": "b" * 40,
        "source_tree": "c" * 40,
        "roots": ["src"],
        "source_count": 1,
        "sources": [source],
    }, "report_id")


def resolution_report():
    source = {
        "source_path": "src/entry.rs",
        "source_blob": "a" * 40,
        "target_path": "src/value.rs",
        "target_blob": "d" * 40,
        "language": "rust",
        "specifier": "crate::value",
        "line": 1,
        "resolution": "RUST_CRATE_PATH_CANDIDATE",
    }
    return seal({
        "schema": "excession.syntax-dependencies.v1",
        "source_commit": "b" * 40,
        "source_tree": "c" * 40,
        "syntax_report_id": "e" * 64,
        "completeness": "CONSERVATIVE_CANDIDATES_ONLY",
        "candidate_count": 1,
        "unresolved_count": 0,
        "candidate_edges": [source],
        "unresolved_imports": [],
        "skipped_sources": [],
        "metadata": [],
    }, "resolution_id")


@unittest.skipUnless(BIN.exists(), "build the Excession binary first")
class PreflightCLI(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="excession-preflight-")
        self.addCleanup(self.temp.cleanup)
        self.report = Path(self.temp.name) / "report.json"

    def call(self, command, data):
        self.report.write_text(json.dumps(data, sort_keys=True))
        return subprocess.run(
            [str(BIN), "syntax", command, "--report", str(self.report)],
            capture_output=True, check=False,
        )

    def test_valid_syntax_preflight_is_not_source_authentication(self):
        response = self.call("preflight", syntax_report())
        self.assertEqual(response.returncode, 0, response.stderr)
        output = json.loads(response.stdout)
        self.assertEqual(output["status"], "SHAPE_AND_DIGEST_ONLY")
        self.assertEqual(output["source_count"], 1)

    def test_valid_resolution_preflight_is_not_source_authentication(self):
        response = self.call("resolve-preflight", resolution_report())
        self.assertEqual(response.returncode, 0, response.stderr)
        self.assertEqual(json.loads(response.stdout)["status"], "SHAPE_AND_DIGEST_ONLY")

    def test_tampering_invalidates_both_digests(self):
        for cmd, factory, key in [
            ("preflight", syntax_report, "source_commit"),
            ("resolve-preflight", resolution_report, "source_tree"),
        ]:
            report = factory()
            report[key] = "f" * 40
            with self.subTest(cmd=cmd):
                response = self.call(cmd, report)
                self.assertNotEqual(response.returncode, 0)
                self.assertEqual(response.stdout, b"")
                self.assertIn(b"REFUSED:", response.stderr)

    def test_self_consistent_forgery_is_only_provisional(self):
        report = syntax_report()
        report["sources"][0]["source_blob"] = "f" * 40
        report = seal({k: v for k, v in report.items() if k != "report_id"}, "report_id")
        result = self.call("preflight", report)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["status"], "SHAPE_AND_DIGEST_ONLY")

    def test_malformed_reports_rejected_without_parsers(self):
        examples = [
            ("preflight", syntax_report(), ("source_count", 2)),
            ("preflight", syntax_report(), ("unexpected", "evil")),
            ("preflight", syntax_report(), ("roots", ["../outside"])),
            ("resolve-preflight", resolution_report(), ("candidate_count", 5)),
            ("resolve-preflight", resolution_report(), ("unknown", True)),
        ]
        for cmd, report, (key, value) in examples:
            report[key] = value
            report = seal({k: v for k, v in report.items()
                           if k not in ("report_id", "resolution_id")},
                          "report_id" if cmd == "preflight" else "resolution_id")
            with self.subTest(cmd=cmd, key=key):
                response = self.call(cmd, report)
                self.assertNotEqual(response.returncode, 0)
                self.assertIn(b"REFUSED:", response.stderr)


if __name__ == "__main__":
    unittest.main()
