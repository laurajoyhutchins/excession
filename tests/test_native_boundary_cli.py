"""Real grammar/Git fixtures for the compiler-declaration boundary.

A declaration is deliberately *unattested* test data. These tests exercise
identity checking and scope comparison, not compiler execution.
"""
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


def declaration(language, subject, paths, config):
    return {
        "schema": "excession.input-declaration.v1",
        "coverage": "DECLARED_INPUTS_ONLY",
        "language": language,
        "subject": subject,
        "inputs": [
            {"path": path, "role": "source"} for path in sorted(paths)
        ] + [{"path": path, "role": "configuration"} for path in sorted(config)],
        "assumptions": {
            "dependency-discovery": "excession-toolchain-native-v1",
            "toolchain-version": "fixture-resolver-1",
            "toolchain-sha256": "f" * 64,
            "platform": "fixture-linux",
        },
    }


@unittest.skipUnless(BIN.is_file() and
                     importlib.util.find_spec("tree_sitter_language_pack"),
                     "requires CLI, installed Tree-sitter grammars")
class NativeBoundaryCLI(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="excession-native-boundary-")
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.repo = base / "repo"
        self.repo.mkdir()
        run("git", "-C", self.repo, "init", "-q", "-b", "main")
        run("git", "-C", self.repo, "config", "user.name", "Excession fixture")
        run("git", "-C", self.repo, "config", "user.email", "fixture@example.invalid")
        fixtures = {
            "go.mod": "module example.org/local\n\ngo 1.23\n",
            "Cargo.toml": '[package]\nname = "fixture"\nversion = "0.1.0"\nedition = "2024"\n',
            "tsconfig.json": '{"compilerOptions": {"module": "esnext"}}\n',
            "src/go/main.go": ('package goapp\nimport "example.org/local/src/lib"\n'
                               'var X=lib.Answer\n'),
            "src/lib/lib.go": "package lib\nconst Answer=42\n",
            "src/main.rs": "mod math;\nuse crate::math::value;\nfn main() { let _ = value(); }\n",
            "src/math.rs": "pub fn value() -> u32 { 42 }\n",
            "src/ui.ts": "import { value } from './util';\nexport const answer = value;\n",
            "src/util.ts": "export const value = 42;\n",
            "src/app.js": "import { value } from './helper.js';\nconsole.log(value);\n",
            "src/helper.js": "export const value = 42;\n",
            "src/outside.py": "answer = 42\n",
        }
        for path, content in fixtures.items():
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content)
        run("git", "-C", self.repo, "add", ".")
        run("git", "-C", self.repo, "commit", "-qm", "replay inputs")
        self.rev = run("git", "-C", self.repo, "rev-parse", "HEAD").stdout.decode().strip()
        self.syntax = base / "syntax.json"
        self.resolution = base / "resolution.json"
        self.declaration = base / "declaration.json"
        self.boundary = base / "boundary.json"
        self.syntax.write_bytes(run(BIN, "syntax", "scan", "--repo", self.repo,
                                    "--rev", self.rev, "--roots", "src").stdout)
        self.resolution.write_bytes(run(BIN, "syntax", "resolve",
                                        "--repo", self.repo, "--report", self.syntax).stdout)

    def evaluate(self, data):
        self.declaration.write_text(json.dumps(data))
        result = run(BIN, "syntax", "native-compare", "--repo", self.repo,
                     "--syntax-report", self.syntax,
                     "--resolution-report", self.resolution,
                     "--declaration", self.declaration)
        self.boundary.write_bytes(result.stdout)
        return json.loads(result.stdout)

    def test_agreement_go_rust_ts_js_with_git_bound_provenance(self):
        scenarios = [
            ("go", "src/go/main.go",
             ["src/go/main.go", "src/lib/lib.go"], ["go.mod"],
             ("src/go/main.go", "src/lib/lib.go")),
            ("rust", "src/main.rs",
             ["src/main.rs", "src/math.rs"], ["Cargo.toml"],
             ("src/main.rs", "src/math.rs")),
            ("typescript", "src/ui.ts",
             ["src/ui.ts", "src/util.ts"], ["tsconfig.json"],
             ("src/ui.ts", "src/util.ts")),
            ("javascript", "src/app.js",
             ["src/app.js", "src/helper.js"], [],
             ("src/app.js", "src/helper.js")),
        ]
        for language, subject, inputs, config, pair in scenarios:
            with self.subTest(language=language):
                data = declaration(language, subject, inputs, config)
                report = self.evaluate(data)
                self.assertEqual(report["status"], "NO_STATIC_NATIVE_DISAGREEMENT_OBSERVED")
                self.assertEqual(report["provenance"], "UNATTESTED_NATIVE_DECLARATION")
                self.assertEqual(report["source_commit"], self.rev)
                edges = {(e["source_path"], e["target_path"]) for e in report["matching_edges"]}
                if language == "rust":
                    # Rust declarations can still be audited as input claims,
                    # but semantic reference edges now come from SCIP only.
                    self.assertEqual(edges, set())
                    self.assertEqual(report["unresolved_imports"], [])
                    self.assertIn("src/math.rs", report["native_only_sources"])
                else:
                    self.assertIn(pair, edges)
                self.assertEqual(report["static_only_edges"], [])
                for item in report["pinned_inputs"]:
                    oid = run("git", "-C", self.repo, "rev-parse",
                              f"{self.rev}:{item['path']}").stdout.decode().strip()
                    self.assertEqual(item["blob"], oid)
                audited = json.loads(run(
                    BIN, "syntax", "native-audit", "--repo", self.repo,
                    "--syntax-report", self.syntax, "--resolution-report", self.resolution,
                    "--declaration", self.declaration,
                    "--report", self.boundary).stdout)
                self.assertEqual(audited["status"], "DECLARATION_AND_GIT_REPLAY_MATCH")

    def test_missing_native_target_is_reviewable_not_a_proven_bug(self):
        report = self.evaluate(declaration(
            "go", "src/go/main.go", ["src/go/main.go"], ["go.mod"]
        ))
        self.assertEqual(report["status"], "REVIEW_STATIC_NATIVE_SCOPE_DIFFERENCE")
        self.assertEqual([e["target_path"] for e in report["static_only_edges"]],
                         ["src/lib/lib.go"])
        self.assertEqual(report["coverage"], "SCOPED_COMPARISON_ONLY")

    def test_undeclared_gitrevision_path_refused_before_graph_output(self):
        data = declaration("go", "src/go/main.go",
                           ["src/go/main.go", "src/ghost.go"], ["go.mod"])
        self.declaration.write_text(json.dumps(data))
        result = run(BIN, "syntax", "native-compare", "--repo", self.repo,
                     "--syntax-report", self.syntax,
                     "--resolution-report", self.resolution,
                     "--declaration", self.declaration, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"REFUSED:", result.stderr)

    def test_fast_preflight_runs_without_repository_or_grammars(self):
        data = declaration("rust", "src/main.rs", ["src/main.rs"], [])
        self.declaration.write_text(json.dumps(data))
        result = run(BIN, "syntax", "native-preflight",
                     "--declaration", self.declaration)
        self.assertEqual(json.loads(result.stdout)["status"], "DECLARATION_SHAPE_ONLY")
        data["assumptions"]["toolchain-sha256"] = "malformed"
        self.declaration.write_text(json.dumps(data))
        rejected = run(BIN, "syntax", "native-preflight",
                       "--declaration", self.declaration, check=False)
        self.assertNotEqual(rejected.returncode, 0)

    def test_boundary_audit_rejects_tampering(self):
        result = self.evaluate(declaration(
            "go", "src/go/main.go", ["src/go/main.go", "src/lib/lib.go"], ["go.mod"]
        ))
        result["matching_edges"][0]["target_blob"] = "0" * 40
        self.boundary.write_text(json.dumps(result))
        rejected = run(BIN, "syntax", "native-audit", "--repo", self.repo,
                       "--syntax-report", self.syntax,
                       "--resolution-report", self.resolution,
                       "--declaration", self.declaration,
                       "--report", self.boundary, check=False)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn(b"REFUSED:", rejected.stderr)


if __name__ == "__main__":
    unittest.main()
