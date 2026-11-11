"""Dedicated Excession CLI integration against immutable, real Git source fixtures."""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"

BUNDLE = ROOT / "core/fixtures/h11-crossrepo.bundle"


def cmd(*argv: object, input: str | None = None, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run([str(a) for a in argv], input=None if input is None else input.encode(), capture_output=True, check=False)
    if check and result.returncode:
        raise AssertionError(f"exit {result.returncode}: {argv!r}\n{result.stderr.decode(errors='replace')}")
    return result


def data(*argv: object) -> dict:
    return json.loads(cmd(*argv).stdout)


class DedicatedCLI(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not BIN.is_file():
            raise unittest.SkipTest("build Excession CLI first")
        cls.temp = tempfile.TemporaryDirectory(prefix="excession-cli-")
        cls.dir = Path(cls.temp.name)
        cls.repo = cls.dir / "source"
        cmd("git", "clone", "-q", BUNDLE, cls.repo)
        cls.commit = cmd("git", "-C", cls.repo, "rev-parse", "HEAD").stdout.decode().strip()
        cls.db = cls.dir / "index.sqlite"
        indexed = data(BIN, "index", "--repo", cls.repo, "--db", cls.db, "--name", "h11", "--rev", "HEAD", "--roots", "h11,tests")
        assert indexed["source_commit"] == cls.commit

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temp.cleanup()

    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory(prefix="cli-test-", dir=self.dir)
        self.addCleanup(self.scratch.cleanup)
        self.work = Path(self.scratch.name)

    def plan(self, seed: str = "h11._connection") -> tuple[dict, Path]:
        output = cmd(BIN, "view", "deps", "--db", self.db, "--name", "h11", "--rev", "HEAD", "--seed", seed).stdout
        manifest = self.work / f"{seed}.json"
        manifest.write_bytes(output)
        return json.loads(output), manifest

    def report(self) -> tuple[Path, Path, str]:
        source = "h11/_connection.py"
        report = self.work / "report.sarif"
        target = self.work / "security.sqlite"
        # These are intentionally controlled path-claim fixtures, not vulnerabilities.
        loc = lambda p, n: {"physicalLocation": {"artifactLocation": {"uri": p}, "region": {"startLine": n}}}
        content = {"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "CodeQL"}}, "results": [{
            "ruleId": "controlled/test-flow", "locations": [loc(source, 20)],
            "codeFlows": [{"threadFlows": [{"locations": [
                {"location": loc(source, 20)},
                {"location": loc("h11/_events.py", 25)},
                {"location": loc("h11/_util.py", 20)},
            ]}]}],
        }]}]}
        report.write_text(json.dumps(content))
        initial = data(BIN, "security", "ingest", "--repo", self.repo, "--report", report, "--db", target, "--engine", "codeql")
        audit = data(BIN, "security", "audit", "--repo", self.repo, "--report", report, "--db", target, "--engine", "codeql")
        self.assertEqual(initial["run_id"], audit["run_id"])
        self.assertTrue(audit["independently_replayed"])
        return report, target, initial["run_id"]

    def test_help_version_and_invalid_flags(self):
        self.assertIn("Excession", cmd(BIN, "--help").stdout.decode())
        self.assertIn("Excession", cmd(BIN, "view", "--help").stdout.decode())
        self.assertTrue(cmd(BIN, "--version").stdout.decode().startswith("excession "))
        root_help = cmd(BIN, "--help").stdout
        self.assertNotIn(b"research", root_help)
        self.assertIn(b"benchmark", root_help)
        self.assertIn(b"inspect", root_help)
        for argv in [
            ["view", "deps", "--db", str(self.db)],
            ["view", "deps", "--db", str(self.db), "--db", "duplicate"],
            ["view", "replay", "deps", "--db", str(self.db), "--name", "h11", "--rev", "HEAD", "--seed", "h11._connection"],
            ["view", "bad-command"],
            ["browse", "--repo", str(self.repo), "--manifest", "missing", "--max-bytes", "0"],
        ]:
            bad = cmd(BIN, *argv, check=False)
            self.assertNotEqual(bad.returncode, 0, argv)
            self.assertEqual(bad.stdout, b"", argv)
            self.assertIn(b"REFUSED:", bad.stderr)

    def test_dependency_plan_replays_without_legacy_commands(self):
        result, file = self.plan()
        self.assertEqual(result["source_commit"], self.commit)
        replay = data(BIN, "view", "replay", "deps", "--db", self.db, "--name", "h11", "--rev", "HEAD", "--seed", "h11._connection", "--manifest", file)
        self.assertEqual(replay["view_id"], result["view_id"])
        file.write_text("{}")
        bad = cmd(BIN, "view", "replay", "deps", "--db", self.db, "--name", "h11", "--rev", "HEAD", "--seed", "h11._connection", "--manifest", file, check=False)
        self.assertNotEqual(bad.returncode, 0)

    def test_one_crate_one_binary_and_removed_legacy_dispatch(self):
        import tomllib
        cargo = tomllib.loads((ROOT / "core/Cargo.toml").read_text())
        self.assertEqual(cargo["package"]["name"], "excession")
        self.assertEqual(cargo["lib"]["name"], "excession")
        self.assertEqual([target["name"] for target in cargo["bin"]], ["excession"])
        retired = cmd(BIN, "research", check=False)
        self.assertNotEqual(retired.returncode, 0)
        self.assertEqual(retired.stdout, b"")
        self.assertIn(b"REFUSED:", retired.stderr)

    def test_virtual_navigation_and_exact_bytes(self):
        plan, manifest = self.plan()
        vpath = next(entry["virtual_path"] for entry in plan["entries"] if entry["role"] == "focus")
        entry = next(e for e in plan["entries"] if e["virtual_path"] == vpath)
        self.assertTrue(data(BIN, "view", "ls", "--repo", self.repo, "--manifest", manifest)["children"])
        description = data(BIN, "view", "stat", "--repo", self.repo, "--manifest", manifest, "--path", vpath)
        self.assertEqual(description["entry"]["blob"], entry["blob"])
        exact = cmd(BIN, "view", "cat", "--repo", self.repo, "--manifest", manifest, "--path", vpath).stdout
        direct = cmd("git", "-C", self.repo, "cat-file", "blob", entry["blob"]).stdout
        self.assertEqual(exact, direct)
        bad = cmd(BIN, "view", "cat", "--repo", self.repo, "--manifest", manifest, "--path", "../invalid", check=False)
        self.assertNotEqual(bad.returncode, 0)
        self.assertEqual(bad.stdout, b"")

    def test_composition_selection_mount_and_materialize(self):
        first, a = self.plan()
        second, b = self.plan("h11._headers")
        self.assertEqual(first["source_commit"], second["source_commit"])
        mount_a = self.work / "mount-a.json"
        mount_a.write_bytes(cmd(BIN, "view", "mount", "--repo", self.repo, "--manifest", a, "--namespace", "connection").stdout)
        mount_b = self.work / "mount-b.json"
        mount_b.write_bytes(cmd(BIN, "view", "mount", "--repo", self.repo, "--manifest", b, "--namespace", "headers").stdout)
        combined = data(BIN, "view", "compose", "--repo", self.repo, "--op", "union", "--left", mount_a, "--right", mount_b)
        self.assertGreater(len(combined["entries"]), len(first["entries"]))
        joined = self.work / "combined.json"
        joined.write_text(json.dumps(combined))
        self.assertEqual(data(BIN, "view", "replay", "compose", "--repo", self.repo, "--op", "union", "--left", mount_a, "--right", mount_b, "--output", joined)["view_id"], combined["view_id"])
        selected = data(BIN, "view", "select", "--repo", self.repo, "--manifest", joined, "--axis", "source-prefix", "--value", "h11")
        self.assertLessEqual(len(selected["entries"]), len(combined["entries"]))
        selection = self.work / "selected.json"
        selection.write_text(json.dumps(selected))
        dest = self.work / "view-output"
        receipt = data(BIN, "view", "materialize", "--repo", self.repo, "--manifest", selection, "--output", dest)
        audit = data(BIN, "view", "audit", "--repo", self.repo, "--manifest", selection, "--output", dest)
        self.assertEqual(receipt, audit)
        self.assertNotEqual(cmd(BIN, "view", "materialize", "--repo", self.repo, "--manifest", selection, "--output", dest, check=False).returncode, 0)

    def test_security_flow_view_and_inspection(self):
        _, db, run = self.report()
        plan = data(BIN, "view", "security", "--db", db, "--repo", self.repo, "--run", run)
        self.assertEqual(plan["kind"], "security-flow")
        manifest = self.work / "security-view.json"
        manifest.write_text(json.dumps(plan))
        data(BIN, "view", "replay", "security", "--db", db, "--repo", self.repo, "--run", run, "--manifest", manifest)
        impacted = data(BIN, "inspect", "impact", "--db", self.db, "--name", "h11", "--rev", "HEAD", "--seed", "h11._connection")
        self.assertIn("candidate_tests", impacted)
        self.assertTrue(data(BIN, "inspect", "manifest", "--db", self.db, "--name", "h11", "--rev", self.commit))
        self.assertTrue(data(BIN, "inspect", "dependencies", "--db", self.db, "--name", "h11", "--rev", self.commit))

    def test_scanner_pipeline_calls_dedicated_cli(self):
        report, _, _ = self.report()
        extra = self.work / "pipeline.sqlite"
        receipt = data(sys.executable, ROOT / "tools/pipeline.py", "import", "--engine", "codeql",
                       "--repo", self.repo, "--report", report, "--evidence", extra,
                       "--rust", BIN)
        self.assertTrue(receipt["receipt"]["independently_replayed"])

    def test_native_scanner_runner_uses_installed_engine_and_replays(self):
        # A controlled scanner process is used; CodeQL/Semgrep themselves are optional.
        fixture = self.work / "scanner-input.json"
        fixture.write_text(json.dumps({"version": "1.0.0", "errors": [], "results": [{
            "check_id": "controlled/scan", "path": "h11/_connection.py", "start": {"line": 20},
        }]}))
        config = self.work / "rules.yaml"
        config.write_text("rules: []\n")
        executable = self.work / "fake-semgrep"
        executable.write_text(f"#!{sys.executable}\nimport shutil, sys\nargs = sys.argv\nshutil.copyfile({str(fixture)!r}, args[args.index('--output') + 1])\n")
        executable.chmod(0o755)
        report = self.work / "report.json"
        evidence = self.work / "scan.sqlite"
        command = [BIN, "security", "scan", "--repo", self.repo, "--report", report,
                   "--db", evidence, "--engine", "semgrep", "--config", config,
                   "--scanner", executable, "--trusted-local", "true", "--timeout-seconds", "5"]
        scan = data(*command)
        self.assertTrue(scan["independently_replayed"])
        self.assertTrue(report.exists())
        self.assertEqual(scan["status"], "UNVERIFIED_STATIC_CANDIDATES")
        self.assertNotEqual(cmd(*command, check=False).returncode, 0, "refuse report overwrite")
        # Refuse untrusted default execution without explicit local trust approval.
        untrusted = [str(c) for c in command]
        untrusted[untrusted.index("--trusted-local") + 1] = "false"
        untrusted[untrusted.index("--report") + 1] = str(self.work / "new-report.json")
        bad = cmd(*untrusted, check=False)
        self.assertNotEqual(bad.returncode, 0)
        self.assertFalse((self.work / "new-report.json").exists())
        # Refuse any evidence output in the source checkout.
        in_repo = [str(c) for c in command]
        in_repo[in_repo.index("--report") + 1] = str(self.repo / "bad-report.json")
        self.assertNotEqual(cmd(*in_repo, check=False).returncode, 0)
        self.assertFalse((self.repo / "bad-report.json").exists())

    def test_integrated_browser_without_materializing(self):
        _, manifest = self.plan()
        proc = cmd(BIN, "browse", "--repo", self.repo, "--manifest", manifest, input="pwd\nls\ncd views\nls\nquit\n")
        text = proc.stdout.decode()
        self.assertIn("views/", text)
        self.assertIn("00-focus/", text)
        self.assertFalse((self.work / "views").exists())
        bad = cmd(BIN, "browse", "--repo", self.repo, "--manifest", manifest, input="cd ..\n", check=False)
        self.assertNotEqual(bad.returncode, 0)


if __name__ == "__main__":
    unittest.main()
