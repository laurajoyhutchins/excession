"""Ground-truth smoke cases for independent Python import extraction."""
from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks" / "experiments"))
from dependency_oracle import python_edges  # noqa: E402


class LiteralImportCoverage(unittest.TestCase):
    def setUp(self):
        self.modules = {
            "src.pkg": "src/pkg/__init__.py",
            "src.pkg.first": "src/pkg/first.py",
            "src.pkg.second": "src/pkg/second.py",
            "src.pkg.third": "src/pkg/third.py",
        }
        self.sources = {
            "src/pkg/__init__.py": "# package\n",
            "src/pkg/first.py": (
                "from pkg import second\n"
                "from . import third\n"
                "import pkg.third\n"
            ),
            "src/pkg/second.py": "VALUE = 2\n",
            "src/pkg/third.py": "VALUE = 3\n",
        }
        self.expected = {
            ("src.pkg.first", "src.pkg", 1, "from"),
            ("src.pkg.first", "src.pkg.second", 1, "from"),
            ("src.pkg.first", "src.pkg", 2, "from"),
            ("src.pkg.first", "src.pkg.third", 2, "from"),
            ("src.pkg.first", "src.pkg.third", 3, "import"),
        }

    def test_independent_oracle_recognizes_absolute_and_relative_imports(self):
        found, unresolved = python_edges(self.sources, self.modules)
        self.assertEqual(found, self.expected)
        self.assertEqual(unresolved, [])

    def test_production_python_adapter_recovers_the_same_edges(self):
        items = [
            {"name": name, "path": path, "content": self.sources[path]}
            for name, path in self.modules.items()
        ]
        proc = subprocess.run(
            [sys.executable, "-c",
             (ROOT / "core/adapters/python_ast.py").read_text()],
            input=json.dumps(items), capture_output=True, text=True,
            check=True, timeout=30,
        )
        result = json.loads(proc.stdout)
        found = {
            (item["name"], imp["target"], imp["line"], imp["syntax"])
            for item in result for imp in item["imports"]
        }
        self.assertEqual(found, self.expected)


if __name__ == "__main__":
    unittest.main()
