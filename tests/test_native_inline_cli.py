"""End-to-end Rust-owned native resolver -> Git-backed syntax boundary.

Unlike the declaration-only boundary fixtures, these checks invoke actual
installed language toolchains on a private snapshot of a pinned Git revision.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"
NPM_BIN = ROOT / "node_modules" / ".bin"


def run(*args, check=True, env=None):
    proc = subprocess.run(list(map(str, args)), capture_output=True,
                          check=False, env=env)
    if check and proc.returncode:
        raise AssertionError(
            f"{args} exited {proc.returncode}:\n"
            + proc.stderr.decode(errors="replace")
        )
    return proc


@unittest.skipUnless(
    BIN.is_file() and importlib.util.find_spec("tree_sitter_language_pack"),
    "requires compiled Rust CLI and prefetched Tree-sitter grammars",
)
class NativeInlineCLI(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="excession-inline-native-")
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name)
        self.repo = self.base / "repo"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q", "-b", "main")
        run("git", "-C", self.repo, "config", "user.email", "test@example.invalid")
        run("git", "-C", self.repo, "config", "user.name", "Native tests")
        files = {
            "go.mod": "module example.org/inline\n\ngo 1.23\n",
            "src/go/main.go": (
                'package goapp\nimport "example.org/inline/src/dep"\n'
                "var A = dep.Answer\n"
            ),
            "src/dep/dep.go": "package dep\nconst Answer = 42\n",
            "Cargo.toml": (
                '[package]\nname = "native-inline"\nversion = "0.1.0"\n'
                'edition = "2024"\n'
            ),
            "src/main.rs": (
                "mod math;\nuse crate::math::value;\n"
                'fn main() { let _ = value(); let _ = include_str!("data.txt"); }\n'
            ),
            "src/math.rs": "pub fn value() -> u32 { 42 }\n",
            "src/data.txt": "source-bound asset\n",
            "tsconfig.json": json.dumps({
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {"@/*": ["lib/*"]},
                },
            }),
            "src/ui.ts": 'import { value } from "@/math";\nexport const answer = value;\n',
            "lib/math.ts": "export const value = 42;\n",
            "src/app.js": 'const {value} = require("./helper.js");\nconsole.log(value);\n',
            "src/helper.js": "module.exports = {value: 42};\n",
            "src/pypkg/__init__.py": 'raise RuntimeError("never execute this source")\n',
            "src/pypkg/dependency.py": "VALUE = 42\n",
            "src/pypkg/entry.py": (
                "from pypkg import dependency\n"
                "from . import dependency\n"
                "import pypkg.dependency\n"
                "import os\n"
                "import importlib\n"
                "importlib.import_module('pypkg.dependency')\n"
                "computed = 'pypkg.' + 'dependency'\n"
                "__import__(computed)\n"
            ),
        }
        for path, content in files.items():
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content)
        run("git", "-C", self.repo, "add", "-A")
        run("git", "-C", self.repo, "commit", "-qm", "pinned native fixture")
        self.rev = run("git", "-C", self.repo, "rev-parse", "HEAD").stdout.decode().strip()
        self.syntax = self.base / "syntax.json"
        self.resolution = self.base / "resolution.json"
        self.syntax.write_bytes(run(BIN, "syntax", "scan", "--repo", self.repo,
                                    "--rev", self.rev, "--roots", "src").stdout)
        self.resolution.write_bytes(run(
            BIN, "syntax", "resolve", "--repo", self.repo,
            "--report", self.syntax,
        ).stdout)

    def invoke(self, entry, language, *, check=True, trust="true", roots=None):
        env = dict(os.environ)
        if NPM_BIN.is_dir():
            env["PATH"] = str(NPM_BIN) + os.pathsep + env.get("PATH", "")
        return run(
            BIN, "syntax", "native-run", "--repo", self.repo,
            "--syntax-report", self.syntax,
            "--resolution-report", self.resolution,
            "--entry", entry, "--language", language,
            "--trusted-local", trust,
            *([] if roots is None else ["--import-roots", roots]),
            check=check, env=env,
        )

    def assert_valid(self, result, language, entry):
        self.assertEqual(result["schema"], "excession.native-syntax-inline.v1")
        self.assertEqual(result["status"], "LOCAL_NATIVE_DISCOVERY_AND_BOUNDARY_CHECK")
        self.assertEqual(result["execution"], "TRUSTED_LOCAL_NOT_ATTESTED")
        self.assertEqual(result["stats"]["commit"], self.rev)
        self.assertEqual(result["declaration"]["subject"], entry)
        self.assertEqual(result["declaration"]["language"], language)
        self.assertEqual(result["declaration"]["coverage"], "DECLARED_INPUTS_ONLY")
        self.assertEqual(result["boundary"]["source_commit"], self.rev)
        self.assertEqual(result["boundary"]["declaration_id"],
                         self.canonical_sha256(result["declaration"]))
        return result["boundary"]

    @staticmethod
    def canonical_sha256(body):
        import hashlib
        # Excession's canonical JSON serializer escapes non-ASCII, but these
        # fixture declaration fields are ASCII by construction.
        return hashlib.sha256(json.dumps(
            body, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
        ).encode()).hexdigest()

    def test_python_native_resolution_authenticated_without_import_execution(self):
        result = json.loads(self.invoke(
            "src/pypkg/entry.py", "python", roots="src"
        ).stdout)
        boundary = self.assert_valid(result, "python", "src/pypkg/entry.py")
        inputs = {x["path"] for x in boundary["pinned_inputs"]}
        self.assertTrue({
            "src/pypkg/entry.py",
            "src/pypkg/__init__.py",
            "src/pypkg/dependency.py",
        } <= inputs)
        evidence = result["resolution"]
        self.assertEqual(evidence["schema"], "excession.python-resolution.v1")
        self.assertEqual(evidence["source_commit"], self.rev)
        self.assertEqual(evidence["import_roots"], "src")
        self.assertEqual(result["declaration"]["assumptions"]["python-runtime-coverage"],
                         "NOT_ESTABLISHED")
        statuses = {x["status"] for x in evidence["observations"]}
        self.assertTrue({"RESOLVED", "CANDIDATE", "UNRESOLVED", "UNSUPPORTED"} <= statuses)
        self.assertEqual(len(evidence["report_id"]), 64)
        self.assertIn("src/pypkg/dependency.py", [
            x["path"] for x in boundary["pinned_inputs"]
        ])

        declaration = self.base / "python-native-declaration.json"
        report = self.base / "python-native-boundary.json"
        declaration.write_text(json.dumps(result["declaration"]))
        report.write_text(json.dumps(boundary))
        audit = json.loads(run(
            BIN, "syntax", "native-audit", "--repo", self.repo,
            "--syntax-report", self.syntax,
            "--resolution-report", self.resolution,
            "--declaration", declaration, "--report", report,
        ).stdout)
        self.assertEqual(audit["status"], "DECLARATION_AND_GIT_REPLAY_MATCH")

        missing = self.invoke("src/pypkg/entry.py", "python", check=False)
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn(b"--import-roots", missing.stderr)
        invalid = self.invoke("src/pypkg/entry.py", "python",
                              roots="../outside", check=False)
        self.assertNotEqual(invalid.returncode, 0)

if __name__ == "__main__":
    unittest.main()
