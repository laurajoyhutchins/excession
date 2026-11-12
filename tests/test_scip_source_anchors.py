"""Independent pinned-source syntax anchors for actual native SCIP observations."""
from __future__ import annotations

import copy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
from scip_source_anchors import (  # noqa: E402
    source_declaration, verify_anchors, load_manifest,
)


class SourceAnchorTests(unittest.TestCase):
    def test_python_relative_import_ast_and_target(self):
        case = {
            "source": "src/pkg/a.py", "target": "src/pkg/b.py",
            "kind": "RELATIVE_PYTHON_IMPORT", "module": "b",
        }
        self.assertTrue(source_declaration(case, "from .b import Item\n"))
        self.assertFalse(source_declaration(case, "from .c import Item\n"))
        bad = dict(case, target="src/pkg/unrelated.py")
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            source_declaration(bad, "from .b import Item\n")

    def test_js_esm_import_uses_actual_relative_specifier(self):
        case = {
            "source": "lib/adapters/http.js", "target": "lib/utils.js",
            "kind": "ESM_IMPORT", "specifier": "../utils.js",
        }
        self.assertTrue(source_declaration(case, "import utils from '../utils.js';\n"))
        self.assertFalse(source_declaration(case, "import utils from '../unknown.js';\n"))

    def test_rust_crate_use_matches_exact_module(self):
        case = {
            "source": "core/src/concepts_scip.rs", "target": "core/src/scip_refactor.rs",
            "kind": "RUST_CRATE_USE", "module": "scip_refactor",
        }
        self.assertTrue(source_declaration(case, "use crate::scip_refactor;\n"))
        self.assertFalse(source_declaration(case, "use crate::other_module;\n"))

    def test_real_manifest_is_frozen_to_source_driven_edges(self):
        declaration = load_manifest(ROOT / "benchmarks/fixtures/concepts-scip-source-anchors.json")
        self.assertEqual(declaration["schema"], "excession.concepts-source-anchors.v1")
        self.assertGreaterEqual(sum(x["fixture"] == "excession" for x in declaration["cases"]), 3)
        self.assertTrue(all(x["source"] != x["target"] for x in declaration["cases"]))

    def test_positive_git_source_anchor_rejects_wrong_native_destination(self):
        with tempfile.TemporaryDirectory(prefix="scip-source-anchor-") as tmp:
            repo = Path(tmp)
            def git(*args):
                return subprocess.check_output(
                    ["git", "-C", str(repo), *args],
                    stderr=subprocess.DEVNULL,
                ).decode().strip()
            git("init", "-q")
            git("config", "user.email", "oracle@example.invalid")
            git("config", "user.name", "Independent source oracle")
            (repo / "a.py").write_text("from .b import B\n")
            (repo / "b.py").write_text("class B:\n    pass\n")
            (repo / "c.py").write_text("class C:\n    pass\n")
            git("add", ".")
            git("commit", "-qm", "Pinned source imports")
            commit = git("rev-parse", "HEAD")
            manifest = {
                "schema": "excession.concepts-source-anchors.v1",
                "cases": [{
                    "fixture": "fixture", "language": "python",
                    "source": "a.py", "target": "b.py",
                    "kind": "RELATIVE_PYTHON_IMPORT", "module": "b",
                }],
            }
            index = {
                "documents": [
                    {"relative_path": "a.py", "occurrences": [{
                        "symbol": "scip python fixture B",
                        "symbol_roles": 0, "range": [0, 0, 4],
                    }]},
                    {"relative_path": "b.py", "occurrences": [{
                        "symbol": "scip python fixture B",
                        "symbol_roles": 1, "range": [0, 0, 4],
                    }]},
                ],
            }
            result = verify_anchors(repo, commit, index, manifest, "fixture", "python")
            self.assertEqual(result["verified_source_imports"], 1)
            self.assertEqual(result["status"], "FROZEN_POSITIVE_SOURCE_ANCHORS_MATCH")
            forged = copy.deepcopy(index)
            forged["documents"][1]["relative_path"] = "c.py"
            with self.assertRaisesRegex(ValueError, "missed independently source-verified"):
                verify_anchors(repo, commit, forged, manifest, "fixture", "python")
            (repo / "a.py").write_text("from .c import C\n")
            # The working tree changed but our pinned Git source did not.
            result = verify_anchors(repo, commit, index, manifest, "fixture", "python")
            self.assertEqual(result["verified_source_imports"], 1)


if __name__ == "__main__":
    unittest.main()
