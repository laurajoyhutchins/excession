"""Independent import oracle and native bounded-context contract tests."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
import dependency_recall as oracle  # noqa: E402


class SourceFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        subprocess.run(["git", "-C", str(self.repo), "init", "-q"], check=True)

    def add(self, path, data):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(data)

    def commit(self):
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True)
        subprocess.run([
            "git", "-C", str(self.repo), "-c", "user.name=Fixture",
            "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"
        ], check=True)
        commit = oracle.command(["git", "-C", self.repo, "rev-parse", "HEAD"]).decode().strip()
        return commit, oracle.inventory(self.repo, commit)


class TestIndependentImportOracle(SourceFixture):
    def test_python_ast_resolves_relative_and_absolute_local_imports(self):
        self.add("src/pkg/__init__.py", "")
        self.add("src/pkg/alpha.py", (
            "from .beta import a\nfrom pkg import gamma\n"
            "import pkg.beta\nimport external_dependency\n"
            "value = __import__('pkg.beta')\n"
        ))
        self.add("src/pkg/beta.py", "a = 1\n")
        self.add("src/pkg/gamma.py", "g = 2\n")
        commit, inventory = self.commit()
        src = {k for k in inventory if k.endswith(".py")}
        result = oracle.python_imports(self.repo, src, inventory)
        self.assertEqual(
            {tuple(p) for p in result["pairs"]},
            {("src/pkg/alpha.py", "src/pkg/beta.py"),
             ("src/pkg/alpha.py", "src/pkg/gamma.py")}
        )
        self.assertEqual(result["dynamic_or_nonliteral_excluded"], 1)
        self.assertGreaterEqual(result["unresolved_module_targets"], 1)
        self.assertFalse(result["semantic_runtime_dependencies_proven"])

    def test_git_blob_substitution_refused(self):
        self.add("src/pkg/__init__.py", "")
        self.add("src/pkg/a.py", "from . import b\n")
        self.add("src/pkg/b.py", "x = 1\n")
        _, inventory = self.commit()
        (self.repo / "src/pkg/b.py").write_text("x = 2\n")
        with self.assertRaisesRegex(ValueError, "dirty or substituted"):
            oracle.python_imports(self.repo, set(inventory), inventory)

    def test_native_directed_pairs_are_checked_independently(self):
        def native(source, target, observed=True):
            return {
                "kind": "SCIP_REFERENCE" if observed else "STATIC_IMPORT",
                "source": {"path": source}, "target": {"path": target},
                "evidence": [{"assessment": "OBSERVED" if observed else "CANDIDATE"}],
            }
        all_rows = [
            native("a.py", "b.py", False),
            native("a.py", "c.py", True),
            native("a.py", "c.py", True),
            native("a.py", "d.py", True),
        ]
        gold = {("a.py", "c.py"), ("a.py", "d.py")}
        found = oracle.independent_priority(all_rows, 2)
        measured = oracle.score(all_rows, found, gold, "a.py", 2)
        self.assertEqual(measured["old_import_confirmed"], 1)
        self.assertEqual(measured["new_import_confirmed"], 2)
        self.assertEqual(measured["new_import_recall"], 1.0)
        self.assertEqual(measured["old_import_recall"], 0.5)
        self.assertFalse(measured["unclassified_native_edges_are_false_positives"])
        with self.assertRaisesRegex(ValueError, "disagrees"):
            oracle.score(all_rows, all_rows[:2], gold, "a.py", 2)

    def test_type_script_compiler_resolves_relative_sources(self):
        if not (ROOT / "node_modules/typescript/package.json").is_file():
            self.skipTest("pinned npm toolchain not installed")
        self.add("lib/main.js", (
            "import util from './util.js';\n"
            "export {hi} from './other.js';\n"
            "const again = require('./util.js');\n"
            "const future = import('./other.js');\n"
        ))
        self.add("lib/util.js", "export default 1;\n")
        self.add("lib/other.js", "export const hi = 1;\n")
        _, inventory = self.commit()
        src = set(inventory)
        report = oracle.typescript_imports(
            self.repo, src, inventory, ROOT / "benchmarks/experiments/ts_import_oracle.cjs"
        )
        self.assertEqual(
            {tuple(p) for p in report["pairs"]},
            {("lib/main.js", "lib/util.js"),
             ("lib/main.js", "lib/other.js")}
        )
        self.assertEqual(report["dynamic_or_nonliteral_excluded"], 1)
        self.assertFalse(report["semantic_runtime_dependencies_proven"])


class TestIndependentSymbolSiteOracle(SourceFixture):
    def test_python_imported_functions_and_class_types_without_dynamic_guessing(self):
        from semantic_symbol_oracle import independent_python_sites
        self.add("src/pkg/__init__.py", "")
        self.add("src/pkg/functions.py", (
            "def compute():\n    return 1\n"
            "class Model:\n    pass\n"
        ))
        self.add("src/pkg/main.py", (
            "from .functions import compute, Model\n"
            "def run(value: Model) -> Model:\n"
            "    return compute()\n"
            "def unsafe(obj):\n"
            "    return obj.method()\n"
        ))
        _, inv = self.commit()
        source = {p for p in inv if p.endswith(".py")}
        result = independent_python_sites(self.repo, source, inv, oracle.checked_source)
        observed = {(s["source"], s["target"], s["line"], s["kind"])
                    for s in result["sites"]}
        self.assertIn(("src/pkg/main.py", "src/pkg/functions.py", 3, "call"), observed)
        self.assertIn(("src/pkg/main.py", "src/pkg/functions.py", 2, "type"), observed)
        self.assertGreater(result["unsupported"].get("dynamic_call", 0), 0)
        self.assertFalse(result["runtime_dispatch_proven"])

    def test_python_lexical_shadowing_fails_closed(self):
        from semantic_symbol_oracle import independent_python_sites
        self.add("src/pkg/__init__.py", "")
        self.add("src/pkg/functions.py", "def compute():\n    return 1\n")
        self.add("src/pkg/main.py", (
            "from .functions import compute\n"
            "def test(compute):\n    return compute()\n"
        ))
        _, inv = self.commit()
        result = independent_python_sites(
            self.repo, {p for p in inv if p.endswith(".py")}, inv,
            oracle.checked_source
        )
        self.assertFalse(result["sites"])
        self.assertGreater(result["unsupported"].get("shadowed_import_aliases", 0), 0)

    def test_exact_native_source_line_and_target_not_just_file_pair(self):
        from semantic_symbol_oracle import score_sites
        anchor = {"source": "a.py", "target": "b.py", "line": 7,
                  "kind": "call", "symbol": "run"}
        oracle_report = {"sites": [anchor]}
        def observed(line, target="b.py"):
            return {"kind": "SCIP_REFERENCE", "source": {"path": "a.py"},
                    "target": {"path": target},
                    "native_observation": {"line": line}}
        miss = score_sites(oracle_report, [observed(8)], [observed(8)], [observed(8)],
                           "a.py")["kinds"]["call"]
        self.assertEqual(miss["full_native_hits"], 0)
        hit = score_sites(oracle_report, [observed(7)], [],
                          [observed(7)], "a.py")["kinds"]["call"]
        self.assertEqual((hit["oracle_sites"], hit["old_bounded_hits"],
                          hit["new_bounded_hits"]), (1, 0, 1))
        with self.assertRaisesRegex(ValueError, "absent"):
            score_sites(oracle_report, [observed(8)], [], [observed(7)], "a.py")

    def test_typescript_symbol_resolution_excludes_instance_method_dispatch(self):
        if not (ROOT / "node_modules/typescript/package.json").is_file():
            self.skipTest("pinned TS compiler unavailable")
        self.add("lib/main.js", (
            "import {doWork, Model} from './work.js';\n"
            "const result = doWork();\n"
            "const model = new Model();\n"
            "model.dynamic();\n"
        ))
        self.add("lib/work.js", (
            "export function doWork() {return 1;}\n"
            "export class Model { dynamic() { return 1; } }\n"
        ))
        _, inv = self.commit()
        for p in inv:
            oracle.checked_source(self.repo, p, inv)
        import json
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "files.json"
            source.write_text(json.dumps(sorted(inv)))
            result = json.loads(oracle.command([
                "node", ROOT / "benchmarks/experiments/ts_symbol_oracle.cjs",
                self.repo, source
            ]))
        symbols = {(x["line"], x["kind"], x["target"]) for x in result["sites"]}
        self.assertIn((2, "call", "lib/work.js"), symbols)
        self.assertIn((3, "call", "lib/work.js"), symbols)
        self.assertNotIn((4, "call", "lib/work.js"), symbols)
        self.assertGreater(result["unsupported"]["instance_methods_excluded"], 0)


if __name__ == "__main__":
    unittest.main()
