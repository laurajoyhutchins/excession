"""Contracts for Python-owned wrappers, benchmark tooling and virtual browsing.

Rust owns Git/SQLite, security replay, and source-bound view behavior.
These tests are intentionally limited to the remaining Python entrypoints.
"""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"
BUNDLE = ROOT / "core/fixtures/h11-crossrepo.bundle"


def load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / filename)
    assert spec is not None and spec.loader is not None
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


comparison = load("excession_compare", "compare.py")
protocol = load("excession_protocol", "protocol.py")


def run(*args: object, check: bool = True, input: str | None = None):
    result = subprocess.run(
        [str(arg) for arg in args], capture_output=True, text=True, input=input,
        check=False, timeout=30,
    )
    if check and result.returncode:
        raise AssertionError(f"{args}: {result.stderr}")
    return result


class PurePythonTooling(unittest.TestCase):
    def test_comparison_preserves_duplicate_logical_anchors(self):
        before = {"key": [{"candidate_id": "one"}, {"candidate_id": "two"}]}
        after = {"key": [{"candidate_id": "one"}]}
        result = comparison.compare(before, after)
        self.assertEqual(len(result["changed"]["key"]["before"]), 2)
        self.assertEqual(len(result["changed"]["key"]["after"]), 1)

    def test_disappearing_warning_does_not_claim_security_fix(self):
        difference = comparison.compare({"abc": {"path": "app.go"}}, {})
        self.assertIn("abc", difference["disappeared"])
        self.assertEqual(difference["status"], "DIFF_ONLY_NOT_SECURITY_CONFIRMATION")

    def test_protocol_freeze_is_immutable(self):
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary) / "source"
            project.mkdir()
            run("git", "-C", project, "init", "-q", "-b", "main")
            run("git", "-C", project, "config", "user.name", "Tests")
            run("git", "-C", project, "config", "user.email", "test@example.invalid")
            (project / "app.go").write_text("package app\n")
            run("git", "-C", project, "add", ".")
            run("git", "-C", project, "commit", "-qm", "source")
            destination = Path(temporary) / "frozen.json"
            digest = protocol.freeze([project], ["authorization", "parsing"], destination)
            self.assertEqual(len(digest), 64)
            self.assertNotIn("CVE", destination.read_text())
            with self.assertRaises(FileExistsError):
                protocol.freeze([project], ["authorization"], destination)


@unittest.skipUnless(BIN.is_file(), "build the Rust binary for Python wrapper checks")
class PythonCliWrappers(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="excession-python-wrapper-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "source"
        run("git", "clone", "-q", BUNDLE, self.repo)
        self.report = self.root / "report.json"
        self.db = self.root / "findings.sqlite"

    def semgrep(self):
        return {
            "version": "1.0.0", "errors": [], "results": [{
                "check_id": "controlled/test", "path": "h11/_connection.py",
                "start": {"line": 20},
                "extra": {"message": "unverified candidate"},
            }],
        }

    def pipeline(self, action: str, *options: object, check: bool = True):
        return run(
            sys.executable, ROOT / "tools/pipeline.py", action,
            "--engine", "semgrep", "--repo", self.repo,
            "--report", self.report, "--evidence", self.db,
            *options, "--rust", BIN, check=check,
        )

    def test_pipeline_import_dispatches_and_audits(self):
        self.report.write_text(json.dumps(self.semgrep()))
        result = json.loads(self.pipeline("import").stdout)
        self.assertEqual(result["receipt"]["candidate_count"], 1)
        self.assertTrue(result["receipt"]["independently_replayed"])

    def test_pipeline_refuses_evidence_inside_source(self):
        self.report.write_text(json.dumps(self.semgrep()))
        result = run(
            sys.executable, ROOT / "tools/pipeline.py", "import",
            "--engine", "semgrep", "--repo", self.repo,
            "--report", self.report, "--evidence", self.repo / "bad.sqlite",
            "--rust", BIN, check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.repo / "bad.sqlite").exists())

    def test_pipeline_scan_uses_python_scanner_protocol(self):
        executable = self.root / "fake-semgrep"
        executable.write_text(
            f"#!{sys.executable}\n"
            "import json, sys\n"
            "from pathlib import Path\n"
            "assert '--metrics=off' in sys.argv and '--json' in sys.argv\n"
            "assert sys.argv[-1] == '.'\n"
            "out = Path(sys.argv[sys.argv.index('--output')+1])\n"
            "out.write_text(json.dumps({'results':[{'check_id':'local.rule',"
            "'path':'h11/_connection.py','start':{'line':20},"
            "'extra':{'message':'candidate'}}],'errors':[]}))\n"
        )
        executable.chmod(0o755)
        config = self.root / "rules.yaml"
        config.write_text("rules: []\n")
        result = json.loads(self.pipeline(
            "scan", "--config", config, "--executable", executable,
        ).stdout)
        self.assertEqual(result["receipt"]["candidate_count"], 1)

    def test_python_virtual_browser_wraps_native_navigation(self):
        index = self.root / "index.sqlite"
        run(BIN, "index", "--repo", self.repo, "--db", index,
            "--name", "h11", "--rev", "HEAD", "--roots", "h11,tests")
        view = self.root / "view.json"
        view.write_text(run(
            BIN, "view", "deps", "--db", index, "--name", "h11",
            "--rev", "HEAD", "--seed", "h11._connection",
        ).stdout)
        browse = lambda instructions: run(
            sys.executable, ROOT / "tools/view_browser.py",
            self.repo, view, "--rust", BIN, check=False, input=instructions,
        )
        accepted = browse("ls\ncd views\nls\ncd 00-focus\npwd\nls\nquit\n")
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertIn("'views'/", accepted.stdout)
        self.assertIn("'00-focus'/", accepted.stdout)
        self.assertIn("/views/00-focus", accepted.stdout)
        self.assertFalse((self.repo / "views").exists())
        refused = browse("cd ..\n")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("above virtual root", refused.stderr)


if __name__ == "__main__":
    unittest.main()
