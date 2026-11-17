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
        # Explicit full qualification must retain its archive receipts and
        # independent upstream acceptance, not only launch platform builds.
        self.assertGreaterEqual(
            source.count("contains(github.event.pull_request.labels.*.name, 'ci:full-portability')"),
            4,
        )
        self.assertIn("name: Installed Linux archive vs independent pinned upstream source", source)
        # On Windows PowerShell, "$EXCESSION_BIN" is not an environment variable.
        # Package with an explicit Bash shell on every matrix platform.
        self.assertIn(
            'name: Construct portable archive and SHA-256 manifest\n'
            '        shell: bash\n'
            '        run: python tools/package_release.py --binary "$EXCESSION_BIN"',
            source,
        )

    def test_github_evidence_read_only_and_pinned(self):
        source = read("github-evidence.yml")
        for required in (
            "  workflow_call:",
            "  workflow_dispatch:",
            "  pull_request:",
            "permissions:\n  contents: read",
            "ref: ${{ github.event.pull_request.base.sha || job.workflow_sha }}",
            "PRODUCER_SHA: ${{ github.event.pull_request.base.sha || job.workflow_sha }}",
            "test \"$TOOL_SHA\" = \"$PRODUCER_SHA\"",
            "github_evidence.py",
            "if: always()",
            "actions/upload-artifact@v4",
            "excession-evidence-v1-",
        ):
            self.assertIn(required, source, required)
        self.assertNotIn("contents: write", source)
        self.assertNotIn("pull_request_target", source)
        self.assertNotIn("secrets:", source)

    def test_github_native_transport_is_only_hosted_agent_path(self):
        request = read("github-native-analysis.yml")
        evidence = read("github-evidence.yml")
        verify = read("verify.yml")
        for expected in (
            "  issues:",
            "github.event.issue.user.id == 219002713",
            "github.actor_id == '219002713'",
            "permissions:\n  contents: read",
            "tools/github_analysis.py validate",
            "tools/github_analysis.py execute",
            "retention-days: 30",
            "bash tools/ci/prepare-excession.sh . release",
        ):
            self.assertIn(expected, request)
        self.assertIn("bash tool/tools/ci/prepare-excession.sh tool debug", evidence)
        self.assertIn("Swatinem/rust-cache@v2", evidence)
        self.assertNotIn("deploy/remote-excession", verify)
        for workflow in (request, evidence, verify):
            self.assertNotIn("pull_request_target:", workflow)
            self.assertNotIn("contents: write", workflow)

    def test_quality_ratchet_retains_baseline_and_review_boundary(self):
        verify = read("verify.yml")
        for required in (
            "test_quality_ratchet.py",
            'git show "$BASE:tools/ci/quality_ratchet.py"',
            'quality_ratchet.py check --base "$BASE" --candidate HEAD',
        ):
            self.assertIn(required, verify)
        fixes = read("quality-improvements.yml")
        for required in (
            "  schedule:", "  workflow_dispatch:",
            "github.ref == 'refs/heads/main'",
            "github.repository == 'laurajoyhutchins/excession'",
            "cargo clippy --fix", "cargo fmt --manifest-path",
            'git diff --check', 'cargo test --offline --locked',
            'gh pr create --draft', "pull-requests: write",
            "actions: write", 'gh workflow run verify.yml --ref "$BRANCH"',
        ):
            self.assertIn(required, fixes)
        self.assertNotIn("pull_request_target:", fixes)
        self.assertNotIn("gh pr merge", fixes)
        self.assertNotIn("  pull_request:", fixes)

    def test_structural_receipts_are_explicitly_trusted_and_not_a_pr_gate(self):
        verify = read("verify.yml")
        manual = read("structural-preservation.yml")
        self.assertIn("test_structural_preservation.py", verify)
        for required in (
            "  workflow_dispatch:", "permissions:\n  contents: read",
            "github.actor_id == '219002713'",
            "github.ref == 'refs/heads/main'",
            "persist-credentials: false", "--trusted-local true",
            "structural_preservation.py",
            "core/target/debug/excession agent verify",
            "actions/upload-artifact@v4",
        ):
            self.assertIn(required, manual)
        self.assertNotIn("  pull_request:", manual)
        self.assertNotIn("pull_request_target:", manual)
        self.assertNotIn("contents: write", manual)
        self.assertNotIn("gh pr merge", manual)

    def test_auto_integration_requires_replay_and_two_exact_head_gates(self):
        verify = read("verify.yml")
        self.assertIn("test_improvement_cycle.py", verify)
        auto = read("quality-auto-integrate.yml")
        for required in (
            "  schedule:", "  workflow_dispatch:",
            "github.repository == 'laurajoyhutchins/excession'",
            "github.ref == 'refs/heads/main'",
            "permissions:\n  contents: write\n  pull-requests: write\n  actions: write",
            "improvement_cycle.py verify", "quality_ratchet.py check",
            "cargo fmt --manifest-path", "cargo test --offline --locked",
            "gh workflow run verify.yml --ref", "gh workflow run github-evidence.yml --ref",
            "bash tools/ci/await_verified_merge.sh", "native-improvement.json",
        ):
            self.assertIn(required, auto, required)
        self.assertNotIn("  pull_request:", auto)
        self.assertNotIn("pull_request_target:", auto)
        self.assertNotIn("  workflow_run:", auto)
        self.assertNotIn(" --admin", auto)
        self.assertNotIn(" --auto", auto)
        cargo = read("quality-cargo-update.yml")
        for required in (
            "  schedule:", "  workflow_dispatch:",
            "cargo update --manifest-path core/Cargo.toml",
            "cargo_patch_audit.py", "quality_ratchet.py check",
            "core/Cargo.lock", 'DRAFT=(--draft)',
            "gh workflow run verify.yml --ref",
            "gh workflow run github-evidence.yml --ref",
            "bash tools/ci/await_verified_merge.sh",
            "persist-credentials: false",
        ):
            self.assertIn(required, cargo)
        self.assertNotIn("  pull_request:", cargo)
        self.assertNotIn("pull_request_target:", cargo)
        self.assertNotIn(" --admin", cargo)
        shared = (Path(__file__).resolve().parents[1] / "tools/ci/await_verified_merge.sh").read_text()
        for required in (
            "--event workflow_dispatch", "select(.headSha == $sha)",
            'test "$REMOTE_BASE" = "$BASE"', 'gh pr merge "$PR" --squash',
            "if [ \"$VERIFY\" = success ] && [ \"$EVIDENCE\" = success ]",
        ):
            self.assertIn(required, shared)
        self.assertNotIn(" --admin", shared)
        review = read("quality-improvements.yml")
        self.assertIn("cargo clippy --fix", review)
        self.assertIn("gh pr create --draft", review)
        self.assertNotIn("gh pr merge", review)

    def test_manual_reproducibility_remains(self):
        for name in ("linux-full-sweep.yml", "linux-kernel-probe.yml", "benchmark-relations.yml"):
            with self.subTest(name=name):
                self.assertIn("  workflow_dispatch:", read(name))


if __name__ == "__main__":
    unittest.main()
