"""Counterexamples for conservative native Cargo patch-update integration."""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

DIRECTORY = Path(__file__).resolve().parents[1] / "tools" / "ci"
sys.path.insert(0, str(DIRECTORY))
import cargo_patch_audit as audit
import improvement_cycle as common


def lock(version="1.2.3", checksum="a" * 64, dep=None, source=audit.REGISTRY):
    if dep is None:
        dep = ["bar"]
    return {
        "version": 4,
        "package": [
            {"name": "bar", "version": "0.3.0", "source": audit.REGISTRY,
             "checksum": "c" * 64},
            {"name": "foo", "version": version, "source": source,
             "checksum": checksum, "dependencies": dep},
            {"name": "workspace", "version": "0.1.0",
             "dependencies": ["foo"]},
        ],
    }


def encoded(data):
    """Only the TOML fields exercised in this source-identity fixture."""
    lines = ["version = 4\n"]
    for p in data["package"]:
        lines.extend(["\n[[package]]", f"name = {json.dumps(p['name'])}",
                      f"version = {json.dumps(p['version'])}"])
        if "source" in p:
            lines.append(f"source = {json.dumps(p['source'])}")
        if "checksum" in p:
            lines.append(f"checksum = {json.dumps(p['checksum'])}")
        if "dependencies" in p:
            lines.append(f"dependencies = {json.dumps(p['dependencies'])}")
    return ("\n".join(lines) + "\n").encode()


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def catalog():
    p = [
        {"id": "native-rustfmt", "producer": "cargo-fmt-pinned",
         "integration": "automatic-after-exact-head-verification",
         "paths": ["core/**/*.rs"], "rationale": "native formatting",
         "evidence": "wiki", "independent_checks": ["formatter-replay", "quality-ratchet",
         "cargo-tests", "verify-workflow", "github-evidence-workflow"]},
        {"id": "native-clippy", "producer": "cargo-clippy-fix-pinned",
         "integration": "review-required", "paths": ["core/**/*.rs"],
         "rationale": "review native changes", "evidence": "wiki",
         "independent_checks": ["human-review"]},
        {"id": "native-cargo-lock-refresh", "producer": "cargo-update-locked",
         "integration": "automatic-after-exact-head-verification",
         "paths": ["core/Cargo.lock"], "rationale": "patch-only dependency updates",
         "evidence": "wiki", "independent_checks": ["cargo-lock-graph-audit",
         "cargo-tests", "quality-ratchet", "verify-workflow", "github-evidence-workflow"]},
    ]
    return {"schema": common.SCHEMA, "recipes": p}


class CargoPatchAcceptance(unittest.TestCase):
    def test_unchanged_graph_same_minor_patch_is_candidate(self):
        changes = audit.patch_changes(encoded(lock()), encoded(lock("1.2.4", "b" * 64)))
        self.assertEqual(changes, [{"name": "foo", "source": audit.REGISTRY,
                                    "from": "1.2.3", "to": "1.2.4"}])

    def test_minor_update_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.3.0", "b" * 64)))

    def test_version_downgrade_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.2.2", "b" * 64)))

    def test_changed_dependencies_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.2.4", "b" * 64, ["new"])))

    def test_checksum_substitution_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.2.3", "forgery")))

    def test_missing_checksum_change_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.2.4", "a" * 64)))

    def test_source_switch_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.2.4", "def", source="git+https://example.invalid/x")))

    def test_prerelease_update_refused(self):
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(lock("1.2.4-alpha.1", "b" * 64)))

    def test_new_package_refused(self):
        candidate = lock("1.2.4", "def")
        candidate["package"].append({"name": "extra", "version": "1.0.0",
                                     "source": audit.REGISTRY, "checksum": "new"})
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(encoded(lock()), encoded(candidate))

    def test_global_metadata_change_refused(self):
        original = encoded(lock())
        modified = encoded(lock("1.2.4", "def")).replace(b"version = 4", b"version = 3", 1)
        with self.assertRaises(audit.common.Refused):
            audit.patch_changes(original, modified)


class CargoGitCandidates(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Fixture")
        git(self.repo, "config", "user.email", "fixture@example.invalid")
        self.write("quality/improvement-recipes.json", json.dumps(catalog()).encode())
        self.write("core/Cargo.lock", encoded(lock()))
        self.base = self.commit()
        self.write("core/Cargo.lock", encoded(lock("1.2.4", "def")))
        self.head = self.commit()

    def write(self, filename, data):
        p = self.repo / filename
        p.parent.mkdir(exist_ok=True, parents=True)
        p.write_bytes(data)

    def commit(self):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "fixture")
        return git(self.repo, "rev-parse", "HEAD")

    def test_exact_source_patch_can_progress_to_independent_ci(self):
        receipt = audit.verify(self.repo, self.base, self.head)
        self.assertEqual(receipt["status"], "PATCH_ONLY_NO_OBSERVED_LOCK_GRAPH_CHANGE_PENDING_CI")
        self.assertEqual(len(receipt["patch_updates"]), 1)
        self.assertFalse(receipt["ready_to_merge"])
        self.assertFalse(receipt["dependency_safety_proved"])

    def test_candidate_with_extra_source_change_is_refused(self):
        git(self.repo, "reset", "--hard", self.base)
        self.write("core/src/lib.rs", b"fn extra() {}\n")
        extra = self.commit()
        with self.assertRaisesRegex(audit.common.Refused, "only the existing core/Cargo.lock"):
            audit.verify(self.repo, self.base, extra)

    def test_rule_edit_in_candidate_is_refused(self):
        git(self.repo, "reset", "--hard", self.base)
        p = self.repo / "quality/improvement-recipes.json"
        p.write_bytes(p.read_bytes() + b"\n")
        extra = self.commit()
        with self.assertRaisesRegex(audit.common.Refused, "own improvement authority"):
            audit.verify(self.repo, self.base, extra)


if __name__ == "__main__":
    unittest.main()
