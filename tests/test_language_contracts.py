"""Actual Git histories for language-neutral declared input transport.

Each test runs an ordinary language-native oracle separately. Excession only
asserts Git input identity and refuses behavioral/effect equivalence claims.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import unittest

BIN = Path(__file__).resolve().parents[1] / "core/target/debug/excession"


def command(*args: str, cwd: Path | None = None, env: dict | None = None, ok: bool = True) -> str:
    p = subprocess.run([str(a) for a in args], cwd=cwd, capture_output=True, text=True, env=env)
    if ok and p.returncode:
        raise AssertionError(f"command {args} exited {p.returncode}: {p.stderr}")
    if not ok and not p.returncode:
        raise AssertionError(f"command {args} incorrectly passed")
    return p.stdout if ok else p.stderr


class InputContractHistory(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-input-contract-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / "source"
        self.repo.mkdir()
        self.db = self.root / "evidence.sqlite"
        self.counter = 0
        command("git", "init", "-q", cwd=self.repo)
        command("git", "config", "user.email", "test@example.invalid", cwd=self.repo)
        command("git", "config", "user.name", "Test", cwd=self.repo)

    def write(self, path: str, data: str) -> None:
        file = self.repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(data)

    def git(self, *args: str) -> str:
        return command("git", *args, cwd=self.repo).strip()

    def commit(self, message: str) -> str:
        self.git("add", "-A")
        self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def declaration(self, language: str, subject: str, input_paths: dict[str, str],
                    assumptions: dict[str, str] | None = None) -> Path:
        data = {"schema": "excession.input-declaration.v1",
                "coverage": "DECLARED_INPUTS_ONLY", "language": language,
                "subject": subject, "assumptions": assumptions or {"runtime": "pinned-fixture"},
                "inputs": [{"path": path, "role": input_paths[path]} for path in sorted(input_paths)]}
        self.counter += 1
        result = self.root / f"declaration-{self.counter}.json"
        result.write_text(json.dumps(data))
        return result

    def plan(self, revision: str, declaration: Path) -> tuple[Path, dict]:
        result = json.loads(command(BIN, "evidence", "contract-plan", "--repo", self.repo,
                                    "--rev", revision, "--declaration", declaration))
        self.counter += 1
        path = self.root / f"contract-{self.counter}.json"
        path.write_text(json.dumps(result))
        return path, result

    def compare(self, left: Path, right: Path, audit: bool = False, ok: bool = True) -> dict | str:
        operation = "contract-audit" if audit else "contract-compare"
        value = command(BIN, "evidence", operation, "--repo", self.repo,
                        "--source-contract", left, "--target-contract", right,
                        "--db", self.db, ok=ok)
        return json.loads(value) if ok else value

    @unittest.skipUnless(shutil.which("node"), "Node.js not installed")
    def test_javascript_esm_branch_and_dependency_change(self) -> None:
        self.write("package.json", '{"name":"contract-fixture","private":true,"type":"module"}\n')
        self.write("src/math.mjs", "export const value = () => 7;\n")
        self.write("tests/check.mjs", "import { strict as assert } from 'node:assert';\n"
                   "import { value } from '../src/math.mjs';\nassert.equal(value(),7);\n")
        base = self.commit("source fixture")
        inputs = {"package.json": "configuration", "src/math.mjs": "source", "tests/check.mjs": "source"}
        decl = self.declaration("javascript", "tests/check.mjs", inputs, {"runtime": "node-v22", "network": "off"})
        source, _ = self.plan(base, decl)
        command("node", "--test", "tests/check.mjs", cwd=self.repo)
        self.git("switch", "-qc", "docs")
        self.write("README.md", "unrelated documentation\n")
        docs = self.commit("docs")
        self.git("switch", "-qc", "implementation", base)
        self.write("src/math.mjs", "export const value = () => 8;\n")
        broken = self.commit("regression")
        target_docs, _ = self.plan(docs, decl)
        carried = self.compare(source, target_docs)
        self.assertEqual(carried["decision"], "DECLARED_INPUTS_MATCH_ONLY")
        self.assertFalse(carried["target_behavior_verified"])
        self.assertTrue(self.compare(source, target_docs, audit=True)["independently_audited"])
        target_broken, _ = self.plan(broken, decl)
        diff = self.compare(source, target_broken)
        self.assertEqual(diff["decision"], "REVERIFY_REQUIRED")
        self.assertEqual(diff["changed_input_paths"], ["src/math.mjs"])
        command("node", "--test", "tests/check.mjs", cwd=self.repo, ok=False)
        self.assertTrue(self.compare(source, target_broken, audit=True)["independently_audited"])

        altered_environment = self.declaration("javascript", "tests/check.mjs", inputs,
                                                {"runtime": "node-v23", "network": "off"})
        changed_env, _ = self.plan(docs, altered_environment)
        outcome = self.compare(source, changed_env)
        self.assertEqual(outcome["decision"], "REVERIFY_REQUIRED")
        self.assertFalse(outcome["assumptions_unchanged"])
        self.assertEqual(outcome["changed_input_paths"], [])

        with sqlite3.connect(self.db) as db:
            db.execute("UPDATE declared_input_transport SET comparison_json='{}' WHERE transport_id=?",
                       (carried["transport_id"],))
        self.compare(source, target_docs, audit=True, ok=False)

    @unittest.skipUnless(shutil.which("go"), "Go not installed")
    def test_go_historical_inputs_and_refusals(self) -> None:
        self.write("go.mod", "module example.com/contract\n\ngo 1.21\n")
        self.write("calc.go", "package calc\nfunc Value() int { return 7 }\n")
        self.write("calc_test.go", "package calc\nimport \"testing\"\nfunc TestValue(t *testing.T) { if Value()!=7 { t.Fatal(Value()) } }\n")
        base = self.commit("Go source")
        env = os.environ.copy()
        env.update({"GONOSUMDB": "*", "GOSUMDB": "off", "GOPROXY": "off", "GOTOOLCHAIN": "local"})
        command("go", "test", "./...", cwd=self.repo, env=env)
        declaration = self.declaration("go", "calc_test.go", {
            "calc.go": "source", "calc_test.go": "source", "go.mod": "configuration"},
            {"toolchain": "pinned-go", "goproxy": "off"})
        source, _ = self.plan(base, declaration)
        self.write("notes.md", "no input change\n")
        docs = self.commit("docs")
        same, _ = self.plan(docs, declaration)
        self.assertEqual(self.compare(source, same)["decision"], "DECLARED_INPUTS_MATCH_ONLY")
        self.write("calc.go", "package calc\nfunc Value() int { return 9 }\n")
        regression = self.commit("Go regression")
        changed, _ = self.plan(regression, declaration)
        result = self.compare(source, changed)
        self.assertEqual(result["changed_input_paths"], ["calc.go"])
        self.assertEqual(result["decision"], "REVERIFY_REQUIRED")
        command("go", "test", "./...", cwd=self.repo, env=env, ok=False)
        self.assertTrue(self.compare(source, changed, audit=True)["independently_audited"])
        forged = json.loads(changed.read_text())
        forged["entries"][0]["blob"] = "0" * 40
        forged_path = self.root / "forged.json"
        forged_path.write_text(json.dumps(forged))
        self.compare(source, forged_path, ok=False)
        command(BIN, "evidence", "contract-plan", "--repo", self.repo,
                "--rev", "--evil", "--declaration", declaration, ok=False)
        command(BIN, "evidence", "contract-plan", "--repo", self.repo,
                "--rev", "0" * 40, "--declaration", declaration, ok=False)

    def test_declared_input_roles_and_order_are_part_of_contract(self) -> None:
        self.write("app.ts", "export const x=1;\n")
        self.write("lock.json", "{}\n")
        base = self.commit("typescript fixture")
        one = self.declaration("typescript", "app.ts", {"app.ts": "source", "lock.json": "configuration"})
        two = self.declaration("typescript", "app.ts", {"app.ts": "source", "lock.json": "asset"})
        left, _ = self.plan(base, one)
        right, _ = self.plan(base, two)
        outcome = self.compare(left, right)
        self.assertEqual(outcome["changed_input_paths"], ["lock.json"])
        self.assertFalse(outcome["target_behavior_verified"])
        other = self.declaration("rust", "app.ts", {"app.ts": "source", "lock.json": "configuration"})
        third, _ = self.plan(base, other)
        changed_language = self.compare(left, third)
        self.assertEqual(changed_language["decision"], "REVERIFY_REQUIRED")
        self.assertFalse(changed_language["language_unchanged"])
        self.assertEqual(changed_language["changed_input_paths"], [])
        invalid = json.loads(one.read_text())
        invalid["inputs"] = list(reversed(invalid["inputs"]))
        p = self.root / "invalid-order.json"
        p.write_text(json.dumps(invalid))
        command(BIN, "evidence", "contract-plan", "--repo", self.repo,
                "--rev", base, "--declaration", p, ok=False)


if __name__ == "__main__":
    unittest.main()
