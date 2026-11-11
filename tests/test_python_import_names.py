"""Language-native Python import-name regressions.

The indexed module name is an immutable source-path identity, not necessarily
the importable name. These tests specify expected edges independently of the
adapter's alias inference.
"""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "core/adapters/python_ast.py"


def adapter(files: dict[str, str]) -> subprocess.CompletedProcess[str]:
    entries = []
    for path, content in files.items():
        stem = path.removesuffix(".py").replace("/", ".")
        name = stem.removesuffix(".__init__")
        entries.append({"name": name, "path": path, "content": content})
    return subprocess.run(
        [sys.executable, "-c", ADAPTER.read_text()],
        input=json.dumps(entries), capture_output=True,
        text=True, check=False, timeout=30,
    )


def edges(result: subprocess.CompletedProcess[str]) -> set[tuple[str, str, int, str]]:
    if result.returncode:
        raise AssertionError(result.stderr)
    return {
        (module["name"], ref["target"], ref["line"], ref["syntax"])
        for module in json.loads(result.stdout)
        for ref in module["imports"]
    }


class PackageImportNameTests(unittest.TestCase):
    def test_arbitrary_regular_package_root_and_nested_namespace(self):
        files = {
            "tree/build/lib/acme/__init__.py": "# package\n",
            "tree/build/lib/acme/tools.py": "VALUE = 1\n",
            "tree/build/lib/acme/nested/leaf.py": "VALUE = 2\n",
            "tree/build/lib/acme/consumer.py": (
                "from acme import tools\n"
                "from . import tools\n"
                "import acme.nested.leaf\n"
                "from acme.nested import leaf\n"
            ),
        }
        name = "tree.build.lib.acme"
        self.assertEqual(edges(adapter(files)), {
            (name + ".consumer", name, 1, "from"),
            (name + ".consumer", name + ".tools", 1, "from"),
            (name + ".consumer", name, 2, "from"),
            (name + ".consumer", name + ".tools", 2, "from"),
            (name + ".consumer", name + ".nested.leaf", 3, "import"),
            (name + ".consumer", name, 4, "from"),
            (name + ".consumer", name + ".nested.leaf", 4, "from"),
        })

    def test_outer_tracked_package_prevents_arbitrary_prefix_stripping(self):
        files = {
            "workspace/__init__.py": "# parent is itself a package\n",
            "workspace/lib/__init__.py": "# nested package\n",
            "workspace/lib/helper.py": "VALUE = 1\n",
            "workspace/lib/consumer.py": (
                "from workspace.lib import helper\n"
                "from . import helper\n"
            ),
        }
        self.assertEqual(edges(adapter(files)), {
            ("workspace.lib.consumer", "workspace.lib", 1, "from"),
            ("workspace.lib.consumer", "workspace.lib.helper", 1, "from"),
            ("workspace.lib.consumer", "workspace.lib", 2, "from"),
            ("workspace.lib.consumer", "workspace.lib.helper", 2, "from"),
        })

    def test_conflicting_package_names_refuse_instead_of_picking_one(self):
        files = {
            "vendor/alpha/pkg/__init__.py": "# one package\n",
            "vendor/beta/pkg/__init__.py": "# another package\n",
            "vendor/alpha/pkg/x.py": "VALUE = 1\n",
            "vendor/beta/pkg/x.py": "VALUE = 2\n",
        }
        result = adapter(files)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ambiguous importable Python module", result.stderr)

    def test_missing_package_marker_does_not_guess_an_import_root(self):
        files = {
            "project/vendor/helper.py": "VALUE = 1\n",
            "project/app/__init__.py": "# package\n",
            "project/app/main.py": "import helper\n",
        }
        self.assertEqual(edges(adapter(files)), set())


if __name__ == "__main__":
    unittest.main()
