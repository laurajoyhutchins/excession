"""Configured Python native resolver: no target-source imports or setup execution."""
from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "native_language_adapter", ROOT / "tools/native_language_adapter.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PythonNativeResolver(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def write(self, path, content):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content)

    def discover(self, entry, roots):
        sources, config, assumptions, observations = MODULE.python(
            self.root, entry, roots
        )
        return sources, config, assumptions, observations

    def test_nested_src_and_namespace_package_without_code_execution(self):
        self.write("build/python/lib/acme/__init__.py",
                   'raise RuntimeError("project code must not execute")\n')
        self.write("build/python/lib/acme/dependency.py", "VALUE = 42\n")
        self.write("build/python/lib/acme/ns/extra.py", "EXTRA = 1\n")
        self.write("build/python/lib/acme/entry.py",
                   "from acme import dependency\n"
                   "from . import dependency\n"
                   "import acme.ns.extra\n"
                   "import importlib\n"
                   "importlib.import_module('acme.dependency')\n"
                   "if condition:\n"
                   "    import acme.ns.extra\n"
                   "name = 'acme.' + 'dependency'\n"
                   "__import__(name)\n")
        source, config, assumptions, obs = self.discover(
            "build/python/lib/acme/entry.py", "build/python/lib"
        )
        self.assertTrue({
            "build/python/lib/acme/entry.py",
            "build/python/lib/acme/__init__.py",
            "build/python/lib/acme/dependency.py",
            "build/python/lib/acme/ns/extra.py",
        } <= source)
        self.assertEqual(config, set())
        self.assertEqual(assumptions["python-runtime-coverage"], "NOT_ESTABLISHED")
        self.assertEqual(assumptions["python-import-roots"], "build/python/lib")
        statuses = {o["status"] for o in obs}
        self.assertTrue({"RESOLVED", "CANDIDATE", "UNRESOLVED", "UNSUPPORTED"} <= statuses)
        self.assertIn("computed dynamic module name",
                      {o["reason"] for o in obs})

    def test_namespace_spans_roots_and_explicit_path_order_controls_shadowing(self):
        self.write("first/app.py", "import ns.other\nimport pkg.helper\n")
        self.write("first/ns/first.py", "FIRST = True\n")
        self.write("second/ns/other.py", "OTHER = True\n")
        self.write("first/pkg/__init__.py", "# first package\n")
        self.write("first/pkg/helper.py", "VALUE = 1\n")
        self.write("second/pkg/__init__.py", "# second package\n")
        self.write("second/pkg/helper.py", "VALUE = 2\n")
        one, _, _, _ = self.discover("first/app.py", "first,second")
        two, _, _, _ = self.discover("first/app.py", "second,first")
        self.assertIn("second/ns/other.py", one)
        self.assertIn("second/ns/other.py", two)
        self.assertIn("first/pkg/helper.py", one)
        self.assertNotIn("second/pkg/helper.py", one)
        self.assertIn("second/pkg/helper.py", two)
        self.assertNotIn("first/pkg/helper.py", two)

    def test_resolution_agrees_with_cpython_pathfinder_without_importing_project(self):
        from importlib.machinery import PathFinder

        self.write("first/pkg/__init__.py",
                   'raise RuntimeError("no code execution")\n')
        self.write("first/pkg/util.py", "VALUE = 1\n")
        self.write("second/pkg/__init__.py", "# other copy\n")
        self.write("second/pkg/util.py", "VALUE = 2\n")
        self.write("first/ns/left.py", "VALUE = 3\n")
        self.write("second/ns/right.py", "VALUE = 4\n")

        def cpython(name, roots):
            paths = [str(self.root / root) for root in roots.split(",")]
            observed = []
            for n, part in enumerate(name.split(".")):
                spec = PathFinder.find_spec(part, paths)
                if spec is None:
                    return None
                if spec.origin is not None and spec.origin.endswith(".py"):
                    observed.append(Path(spec.origin).relative_to(self.root).as_posix())
                if n < len(name.split(".")) - 1:
                    if spec.submodule_search_locations is None:
                        return None
                    paths = list(spec.submodule_search_locations)
            return observed

        for roots in ("first,second", "second,first"):
            finder = MODULE.PythonFinder(
                self.root, MODULE.python_roots(self.root, roots)
            )
            for name in ("pkg", "pkg.util", "ns", "ns.left", "ns.right"):
                with self.subTest(roots=roots, name=name):
                    self.assertEqual(finder.resolve(name), cpython(name, roots))

    def test_shadowed_subject_refuses(self):
        self.write("first/main.py", "VALUE = 1\n")
        self.write("second/main.py", "VALUE = 2\n")
        with self.assertRaisesRegex(MODULE.Unsupported, "shadowed"):
            self.discover("second/main.py", "first,second")

    def test_metadata_is_bound_and_conflicting_root_refuses(self):
        self.write("src/app/__init__.py", "# package\n")
        self.write("src/app/main.py", "from . import util\n")
        self.write("src/app/util.py", "VALUE = 1\n")
        self.write("pyproject.toml",
                   '[tool.setuptools.package-dir]\n"" = "src"\n')
        source, config, _, _ = self.discover("src/app/main.py", "src")
        self.assertIn("pyproject.toml", config)
        self.assertIn("src/app/util.py", source)
        with self.assertRaisesRegex(MODULE.Unsupported, "conflict"):
            self.discover("src/app/main.py", ".")

    def test_root_does_not_leak_ambient_pythonpath(self):
        self.write("local/main.py", "import arbitrary_nonexistent_package\n")
        sources, _, _, obs = self.discover("local/main.py", "local")
        self.assertEqual(sources, {"local/main.py"})
        self.assertEqual(obs[0]["status"], "UNRESOLVED")
        for roots in ("", "../local", "/tmp", "local,local", ".,local"):
            with self.subTest(roots=roots):
                with self.assertRaises(MODULE.Unsupported):
                    self.discover("local/main.py", roots)

    def test_wildcard_and_runtime_import_hooks_are_not_claimed_complete(self):
        self.write("src/pkg/__init__.py", "__all__ = ['mod']\n")
        self.write("src/pkg/mod.py", "VALUE = 1\n")
        self.write("src/main.py",
                   "from pkg import *\n"
                   "import sys\n"
                   "sys.path.append('/tmp')\n"
                   "eval('import pkg.mod')\n")
        source, _, assumptions, obs = self.discover("src/main.py", "src")
        self.assertIn("src/pkg/__init__.py", source)
        self.assertTrue(any(x["status"] == "UNSUPPORTED" and
                            "wildcard" in x["reason"] for x in obs))
        self.assertTrue(any("runtime import machinery" in x["reason"]
                            for x in obs))
        self.assertGreater(int(assumptions["python-unsupported-count"]), 0)


if __name__ == "__main__":
    unittest.main()
