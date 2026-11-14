"""Prevent broad CI fan-out and protect release verification (stdlib only)."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / ".github" / "workflows"
FOCUSED = (
    "clang-compile-db.yml",
    "compiler-cross-repo.yml",
    "configured-graph.yml",
    "kbuild-native-graph.yml",
    "kernel-whole-inventory.yml",
    "linux-native-parsers.yml",
    "native-test-selectors.yml",
    "safe-uncommitted-tests.yml",
    "refactor-codegen.yml",
    "syntax-parsers.yml",
    "scip-fixtures.yml",
)
CATCHALL = {"core/src/lib.rs", "core/src/bin/excession.rs", "core/**", "core/src/**"}


def read(name):
    return (ROOT / name).read_text(encoding="utf-8")


def owned_paths(source):
    trigger = re.search(
        r"(?ms)^  pull_request:\n(.*?)(?=^  (?:push|schedule|workflow_dispatch|workflow_call):|^concurrency:|^permissions:|^jobs:)",
        source,
    )
    if trigger is None or "    paths:\n" not in trigger.group(1):
        raise AssertionError("focused workflow is missing owned PR path filter")
    return set(
        re.findall(r"(?m)^\s{6}-\s+['\"]?([^'\"\n]+)['\"]?\s*$", trigger.group(1))
    )


class CiTopology(unittest.TestCase):
    def test_required_admission_commands(self):
        source = read("verify.yml")
        for required in (
            "name: Verify", "  pull_request:", "    branches: [main]",
            "cargo fmt --manifest-path core/Cargo.toml -- --check",
            "cargo clippy --offline --locked", "cargo nextest run --offline --locked",
            "history audit --repo .", "agent verify --repo .",
            "npm run check && npm test", "python3 -m pytest -q tests",
        ):
            self.assertIn(required, source, required)

    def test_owned_scopes_do_not_use_shared_cli_as_catchall(self):
        for name in FOCUSED:
            with self.subTest(name=name):
                source = read(name)
                paths = owned_paths(source)
                self.assertTrue(paths)
                self.assertFalse(paths & CATCHALL, paths & CATCHALL)
                self.assertIn("concurrency:", source)
                self.assertIn("github.event.pull_request.number || github.ref", source)
                self.assertIn("  workflow_dispatch:", source)
                self.assertIn("cancel-in-progress:", source)

    def test_immutable_release_requirements(self):
        source = read("portable-cli.yml")
        for required in (
            "types: [opened, reopened, synchronize, labeled]",
            "startsWith(github.head_ref, 'release/')",
            "'ci:full-portability'",
            '      - "v*"',
            "needs: [portability, musl, acceptance]",
            "test_release_candidate.py",
            'gh release view "$GITHUB_REF_NAME"',
            "Verify source tag and all downloaded archive digests",
        ):
            self.assertIn(required, source, required)

    def test_manual_reproducibility_remains(self):
        for name in ("linux-full-sweep.yml", "linux-kernel-probe.yml", "benchmark-relations.yml"):
            with self.subTest(name=name):
                self.assertIn("  workflow_dispatch:", read(name))


if __name__ == "__main__":
    unittest.main()
