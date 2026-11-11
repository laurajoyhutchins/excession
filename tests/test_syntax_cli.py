"""End-to-end tests: actual Tree-sitter grammars against a pinned Git checkout."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"


def run(*args, check=True):
    result = subprocess.run(list(map(str, args)), capture_output=True, check=False)
    if check and result.returncode:
        raise AssertionError(
            f"{args}: exit {result.returncode}\n"
            + result.stderr.decode(errors="replace")
        )
    return result


@unittest.skipUnless(BIN.is_file() and
                     importlib.util.find_spec("tree_sitter_language_pack"),
                     "build CLI and install optional parser package first")
class SourceBoundSyntaxTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-syntax-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / "source"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q", "-b", "main")
        run("git", "-C", self.repo, "config", "user.name", "Syntax test")
        run("git", "-C", self.repo, "config", "user.email", "syntax@example.invalid")
        (self.repo / "src").mkdir()
        fixtures = {
            "src/main.rs": 'pub fn answer() -> u32 { 42 }\n',
            "src/main.go": 'package main\nimport "fmt"\nfunc main() { fmt.Println("hi") }\n',
            "src/Main.java": 'class Main { static int answer() { return 42; } }\n',
            "src/main.c": '#include <stdio.h>\nint answer(void) { return 42; }\n',
            "src/ignored.bin": "arbitrary",
        }
        for name, contents in fixtures.items():
            (self.repo / name).write_text(contents)
        run("git", "-C", self.repo, "add", ".")
        run("git", "-C", self.repo, "commit", "-qm", "fixture")
        self.commit = run("git", "-C", self.repo, "rev-parse", "HEAD").stdout.decode().strip()

    def scan(self):
        return run(BIN, "syntax", "scan", "--repo", self.repo,
                   "--rev", self.commit, "--roots", "src")

    def test_cross_language_bound_to_git_and_auditable(self):
        report = json.loads(self.scan().stdout)
        self.assertEqual(report["schema"], "excession.syntax-observations.v1")
        self.assertEqual(report["source_commit"], self.commit)
        self.assertEqual(report["source_count"], 4)
        self.assertEqual(len(report["report_id"]), 64)
        sources = {item["source_path"]: item for item in report["sources"]}
        self.assertEqual(set(sources), {
            "src/main.rs", "src/main.go", "src/Main.java", "src/main.c"
        })
        for path, item in sources.items():
            expected_blob = run("git", "-C", self.repo,
                                "rev-parse", f"{self.commit}:{path}").stdout.decode().strip()
            self.assertEqual(item["source_blob"], expected_blob)
            self.assertEqual(item["status"], "PARSED", path)
            self.assertTrue(item["language"], path)
        self.assertTrue(sources["src/main.rs"]["structure"])
        self.assertTrue(sources["src/main.go"]["imports"])
        saved = self.root / "report.json"
        saved.write_text(json.dumps(report))
        audited = json.loads(run(BIN, "syntax", "audit", "--repo", self.repo,
                                 "--report", saved).stdout)
        self.assertEqual(audited["status"], "REPARSED_MATCH")
        self.assertEqual(audited["report_id"], report["report_id"])

        report["sources"][0]["source_blob"] = "0" * 40
        saved.write_text(json.dumps(report))
        rejected = run(BIN, "syntax", "audit", "--repo", self.repo,
                       "--report", saved, check=False)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn(b"REFUSED:", rejected.stderr)

    def test_scan_is_bounded_and_fails_on_dirty_checkout(self):
        result = run(BIN, "syntax", "scan", "--repo", self.repo,
                     "--roots", "src", "--rev", self.commit)
        self.assertEqual(result.returncode, 0)
        (self.repo / "src" / "untracked.rs").write_text("fn unknown() {}")
        rejected = run(BIN, "syntax", "scan", "--repo", self.repo,
                       "--roots", "src", check=False)
        # The command must fail rather than accidentally reading a dirty checkout.
        self.assertNotEqual(rejected.returncode, 0)


if __name__ == "__main__":
    unittest.main()
