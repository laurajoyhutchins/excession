"""Actual CLI join over pinned syntax, optional native declarations and runtime witnesses."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from validate_evidence_contract import validate_evidence_query  # noqa: E402

BIN = ROOT / "core/target/debug/excession"
OBSERVER = ROOT / "tools/python_import_observer.py"
HAVE_GRAMMARS = importlib.util.find_spec("tree_sitter_language_pack") is not None


def call(*args, check=True, env=None):
    result = subprocess.run([str(a) for a in args], capture_output=True, check=False,
                            env=env, timeout=60)
    if check and result.returncode != 0:
        raise AssertionError(f"{args}\n{result.stderr.decode(errors='replace')}")
    return result


@unittest.skipUnless(BIN.is_file() and HAVE_GRAMMARS,
                     "requires native CLI and installed Tree-sitter grammars")
class UnifiedEvidenceQuery(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="excession-evidence-query-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        call("git", "init", "-q", "-b", "main", self.repo)
        call("git", "-C", self.repo, "config", "user.name", "Evidence tests")
        call("git", "-C", self.repo, "config", "user.email", "evidence@example.invalid")
        files = {
            "src/app.py": "import importlib\nimport pkg.plugin\n"
                          "assert importlib.import_module('pkg.plugin').VALUE == 41\n",
            "src/pkg/__init__.py": "",
            "src/pkg/plugin.py": "VALUE = 41\n",
            "src/main.rs": "mod helper;\nuse crate::helper::value;\nfn main() { let _ = value(); }\n",
            "src/helper.rs": "pub fn value() -> u32 { 41 }\n",
        }
        for relative, content in files.items():
            target = self.repo / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content)
        call("git", "-C", self.repo, "add", ".")
        call("git", "-C", self.repo, "commit", "-qm", "Pinned evidence fixture")
        self.commit = call("git", "-C", self.repo, "rev-parse", "HEAD").stdout.decode().strip()
        self.tree = call("git", "-C", self.repo, "rev-parse", "HEAD^{tree}").stdout.decode().strip()
        self.syntax = self.root / "syntax.json"
        self.resolution = self.root / "resolution.json"
        self.runtime = self.root / "runtime.json"
        self.native = self.root / "native.json"
        self.syntax.write_bytes(call(BIN, "syntax", "scan", "--repo", self.repo,
                                     "--rev", self.commit, "--roots", "src").stdout)
        self.resolution.write_bytes(call(BIN, "syntax", "resolve", "--repo", self.repo,
                                         "--report", self.syntax).stdout)
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
        observed = call(sys.executable, OBSERVER, "--repo", self.repo, "--rev", self.commit,
                        "--script", "src/app.py", "--out", self.runtime, "--trusted-local",
                        env=env)
        self.assertEqual(observed.returncode, 0, observed.stderr)
        self.native.write_text(json.dumps({
            "schema": "excession.input-declaration.v1",
            "coverage": "DECLARED_INPUTS_ONLY",
            "language": "rust", "subject": "src/main.rs",
            "inputs": [
                {"path": "src/main.rs", "role": "source"},
                {"path": "src/helper.rs", "role": "source"},
            ],
            "assumptions": {
                "dependency-discovery": "excession-toolchain-native-v1",
                "toolchain-version": "test-only-declaration",
                "toolchain-sha256": "f" * 64, "platform": "test",
                "source-commit": self.commit, "source-tree": self.tree,
            },
        }))

    def query(self, path, native=True, runtime=True, check=True):
        cmd = [BIN, "inspect", "evidence", "--repo", self.repo, "--rev", self.commit,
               "--focus", path, "--syntax-report", self.syntax,
               "--resolution-report", self.resolution]
        if native:
            cmd.extend(["--declaration", self.native])
        if runtime:
            cmd.extend(["--runtime-report", self.runtime])
        result = call(*cmd, check=check)
        return json.loads(result.stdout) if check else result

    def test_one_query_preserves_separate_claims(self):
        app = self.query("src/app.py")
        validate_evidence_query(app)
        self.assertEqual(app["schema"], "excession.evidence-query.v1")
        self.assertEqual(app["source_commit"], self.commit)
        self.assertEqual(app["source_tree"], self.tree)
        self.assertEqual(app["static_replay"], "REGENERATED_FROM_PINNED_GIT")
        self.assertEqual(app["compiler_inputs"], [])
        self.assertEqual(app["compiler_receipt"]["provenance"],
                         "UNATTESTED_NATIVE_DECLARATION")
        self.assertTrue(any(x["source_path"] == "src/app.py"
                            for x in app["static_candidates"]))
        self.assertTrue(any(x["witness"]["module"] == "pkg.plugin"
                            for x in app["runtime_witnesses"]))
        self.assertTrue(all(x["selection"] == "EXECUTION_CONTEXT_ONLY"
                            for x in app["runtime_witnesses"]))
        self.assertEqual(app["runtime_receipt"]["provenance"],
                         "SELF_HASHED_UNATTESTED_EXECUTION")
        self.assertIn("ABSENCE_OF_OBSERVATION_IS_NEVER_ABSENCE_OF_DEPENDENCY",
                      app["limits"])

        plugin = self.query("src/pkg/plugin.py")
        validate_evidence_query(plugin)
        self.assertTrue(any(x["target_path"] == "src/pkg/plugin.py"
                            for x in plugin["static_candidates"]))
        self.assertTrue(any(x["selection"] == "PINNED_TARGET_PATH"
                            for x in plugin["runtime_witnesses"]))

        rust = self.query("src/main.rs")
        validate_evidence_query(rust)
        self.assertEqual(rust["compiler_receipt"]["subject"], "src/main.rs")
        self.assertEqual({x["path"] for x in rust["compiler_inputs"]},
                         {"src/main.rs", "src/helper.rs"})
        self.assertEqual(rust["runtime_witnesses"], [])
        # Rust compiler inputs remain separate from syntax candidates.
        # Resolved Rust relationships require rust-analyzer SCIP evidence.
        self.assertFalse(any(x["target_path"] == "src/helper.rs"
                             for x in rust["static_candidates"]))

    def test_omitted_evidence_is_null_not_absence_assertion(self):
        result = self.query("src/main.rs", native=False, runtime=False)
        self.assertIsNone(result["compiler_receipt"])
        self.assertIsNone(result["runtime_receipt"])
        self.assertEqual(result["counts"]["runtime_witness_records"], 0)

    def test_runtime_digest_and_git_identity_are_rejected_if_tampered(self):
        initial = self.query("src/app.py")
        self.assertIn("query_id", initial)
        self.runtime.write_text(self.runtime.read_text().replace(
            '"OBSERVED_RUNTIME_ONLY"', '"COMPLETE_RUNTIME"', 1))
        result = self.query("src/app.py", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"REFUSED", result.stderr)

    def test_native_revision_mismatch_is_rejected(self):
        data = json.loads(self.native.read_text())
        data["assumptions"]["source-tree"] = "0" * 40
        self.native.write_text(json.dumps(data))
        result = self.query("src/main.rs", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"source commit/tree", result.stderr)

    def test_static_tamper_is_refused_despite_other_receipts(self):
        data = json.loads(self.resolution.read_text())
        self.assertTrue(data["candidate_edges"])
        data["candidate_edges"][0]["target_blob"] = "0" * 40
        self.resolution.write_text(json.dumps(data))
        result = self.query("src/app.py", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"REFUSED", result.stderr)


if __name__ == "__main__":
    unittest.main()
