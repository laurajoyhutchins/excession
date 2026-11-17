"""Fail-closed tests for GitHub-native analysis request and result validation."""
import importlib.util
import json
import os
import sqlite3
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

    def test_owner_can_select_index_budgets_without_shell_interpolation(self):
        chosen = {
            "max_file_bytes": 4 * 1024 * 1024,
            "max_segment_bytes": 8 * 1024 * 1024,
            "max_segment_files": 3,
            "refresh_segments": 2,
        }
        value = module.validate(request(budgets=chosen), TARGETS)
        argv = module.command(value, Path("/source"), Path("/tool/excession"))
        for flag, expected in (
            ("--max-file-bytes", chosen["max_file_bytes"]),
            ("--max-segment-bytes", chosen["max_segment_bytes"]),
            ("--max-segment-files", chosen["max_segment_files"]),
            ("--index-segments", chosen["refresh_segments"]),
        ):
            self.assertEqual(argv[argv.index(flag) + 1], str(expected))
        observed = {
            "schema": module.SCHEMAS["locate"],
            "source_commit": SHA,
            "source_tree": BASE,
            "enrichment": "PERFORMED",
            "cache": {"refresh_budget_segments": chosen["refresh_segments"]},
            "index": {"limits": {
                "max_file_bytes": chosen["max_file_bytes"],
                "max_bytes_per_segment": chosen["max_segment_bytes"],
                "max_files_per_segment": chosen["max_segment_files"],
            }},
            "cache": {"refresh_budget_segments": chosen["refresh_segments"]},
        }
        module.check_report(value, observed, BASE)
        observed["index"]["limits"]["max_file_bytes"] = 1024 * 1024
        with self.assertRaises(module.Refusal):
            module.check_report(value, observed, BASE)

    def test_source_bound_cache_identity_and_explicit_enrichment(self):
        values = module.validate(request(
            repository="nodejs/node", enrich=True,
            budgets={"refresh_segments": 8},
        ), TARGETS | {"nodejs/node"})
        self.assertIn("--enrich", module.command(
            values, Path("/source"), Path("/tool/excession")))
        tool = "d" * 40
        key = module.cache_identity(values, tool)
        self.assertEqual(len(key), 64)
        self.assertEqual(key, module.cache_identity(
            {**values, "task": "Look for a different behavior"}, tool))
        self.assertNotEqual(key, module.cache_identity(
            {**values, "revision": BASE}, tool))
        self.assertNotEqual(key, module.cache_identity(
            {**values, "budgets": {"refresh_segments": 16}}, tool))
        self.assertNotEqual(key, module.cache_identity(values, "e" * 40))
        self.assertIsNone(module.cache_identity(request("context"), tool))
        for bad in ("true", 1, None):
            with self.subTest(bad=bad), self.assertRaises(module.Refusal):
                module.validate(request(enrich=bad), TARGETS)
        with self.assertRaises(module.Refusal):
            module.validate(request("context", enrich=True), TARGETS)
        with self.assertRaises(module.Refusal):
            module.cache_identity(values, "HEAD")

    def test_legitimate_git_only_report_can_omit_fts_but_not_requested_enrichment(self):
        value = request()
        bare = {
            "schema": module.SCHEMAS["locate"],
            "source_commit": SHA,
            "source_tree": BASE,
            "retrieval": "COMMITTED_GIT_GREP",
            "enrichment": "NOT_REQUESTED_GIT_SUFFICIENT",
            "index": None,
            "cache": None,
        }
        module.check_report(value, bare, BASE)
        with self.assertRaises(module.Refusal):
            module.check_report(request(enrich=True), bare, BASE)
        with self.assertRaises(module.Refusal):
            module.check_report(value, {**bare, "enrichment": "PERFORMED"}, BASE)

    def test_github_checkpoint_transport_is_artifact_based_not_issue_cache(self):
        workflow = (ROOT / ".github/workflows/github-native-analysis.yml").read_text()
        self.assertIn("tools/github_analysis.py cache-identity", workflow)
        self.assertIn("actions/download-artifact@v4", workflow)
        self.assertIn("actions/upload-artifact@v4", workflow)
        self.assertIn("tools/github_analysis.py restore-checkpoint", workflow)
        self.assertIn("tools/github_analysis.py publish-checkpoint", workflow)
        self.assertIn("actions: read", workflow)
        self.assertIn("EXCESSION_CACHE_DIR:", workflow)
        self.assertNotIn("actions/cache/save@v4", workflow)
        self.assertNotIn("actions/cache/restore@v4", workflow)
        self.assertLess(workflow.index("restore-checkpoint"), workflow.index("tools/github_analysis.py execute"))
        self.assertLess(workflow.index("tools/github_analysis.py execute"), workflow.index("publish-checkpoint"))

    def test_resume_must_be_pinned_and_explicitly_enriched(self):
        targets = TARGETS | {"nodejs/node"}
        value = module.validate(request(
            repository="nodejs/node", enrich=True, resume_run_id=12345
        ), targets)
        self.assertEqual(value["resume_run_id"], 12345)
        tool = "d" * 40
        self.assertEqual(
            module.cache_identity(value, tool),
            module.cache_identity({k: v for k, v in value.items() if k != "resume_run_id"}, tool),
        )
        for bad in (0, -1, "123", True, None, 2**63):
            with self.subTest(bad=bad), self.assertRaises(module.Refusal):
                module.validate(request(enrich=True, resume_run_id=bad), TARGETS)
        with self.assertRaises(module.Refusal):
            module.validate(request(resume_run_id=12345), TARGETS)
        with self.assertRaises(module.Refusal):
            module.validate(request("context", resume_run_id=12345), TARGETS)

    def test_checkpoint_artifact_is_identity_checked_and_tamper_evident(self):
        tool_commit = "d" * 40
        request_value = module.validate(request(enrich=True), TARGETS)
        run_id = 123456789
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            cache_root = root / "cache"
            cache_root.mkdir()
            native = cache_root / "a-prefix"
            native.mkdir()
            native_db = native / "agent-locate-chunks.sqlite"
            with sqlite3.connect(native_db) as db:
                db.execute("CREATE TABLE locator_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
                db.execute("INSERT INTO locator_meta VALUES('version', 'agent-locate-segments-v3')")
                db.execute("CREATE TABLE locator_segments(segment INTEGER PRIMARY KEY, fingerprint TEXT)")
                db.execute("INSERT INTO locator_segments VALUES(12, 'fingerprint')")

            report_root = root / "evidence"
            report_root.mkdir()
            report = {
                "schema": "excession.agent-locate.v1",
                "source_commit": SHA,
                "source_tree": BASE,
                "enrichment": "PERFORMED",
                "report_id": "fixed-report",
                "index": {
                    "limits": {
                        "max_file_bytes": 1024 * 1024,
                        "max_files_per_segment": 6000,
                        "max_bytes_per_segment": 32 * 1024 * 1024,
                    },
                    "ready_segments": 1,
                    "pending_segments": 255,
                },
                "cache": {
                    "kind": "PERSISTENT_SQLITE_FTS5",
                    "path": str(native_db),
                    "refresh_budget_segments": 256,
                },
            }
            report_file = report_root / "report.json"
            report_file.write_text(json.dumps(report))
            receipt = {
                "status": "COMPLETED",
                "request": request_value,
                "tool_commit": tool_commit,
                "source_tree": BASE,
                "report_id": report["report_id"],
                "report_sha256": module.file_digest(report_file),
            }
            (report_root / "receipt.json").write_text(json.dumps(receipt))
            artifact = root / "artifact"
            self.assertTrue(module.publish_checkpoint(
                request_value, report_root, artifact, tool_commit, run_id, cache_root,
            ))
            metadata = {
                "id": run_id,
                "event": "issues",
                "conclusion": "success",
                "name": "Excession analysis request 123456789",
                "path": ".github/workflows/github-native-analysis.yml",
                "head_sha": tool_commit,
                "head_branch": "main",
                "actor": {"id": module.OWNER_ID},
                "repository": {"full_name": "laurajoyhutchins/excession"},
            }
            restored_root = root / "fresh-cache"
            destination = module.restore_checkpoint(
                {**request_value, "resume_run_id": run_id}, artifact, metadata,
                tool_commit, restored_root, report_root,
            )
            self.assertTrue(destination.is_file())
            self.assertEqual(module.file_digest(destination), module.file_digest(
                artifact / "locator.sqlite"))
            self.assertEqual(sqlite3.connect(destination).execute(
                "SELECT COUNT(*) FROM locator_segments").fetchone(), (1,))
            with self.assertRaises(module.Refusal):
                module.restore_checkpoint(
                    {**request_value, "resume_run_id": run_id + 1}, artifact,
                    metadata, tool_commit, root / "wrong-cache", report_root,
                )
            with self.assertRaises(module.Refusal):
                module.restore_checkpoint(
                    {**request_value, "resume_run_id": run_id}, artifact,
                    {**metadata, "actor": {"id": 1234}}, tool_commit,
                    root / "forged-cache", report_root,
                )
            with self.assertRaises(module.Refusal):
                module.restore_checkpoint(
                    {**request_value, "resume_run_id": run_id}, artifact,
                    {**metadata, "path": ".github/workflows/untrusted.yml"}, tool_commit,
                    root / "wrong-workflow-cache", report_root,
                )

            with self.assertRaises(module.Refusal):
                module.restore_checkpoint(
                    {**request_value, "resume_run_id": run_id}, artifact,
                    metadata, "e" * 40, root / "stale-cache", report_root,
                )
            with (artifact / "locator.sqlite").open("ab") as f:
                f.write(b"tamper")
            with self.assertRaises(module.Refusal):
                module.restore_checkpoint(
                    {**request_value, "resume_run_id": run_id}, artifact,
                    metadata, tool_commit, root / "tampered-cache", report_root,
                )


    def test_malformed_or_cross_operation_budgets_refused(self):
        bad = (
            {}, {"unknown": 2}, {"refresh_segments": 257},
            {"max_file_bytes": True}, {"max_file_bytes": 0}, {"max_file_bytes": 3},
            {"max_file_bytes": -1}, {"max_segment_files": "3"},
            {"max_segment_bytes": 2},
            {"max_file_bytes": 16 * 1024 * 1024, "max_segment_bytes": 1 * 1024 * 1024},
        )
        for budgets in bad:
            with self.subTest(budgets=budgets), self.assertRaises(module.Refusal):
                module.validate(request(budgets=budgets), TARGETS)
        with self.assertRaises(module.Refusal):
            module.validate(request("context", budgets={"max_file_bytes": 1024}), TARGETS)

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

    def test_nodejs_core_public_analysis_target(self):
        # An unfamiliar, large public repository must be admitted explicitly.
        # This tests authorization only; it does not assert indexing completeness.
        targets = module.load_targets(ROOT / "config/github-analysis-targets.json")
        self.assertIn("nodejs/node", targets)
        value = request(
            repository="nodejs/node",
            task="Trace CommonJS require caching, cycles, and their tests",
        )
        validated = module.validate(value, targets)
        self.assertEqual(validated["repository"], "nodejs/node")
        with self.assertRaises(module.Refusal):
            module.validate(request(repository="unknown/project"), targets)

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
