"""Fail-closed tests for GitHub-native analysis request and result validation."""
import importlib.util
import json
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("github_analysis", ROOT / "tools/github_analysis.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

SHA = "a" * 40
BASE = "b" * 40
TARGETS = {"laurajoyhutchins/excession", "python-hyper/h11"}


def request(operation="locate", **overrides):
    value = {
        "schema": module.REQUEST_SCHEMA,
        "repository": "laurajoyhutchins/excession",
        "revision": SHA,
        "operation": operation,
    }
    value.update({"locate": {"task": "Find source verification"},
                  "context": {"focus": ["core/src/agent.rs"]},
                  "verify": {"base": BASE}}.get(operation, {}))
    value.update(overrides)
    return value


def issue(value=None, owner=module.OWNER_ID):
    return {"action": "opened", "issue": {
        "title": "Excession analysis request",
        "user": {"id": owner},
        "body": json.dumps(value or request()),
    }}


class RequestTests(unittest.TestCase):
    def test_minimal_modes(self):
        for mode in ("locate", "context", "verify"):
            with self.subTest(mode=mode):
                self.assertEqual(module.validate(request(mode), TARGETS)["limit"], 16)

    def test_owner_issue_and_dispatch(self):
        self.assertEqual(module.from_event(issue(), 0, TARGETS)["revision"], SHA)
        self.assertEqual(module.from_event(
            {"inputs": {"request": json.dumps(request())}},
            module.OWNER_ID, TARGETS)["revision"], SHA)

    def test_unauthorized_requests(self):
        for event, actor in ((issue(owner=2), 2),
                             ({"inputs": {"request": json.dumps(request())}}, 2)):
            with self.assertRaises(module.Refusal):
                module.from_event(event, actor, TARGETS)

    def test_irrelevant_issue_and_pr_refused(self):
        event = issue()
        event["issue"]["title"] = "Other"
        with self.assertRaises(module.Refusal):
            module.from_event(event, module.OWNER_ID, TARGETS)
        event = issue()
        event["issue"]["pull_request"] = {"url": "https://example.invalid"}
        with self.assertRaises(module.Refusal):
            module.from_event(event, module.OWNER_ID, TARGETS)

    def test_unknown_repo_fields_and_sha(self):
        for fields in ({"repository": "attacker/evil"}, {"command": "sh -c bad"},
                       {"operation": "shell"}, {"revision": "HEAD"}, {"revision": "A" * 40}):
            with self.subTest(fields=fields), self.assertRaises(module.Refusal):
                module.validate(request(**fields), TARGETS)

    def test_cross_operation_fields(self):
        for operation, fields in (("locate", {"base": BASE}),
                                  ("context", {"base": BASE}),
                                  ("verify", {"task": "execute"})):
            with self.subTest(operation=operation), self.assertRaises(module.Refusal):
                module.validate(request(operation, **fields), TARGETS)

    def test_limit_refuses_bad_values(self):
        for value in (0, 65, -1, True, 1.2, "1"):
            with self.subTest(value=value), self.assertRaises(module.Refusal):
                module.validate(request(limit=value), TARGETS)

    def test_focus_path_traversal(self):
        for path in ("/etc/passwd", "../src", "src/../a", "a//b", "a,b",
                     "a\\b", "a\x00b", ""):
            with self.subTest(path=path), self.assertRaises(module.Refusal):
                module.validate(request("context", focus=[path]), TARGETS)

    def test_invalid_task(self):
        for task in ("", "ab", "x" * 4001, "abc\x00def"):
            with self.subTest(length=len(task)), self.assertRaises(module.Refusal):
                module.validate(request(task=task), TARGETS)

    def test_json_strictness(self):
        with self.assertRaises(module.Refusal):
            module.load_request('{"schema":"a","schema":"b"}')
        fence = chr(96) * 3
        value = module.load_request(fence + "json\n" + json.dumps(request()) + "\n" + fence)
        self.assertEqual(value["revision"], SHA)
        with self.assertRaises(module.Refusal):
            module.load_request("z" * 8193)

    def test_literal_args_without_shell(self):
        val = module.validate(request(task="literal ; $(whoami)"), TARGETS)
        argv = module.command(val, Path("/some/repo"), Path("/bin/excession"))
        self.assertEqual(argv[argv.index("--task") + 1], "literal ; $(whoami)")
        self.assertNotIn("sh", argv)

    def test_native_identity_contract(self):
        with self.assertRaises(module.Refusal):
            module.check_report(request(), {"schema": module.SCHEMAS["locate"],
                                            "source_commit": BASE, "source_tree": BASE}, SHA)
        module.check_report(request("verify"),
                            {"schema": module.SCHEMAS["verify"], "base_commit": BASE,
                             "candidate_commit": SHA, "candidate_tree": BASE}, BASE)

    def test_allowlist_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "targets.json"
            path.write_text(json.dumps({"schema": "excession.github-analysis-targets.v1",
                                        "repositories": ["laurajoyhutchins/excession"]}))
            self.assertEqual(module.load_targets(path), {"laurajoyhutchins/excession"})
            path.write_text(json.dumps({"schema": "excession.github-analysis-targets.v1",
                                        "repositories": ["attacker/../../bad"]}))
            with self.assertRaises(module.Refusal):
                module.load_targets(path)


class PrivateCallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name) / "private"
        self.repo.mkdir()
        subprocess.run(["git", "-C", str(self.repo), "init", "-q"], check=True)
        for argv in (("config", "user.email", "fixture@example.invalid"),
                     ("config", "user.name", "Fixture")):
            subprocess.run(["git", "-C", str(self.repo), *argv], check=True)
        (self.repo / "source.rs").write_text("pub fn value() -> i32 { 42 }\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "source.rs"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qm", "base"], check=True)
        self.base = self.git("rev-parse", "HEAD")
        (self.repo / "source.rs").write_text("pub fn value() -> i32 { 43 }\n")
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qam", "candidate"], check=True)
        self.candidate = self.git("rev-parse", "HEAD")
        self.tree = self.git("rev-parse", "HEAD^{tree}")

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-C", str(self.repo), *args], text=True
        ).strip()

    def test_trusted_checkout_uses_original_git_objects_without_network(self):
        request = {
            "schema": module.REQUEST_SCHEMA,
            "repository": "laurajoyhutchins/arcata",
            "revision": self.candidate,
            "operation": "verify",
            "base": self.base,
            "limit": 3,
        }
        with patch.object(module, "git", wraps=module.git) as inspected:
            source, tree = module.prepare_checkout_source(request, self.repo)
        self.assertEqual(source, self.repo.resolve())
        self.assertEqual(tree, self.tree)
        self.assertFalse(any("fetch" in call.args for call in inspected.call_args_list))
        self.assertFalse(any("update-ref" in call.args for call in inspected.call_args_list))
        self.assertEqual(self.git("rev-parse", "HEAD"), self.candidate)

    def test_invalid_private_revision_and_checkout_refused(self):
        request = {"revision": "f" * 40}
        with self.assertRaises(module.Refusal):
            module.prepare_checkout_source(request, self.repo)
        with self.assertRaises(module.Refusal):
            module.prepare_checkout_source({"revision": self.candidate},
                                           self.repo / ".git")

    def test_private_authorization_and_unchanged_public_allowlist(self):
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "laurajoyhutchins/arcata"}):
            private = {"laurajoyhutchins/arcata"}
            value = {
                "schema": module.REQUEST_SCHEMA,
                "repository": "laurajoyhutchins/arcata",
                "revision": self.candidate,
                "operation": "locate",
                "task": "Find a transformer sizing routine",
            }
            self.assertEqual(module.from_event(issue(value), 0, private)["limit"], 16)
            with self.assertRaises(module.Refusal):
                module.validate(value, TARGETS)
            with self.assertRaises(module.Refusal):
                module.from_event(issue(value, owner=999), 0, private)

    def test_private_report_executes_pinned_local_git_not_public_clone(self):
        stub = Path(self.tmp.name) / "fake-excession"
        stub.write_text(
            "#!/usr/bin/env python3\n"
            "import json,sys\n"
            "if '--version' in sys.argv: print('fake-excession'); sys.exit(0)\n"
            f"print(json.dumps({{'schema':'excession.agent-context.v1',"
            f"'source_commit':'{self.candidate}','source_tree':'{self.tree}'}}))\n"
        )
        stub.chmod(0o755)
        request = {
            "schema": module.REQUEST_SCHEMA,
            "repository": "laurajoyhutchins/arcata",
            "revision": self.candidate,
            "operation": "context",
            "focus": ["source.rs"],
            "limit": 3,
        }
        out = Path(self.tmp.name) / "out"
        with patch.object(module, "prepare_source",
                          side_effect=AssertionError("public clone used")):
            module.execute(request, stub, out,
                           source_checkout=self.repo, tool_checkout=self.repo)
        receipt = json.loads((out / "receipt.json").read_text())
        report = json.loads((out / "report.json").read_text())
        self.assertEqual(receipt["status"], "COMPLETED")
        self.assertEqual(receipt["source_tree"], self.tree)
        self.assertEqual(receipt["tool_commit"], self.candidate)
        self.assertFalse(receipt["safe_to_skip_unlisted_tests"])
        self.assertEqual(report["source_commit"], self.candidate)


if __name__ == "__main__":
    unittest.main()
