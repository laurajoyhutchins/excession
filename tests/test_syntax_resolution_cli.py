"""Real-language pinned-Git candidate resolution, replay and refusal fixtures."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"


def run(*args, check=True):
    process = subprocess.run(list(map(str, args)), capture_output=True, check=False)
    if check and process.returncode:
        raise AssertionError(f"{args}: {process.stderr.decode(errors='replace')}")
    return process


@unittest.skipUnless(BIN.is_file() and
                     importlib.util.find_spec("tree_sitter_language_pack"),
                     "requires built CLI and preinstalled Tree-sitter grammars")
class SyntaxResolutionCLI(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="excession-resolve-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.repo = self.base / "git"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q", "-b", "main")
        run("git", "-C", self.repo, "config", "user.name", "Syntax fixtures")
        run("git", "-C", self.repo, "config", "user.email", "syntax@example.invalid")
        files = {
            "go.mod": "module example.org/test\n\ngo 1.23\n",
            "src/go/main.go": 'package goapp\nimport "example.org/test/src/gohelper"\n'
                              'func answer() int { return gohelper.Answer }\n',
            "src/gohelper/helper.go": "package gohelper\nconst Answer = 42\n",
            "src/gohelper/helper_test.go": "package gohelper\nfunc TestUnused() {}\n",
            "src/main.rs": "mod widget;\nuse crate::widget::answer;\n"
                           "fn main() { let _ = answer(); }\n",
            "src/widget.rs": "pub fn answer() -> u32 { 42 }\n",
            "src/ui.ts": "import {value} from './util';\n"
                         "import foo from 'external-package';\nexport const out=value;\n",
            "src/util.ts": "export const value=42;\n",
            "src/ambiguous.ts": "export const answer=2;\n",
            "src/ambiguous.js": "export const answer=3;\n",
            "src/ambiguous_user.ts": "import './ambiguous';\n",
            "src/pkg/__init__.py": "",
            "src/pkg/entry.py": "import pkg.util\n",
            "src/pkg/util.py": "answer = 42\n",
        }
        for path, contents in files.items():
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(contents)
        run("git", "-C", self.repo, "add", ".")
        run("git", "-C", self.repo, "commit", "-qm", "candidate fixtures")
        self.commit = run("git", "-C", self.repo, "rev-parse", "HEAD").stdout.decode().strip()
        self.syntax_path = self.base / "syntax.json"
        self.resolution_path = self.base / "resolution.json"
        self.syntax_path.write_bytes(run(
            BIN, "syntax", "scan", "--repo", self.repo, "--rev", self.commit,
            "--roots", "src").stdout)

    def resolve(self):
        output = run(BIN, "syntax", "resolve", "--repo", self.repo,
                     "--report", self.syntax_path)
        self.resolution_path.write_bytes(output.stdout)
        return json.loads(output.stdout)

    def test_candidates_are_source_bound_with_fail_closed_replay(self):
        report = self.resolve()
        self.assertEqual(report["schema"], "excession.syntax-dependencies.v1")
        self.assertEqual(report["source_commit"], self.commit)
        self.assertEqual(report["completeness"], "CONSERVATIVE_CANDIDATES_ONLY")
        self.assertEqual(report["candidate_count"], len(report["candidate_edges"]))
        edges = {(x["source_path"], x["target_path"]) for x in report["candidate_edges"]}
        for source, target in [
            ("src/ui.ts", "src/util.ts"),
            ("src/pkg/entry.py", "src/pkg/util.py"),
            ("src/go/main.go", "src/gohelper/helper.go"),
            ("src/main.rs", "src/widget.rs"),
        ]:
            self.assertIn((source, target), edges)
        self.assertNotIn(("src/go/main.go", "src/gohelper/helper_test.go"), edges)
        self.assertFalse(any(x["source_path"] == "src/ambiguous_user.ts"
                             for x in report["candidate_edges"]))
        unresolved = {(x["source_path"], x["reason"]) for x in report["unresolved_imports"]}
        self.assertIn(("src/ambiguous_user.ts", "AMBIGUOUS_TRACKED_TARGETS"), unresolved)
        self.assertIn(("src/ui.ts", "BARE_OR_ALIAS_IMPORT"), unresolved)

        for edge in report["candidate_edges"]:
            for prefix in ("source", "target"):
                expected = run("git", "-C", self.repo, "rev-parse",
                               f"{self.commit}:{edge[prefix + '_path']}").stdout.decode().strip()
                self.assertEqual(expected, edge[prefix + "_blob"])
        replay = json.loads(run(BIN, "syntax", "resolve-audit",
                                "--repo", self.repo, "--syntax-report", self.syntax_path,
                                "--report", self.resolution_path).stdout)
        self.assertEqual(replay["status"], "REGENERATED_MATCH")
        report["candidate_edges"][0]["target_blob"] = "0" * 40
        self.resolution_path.write_text(json.dumps(report))
        rejected = run(BIN, "syntax", "resolve-audit", "--repo", self.repo,
                       "--syntax-report", self.syntax_path,
                       "--report", self.resolution_path, check=False)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn(b"REFUSED:", rejected.stderr)

    def test_forged_syntax_report_cannot_produce_dependencies(self):
        source = json.loads(self.syntax_path.read_text())
        source["sources"][0]["source_blob"] = "0" * 40
        self.syntax_path.write_text(json.dumps(source))
        result = run(BIN, "syntax", "resolve", "--repo", self.repo,
                     "--report", self.syntax_path, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"REFUSED:", result.stderr)

    def test_checkout_dirty_or_changed_revision_refused(self):
        self.resolve()
        (self.repo / "src" / "untracked.rs").write_text("const X: u8=1;")
        result = run(BIN, "syntax", "resolve", "--repo", self.repo,
                     "--report", self.syntax_path, check=False)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
