"""Independent tests for interpreting official decoded SCIP occurrences."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import tempfile
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
from concepts_scip_oracle import (  # noqa: E402
    native_references, reconcile, verified_content, sha256,
)


def occ(symbol, role=0, line=0):
    return {"symbol": symbol, "symbol_roles": role, "range": [line, 0, 1]}


def doc(path, occurrences):
    return {"relative_path": path, "language": "python",
            "occurrences": occurrences}


def index(reciprocal=True, duplicates=False, unresolved=True):
    alpha = "scip python pypi fixture 1.0 a.A."
    beta = "scip python pypi fixture 1.0 b.B."
    unknown = "scip python pypi other 0 missing."
    a = [occ(alpha, 1), occ(beta, 0)]
    b = [occ(beta, 1)]
    if reciprocal:
        b.append(occ(alpha))
    if duplicates:
        b.append(occ(beta, 1))
    if unresolved:
        a.append(occ(unknown))
    return {"documents": [doc("a.py", a), doc("b.py", b)]}


class IndependentScipOracleTests(unittest.TestCase):
    def test_reciprocal_is_two_exact_symbol_references(self):
        found, docs, counts = native_references(index())
        self.assertEqual({(a, b) for a, b, _, _ in found},
                         {("a.py", "b.py"), ("b.py", "a.py")})
        self.assertEqual(len(found), 2)
        self.assertEqual(counts["unresolved_references"], 1)
        self.assertEqual(set(docs), {"a.py", "b.py"})

    def test_one_way_does_not_become_a_reciprocal_reference(self):
        found, _, _ = native_references(index(reciprocal=False))
        self.assertEqual({(a, b) for a, b, _, _ in found},
                         {("a.py", "b.py")})

    def test_two_definitions_in_one_file_are_ambiguous(self):
        found, _, counts = native_references(index(duplicates=True))
        self.assertEqual({(a, b) for a, b, _, _ in found},
                         {("b.py", "a.py")})
        self.assertEqual(counts["unresolved_references"], 2)

    def test_local_and_forward_declarations_are_not_connected(self):
        local = "local 0"
        native = {"documents": [
            doc("a.py", [occ(local, 1), occ("external.s", 64)]),
            doc("b.py", [occ(local, 0), occ("external.s", 0)]),
        ]}
        found, _, _ = native_references(native)
        self.assertEqual(found, set())

    def test_range_format_validation(self):
        broken = index()
        broken["documents"][0]["occurrences"][0]["range"] = [-1, 1, 5]
        with self.assertRaisesRegex(ValueError, "invalid SCIP occurrence range"):
            native_references(broken)

    def test_canonical_semantic_digest_catches_tampered_content(self):
        doc = {"schema": "test", "status": "PARTIAL"}
        doc["model_id"] = sha256(doc)
        self.assertTrue(verified_content(doc, "model_id"))
        doc["status"] = "COMPLETE"
        self.assertFalse(verified_content(doc, "model_id"))

    def test_forged_native_projection_does_not_pass_independent_gate(self):
        with tempfile.TemporaryDirectory() as folder:
            repo = Path(folder)
            def git(*args):
                return subprocess.check_output(
                    ["git", "-C", str(repo), *args], stderr=subprocess.DEVNULL,
                ).decode().strip()
            git("init", "-q")
            git("config", "user.name", "SCIP oracle")
            git("config", "user.email", "oracle@example.invalid")
            (repo / "a.py").write_text("x = 1\n")
            (repo / "b.py").write_text("y = 2\n")
            git("add", ".")
            git("commit", "-qm", "source")
            commit = git("rev-parse", "HEAD")
            tree = git("rev-parse", "HEAD^{tree}")
            # The empty baseline is well formed, but the index has real
            # references, so its empty normalized projection must be refused.
            src = index(unresolved=False)
            batch = {
                "source_commit": commit, "source_tree": tree,
                "schema": "excession.concepts-observation-batch.v1",
                "completeness": "PARTIAL_CANDIDATES_ONLY",
                "subjects": [], "observations": [],
                "inputs": [{"kind": "SCIP", "receipt_id": "a" * 64}],
                "gaps": [{"reason": "PARTIAL_CONFIGURATION"}],
            }
            batch["batch_id"] = sha256(batch)
            model = {
                "schema": "excession.concepts-model.v1",
                "source_commit": commit, "source_tree": tree,
                "coverage": {"status": "PARTIAL", "producer_receipts": ["a" * 64]},
                "subjects": [], "claims": [], "concepts": [],
            }
            model["model_id"] = sha256(model)
            manifest = {
                "source_commit": commit, "source_tree": tree,
                "model_id": model["model_id"], "input_batch_ids": [batch["batch_id"]],
            }
            manifest["snapshot_id"] = sha256(manifest)
            audit = {
                "snapshot_id": manifest["snapshot_id"],
                "status": "SOURCE_BOUND_SCIP_INDEX_REPLAYED",
                "indexer_independently_executed": False,
            }
            with self.assertRaisesRegex(ValueError, "disagrees with native occurrences"):
                reconcile(src, batch, model, manifest, audit, repo, commit, tree)


if __name__ == "__main__":
    unittest.main()
