"""Fail-closed tests for GitHub-native analysis request and result validation."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
