"""Real Git-fixture tests for the explicitly trusted structural preservation runner."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SRC = Path(__file__).resolve().parents[1] / "tools" / "ci" / "structural_preservation.py"
spec = importlib.util.spec_from_file_location("structural_preservation", SRC)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def git(repo, *args):
    return subprocess.check_output(("git", "-C", str(repo), *args), text=True).strip()


class StructuralPreservation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Fixture")
        git(self.repo, "config", "user.email", "fixture@example.invalid")
        self.write("module.py", "def answer():\n    return 6 * 7\n")
        self.base = self.commit()
        self.write("module.py", "def answer():\n    return 40 + 2\n")
        self.candidate = self.commit()
        self.doc = {
            "schema": runner.SCHEMA,
            "changed_paths": ["module.py"],
            "obligations": [
                {"id": "observable-output", "kind": "differential",
                 "cwd": ".", "argv": ["python3", "-B", "-c",
                    "import runpy; print(runpy.run_path('module.py')['answer']())"],
                 "timeout_seconds": 10, "max_output_bytes": 1024},
                {"id": "candidate-regression", "kind": "candidate_pass",
                 "cwd": ".", "argv": ["python3", "-B", "-c",
                    "import runpy; assert runpy.run_path('module.py')['answer']() == 42"],
                 "timeout_seconds": 10, "max_output_bytes": 1024},
            ],
        }

    def write(self, name, content):
        file = self.repo / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content, encoding="utf-8")

    def commit(self):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "fixture")
        return git(self.repo, "rev-parse", "HEAD")

    def evaluate(self, base=None, candidate=None):
        return runner.evaluate(self.repo, base or self.base, candidate or self.candidate,
                               self.doc, "contract-digest")

    def test_equivalent_observations_are_scoped_not_merge_approval(self):
        report = self.evaluate()
        self.assertEqual(report["status"], "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE")
        self.assertFalse(report["ready_to_merge"])
        self.assertFalse(report["safe_to_skip_unlisted_tests"])
        self.assertEqual(report["semantic_equivalence"], "NOT_PROVED")
        self.assertEqual([o["result"] for o in report["obligations"]], ["PASS", "PASS"])
        self.assertEqual(report["changed_paths"], ["module.py"])
        self.assertEqual(report["candidate_tree"], git(self.repo, "rev-parse", self.candidate + "^{tree}"))
        self.assertEqual(len(git(self.repo, "worktree", "list").splitlines()), 1)

    def test_changed_behavior_is_a_counterexample(self):
        self.write("module.py", "def answer():\n    return 43\n")
        bad = self.commit()
        report = self.evaluate(candidate=bad)
        self.assertEqual(report["status"], "FAILED")
        self.assertEqual(report["obligations"][0]["result"], "FAIL")
        self.assertEqual(report["obligations"][1]["result"], "FAIL")

    def test_relative_executable_runs_in_each_checked_out_tree(self):
        # Rebuild baseline and candidate commits so the executable belongs
        # to each pinned tree instead of the CI runner's current checkout.
        git(self.repo, "reset", "--hard", self.base)
        self.write("runner.sh", "#!/bin/sh\npython3 -B -c \"import runpy; print(runpy.run_path('module.py')['answer']())\"\n")
        (self.repo / "runner.sh").chmod(0o755)
        self.base = self.commit()
        self.write("module.py", "def answer():\n    return 40 + 2\n")
        self.candidate = self.commit()
        self.doc["obligations"][0]["argv"] = ["./runner.sh"]
        self.assertEqual(self.evaluate()["status"],
                         "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE")

    def test_excessive_stdout_is_inconclusive_without_spooling(self):
        self.doc["obligations"][1]["argv"] = [
            "python3", "-B", "-c",
            "import os; os.write(1, b'x' * (1024 * 1024))",
        ]
        receipt = self.evaluate()["obligations"][1]["candidate"]
        self.assertEqual(receipt["status"], "INCONCLUSIVE")
        self.assertEqual(receipt["reason"], "OUTPUT_BUDGET")
        self.assertIsNone(receipt["stdout_sha256"])
        self.assertEqual(receipt["stdout_bytes"], 1025)  # budget plus one sentinel byte

    def test_small_binary_output_matches_independent_hash(self):
        payload = bytes(range(256))
        self.doc["obligations"][1]["argv"] = [
            "python3", "-B", "-c",
            "import os; os.write(1, bytes(range(256))); os.write(2, bytes(reversed(range(256))))",
        ]
        receipt = self.evaluate()["obligations"][1]["candidate"]
        self.assertEqual(receipt["status"], "OBSERVED")
        self.assertEqual(receipt["stdout_bytes"], len(payload))
        self.assertEqual(receipt["stdout_sha256"], hashlib.sha256(payload).hexdigest())
        self.assertEqual(receipt["stderr_bytes"], len(payload))
        self.assertEqual(receipt["stderr_sha256"], hashlib.sha256(payload[::-1]).hexdigest())

    def test_excessive_stderr_is_inconclusive_at_first_overflow_byte(self):
        self.doc["obligations"][1]["argv"] = [
            "python3", "-B", "-c",
            "import os; os.write(2, b'e' * (1024 * 1024))",
        ]
        receipt = self.evaluate()["obligations"][1]["candidate"]
        self.assertEqual(receipt["status"], "INCONCLUSIVE")
        self.assertEqual(receipt["reason"], "OUTPUT_BUDGET")
        self.assertEqual(receipt["stderr_bytes"], 1025)
        self.assertIsNone(receipt["stderr_sha256"])

    def test_explicit_timeout_has_distinct_reason(self):
        home = self.root / "timeout-home"
        home.mkdir()
        check = dict(self.doc["obligations"][1])
        check.update(
            argv=[sys.executable, "-B", "-c", "import time; time.sleep(30)"],
            timeout_seconds=1,
        )
        receipt = runner.run_command(self.repo, check, home)
        self.assertEqual(receipt["status"], "INCONCLUSIVE")
        self.assertEqual(receipt["reason"], "TIMEOUT")
        self.assertTrue(receipt["timed_out"])
        self.assertIsNone(receipt["stdout_sha256"])

    @unittest.skipUnless(os.name == "posix" and Path("/proc").is_dir(),
                         "independent process-state oracle requires Linux /proc")
    def test_descendant_retaining_pipe_is_terminated_after_parent_exit(self):
        home = self.root / "descendant-home"
        home.mkdir()
        worker = (
            "import os,time;from pathlib import Path;"
            "Path(os.environ['HOME'],'descendant.pid').write_text(str(os.getpid()));"
            "time.sleep(30)"
        )
        launcher = (
            "import subprocess,sys,os,time;"
            f"subprocess.Popen([sys.executable,'-B','-c',{worker!r}]);"
            "p=os.path.join(os.environ['HOME'],'descendant.pid');"
            "[(time.sleep(.01)) for _ in range(100) if not os.path.exists(p)]"
        )
        check = dict(self.doc["obligations"][1])
        check.update(argv=[sys.executable, "-B", "-c", launcher],
                     timeout_seconds=3)
        receipt = runner.run_command(self.repo, check, home)
        marker = home / "descendant.pid"
        self.assertTrue(marker.exists(), "worker did not start; invalid cleanup fixture")
        pid = int(marker.read_text())

        def still_running():
            # A reparented, unreaped zombie is no longer executing.
            try:
                state = (Path("/proc") / str(pid) / "stat").read_text()
            except FileNotFoundError:
                return False
            return state.rsplit(") ", 1)[1].split()[0] not in ("Z", "X")

        try:
            self.assertEqual(receipt["reason"], "TIMEOUT")
            self.assertEqual(receipt["exit_code"], 0)  # launcher exited; pipe owner did not
            for _ in range(40):
                if not still_running():
                    break
                time.sleep(0.025)
            self.assertFalse(still_running(), "inherited-pipe descendant survived timeout")
        finally:
            if still_running():
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    @unittest.skipUnless(os.name == "posix" and Path("/proc").is_dir(),
                         "independent process-state oracle requires Linux /proc")
    def test_successful_parent_cannot_leave_background_worker_running(self):
        home = self.root / "background-home"
        home.mkdir()
        worker = (
            "import os,time;from pathlib import Path;"
            "Path(os.environ['HOME'],'background.pid').write_text(str(os.getpid()));"
            "os.close(1);os.close(2);time.sleep(30)"
        )
        launcher = (
            "import subprocess,sys,os,time;"
            f"subprocess.Popen([sys.executable,'-B','-c',{worker!r}]);"
            "p=os.path.join(os.environ['HOME'],'background.pid');"
            "[(time.sleep(.01)) for _ in range(100) if not os.path.exists(p)]"
        )
        check = dict(self.doc["obligations"][1])
        check.update(argv=[sys.executable, "-B", "-c", launcher], timeout_seconds=3)
        receipt = runner.run_command(self.repo, check, home)
        marker = home / "background.pid"
        self.assertTrue(marker.exists(), "worker never started")
        pid = int(marker.read_text())

        def running():
            try:
                stat = (Path("/proc") / str(pid) / "stat").read_text()
            except FileNotFoundError:
                return False
            return stat.rsplit(") ", 1)[1].split()[0] not in ("Z", "X")

        try:
            self.assertEqual(receipt["status"], "OBSERVED")
            self.assertEqual(receipt["exit_code"], 0)
            for _ in range(40):
                if not running():
                    break
                time.sleep(.025)
            self.assertFalse(running(), "successful launcher leaked background worker")
        finally:
            if running():
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_namespace_contract_requires_an_explicit_scratch_budget(self):
        obligation = self.doc["obligations"][1]
        obligation["isolation"] = "linux_namespaces"
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaisesRegex(runner.Refused, "scratch-byte limit"):
            runner.read_contract(path)
        obligation["max_scratch_bytes"] = 8 * 1024 * 1024
        path.write_text(json.dumps(self.doc))
        self.assertEqual(runner.read_contract(path)[0], self.doc)
        obligation["isolation"] = "trusted_local"
        path.write_text(json.dumps(self.doc))
        with self.assertRaisesRegex(runner.Refused, "requires linux_namespaces"):
            runner.read_contract(path)

    def test_namespace_builder_has_read_only_source_and_private_writable_paths(self):
        args = runner.linux_namespace_argv(
            "/usr/bin/bwrap", self.repo, self.repo,
            ["python3", "-B", "-c", "print(42)"], 64 * 1024 * 1024,
        )
        self.assertEqual(args[:2], ["/usr/bin/bwrap", "--unshare-user"])
        for flag in ("--unshare-pid", "--unshare-net", "--die-with-parent"):
            self.assertIn(flag, args)
        self.assertIn(["--ro-bind", str(self.repo), "/tmp/excession-worktree"],
                      [args[i:i + 3] for i in range(len(args) - 2)])
        self.assertNotIn("--bind", args)
        self.assertIn(["--size", str(64 * 1024 * 1024), "--tmpfs", "/tmp"],
                      [args[i:i + 4] for i in range(len(args) - 3)])
        self.assertEqual(args[-3:], ["-B", "-c", "print(42)"])

    def test_namespace_mode_fails_closed_when_bubblewrap_is_missing(self):
        from unittest.mock import patch
        home = self.root / "isolated-home"
        home.mkdir()
        check = dict(self.doc["obligations"][1])
        check.update(argv=[sys.executable, "-B", "-c", "print('not executed')"],
                     isolation="linux_namespaces", max_scratch_bytes=16 * 1024 * 1024)
        with patch.object(runner.shutil, "which", return_value=None):
            receipt = runner.run_command(self.repo, check, home)
        self.assertEqual(receipt["status"], "INCONCLUSIVE")
        self.assertEqual(receipt["reason"], "SANDBOX_UNAVAILABLE")

    @unittest.skipUnless(sys.platform == "linux", "requires Linux namespaces")
    def test_namespace_observation_and_private_tmp_when_available(self):
        import shutil
        if not shutil.which("bwrap"):
            self.skipTest("bubblewrap not installed")
        home = self.root / "isolated-home"
        home.mkdir()
        host_marker = self.root / "sandbox-private-marker"
        script = (
            "from pathlib import Path; import sys;"
            "Path(sys.argv[1]).write_bytes(b'inside-only');"
            "print('namespace-ok')"
        )
        check = dict(self.doc["obligations"][1])
        check.update(
            argv=[sys.executable, "-B", "-c", script, str(host_marker)],
            isolation="linux_namespaces", max_scratch_bytes=16 * 1024 * 1024,
        )
        receipt = runner.run_command(self.repo, check, home)
        if receipt["reason"] == "SANDBOX_UNAVAILABLE":
            self.skipTest("bubblewrap namespace setup unavailable on host")
        self.assertEqual(receipt["status"], "OBSERVED")
        self.assertEqual(receipt["exit_code"], 0)
        self.assertEqual(receipt["stdout_sha256"],
                         hashlib.sha256(b"namespace-ok\\n").hexdigest())
        self.assertFalse(host_marker.exists(), "sandbox wrote into host /tmp")

    @unittest.skipUnless(sys.platform == "linux" and Path("/proc").is_dir(),
                         "Linux /proc independent process oracle required")
    def test_namespace_does_not_leak_setsid_descendant_when_available(self):
        import shutil
        import uuid
        if not shutil.which("bwrap"):
            self.skipTest("bubblewrap not installed")
        home = self.root / "isolated-descendant-home"
        home.mkdir()
        token = "excession-ns-probe-" + uuid.uuid4().hex
        worker = (
            "import os,sys,time;"
            "os.setsid();"
            "os.write(int(sys.argv[1]),b'1');"
            "os.close(int(sys.argv[1]));"
            "time.sleep(30)"
        )
        launcher = (
            "import os,subprocess,sys;"
            "r,w=os.pipe();"
            f"p=subprocess.Popen([sys.executable,'-B','-c',{worker!r},str(w),{token!r}],"
            "pass_fds=(w,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);"
            "os.close(w);"
            "assert os.read(r,1)==b'1';"
            "os.close(r);"
            "print('detached-started')"
        )
        check = dict(self.doc["obligations"][1])
        check.update(
            argv=[sys.executable, "-B", "-c", launcher],
            isolation="linux_namespaces", max_scratch_bytes=16 * 1024 * 1024,
            timeout_seconds=5,
        )
        receipt = runner.run_command(self.repo, check, home)
        if receipt["reason"] == "SANDBOX_UNAVAILABLE":
            self.skipTest("bubblewrap namespaces unavailable on host")

        def live_pids():
            result = []
            for proc in Path("/proc").iterdir():
                if not proc.name.isdecimal():
                    continue
                try:
                    cmdline = (proc / "cmdline").read_bytes()
                    state = (proc / "stat").read_text().rsplit(") ", 1)[1].split()[0]
                except (OSError, IndexError):
                    continue
                if token.encode() in cmdline and state not in ("Z", "X"):
                    result.append(int(proc.name))
            return result

        try:
            self.assertEqual(receipt["status"], "OBSERVED")
            self.assertEqual(receipt["exit_code"], 0)
            self.assertEqual(receipt["stdout_sha256"],
                             hashlib.sha256(b"detached-started\\n").hexdigest())
            for _ in range(50):
                if not live_pids():
                    break
                time.sleep(.02)
            self.assertEqual(live_pids(), [], "detached worker survived PID namespace exit")
        finally:
            for pid in live_pids():
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_missing_test_tool_is_inconclusive_not_green(self):
        self.doc["obligations"][1]["argv"] = ["excession-tool-never-installed-xyz"]
        report = self.evaluate()
        self.assertEqual(report["status"], "INCONCLUSIVE")
        self.assertEqual(report["obligations"][1]["candidate"]["reason"], "EXECUTABLE_UNAVAILABLE")

    def test_nonzero_baseline_cannot_masquerade_as_equivalence(self):
        self.doc["obligations"][0]["argv"] = ["python3", "-c", "raise SystemExit(7)"]
        self.assertEqual(self.evaluate()["obligations"][0]["result"], "FAIL")

    def test_wrong_changed_paths_are_refused(self):
        self.doc["changed_paths"] = ["different.py"]
        with self.assertRaises(runner.Refused):
            self.evaluate()

    def test_changed_symlink_is_refused_before_execution(self):
        import os
        (self.repo / "module.py").unlink()
        os.symlink("missing-target.py", self.repo / "module.py")
        candidate = self.commit()
        with self.assertRaisesRegex(runner.Refused, "not a regular tracked Git source file"):
            self.evaluate(candidate=candidate)

    def test_identity_and_ancestry_are_authoritative(self):
        with self.assertRaises(runner.Refused):
            self.evaluate(base="HEAD")
        with self.assertRaises(runner.Refused):
            self.evaluate(base=self.candidate, candidate=self.base)

    def test_candidate_mutating_tracked_source_refuses_and_cleans_worktrees(self):
        self.doc["obligations"][1]["argv"] = [
            "python3", "-B", "-c",
            "from pathlib import Path; Path('module.py').write_text('corrupted')",
        ]
        with self.assertRaisesRegex(runner.Refused, "modified tracked Git source"):
            self.evaluate()
        self.assertEqual(len(git(self.repo, "worktree", "list").splitlines()), 1)
        self.assertEqual((self.repo / "module.py").read_text(),
                         "def answer():\n    return 40 + 2\n")

    def test_separate_home_environments_do_not_share_hidden_state(self):
        self.doc["obligations"][0]["argv"] = [
            "python3", "-B", "-c",
            "from pathlib import Path; import os; home = Path(os.environ['HOME']); "
            "print((home / 'marker').exists()); (home / 'marker').write_text('state')",
        ]
        self.doc["obligations"][1]["argv"] = [
            "python3", "-B", "-c",
            "from pathlib import Path; import os; assert not (Path(os.environ['HOME']) / 'marker').exists()",
        ]
        # Every obligation has new source worktrees and HOME directories; the
        # candidate's second command cannot see the first one's marker.
        self.assertEqual(self.evaluate()["status"],
                         "OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE")

    def test_missing_cwd_refuses(self):
        self.doc["obligations"][0]["cwd"] = "not-a-directory"
        with self.assertRaises(runner.Refused):
            self.evaluate()

    def test_contract_rejects_semantic_shortcuts(self):
        self.doc["obligations"] = [self.doc["obligations"][1]]
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaises(runner.Refused):
            runner.read_contract(path)

    def test_contract_rejects_traversal(self):
        self.doc["obligations"][0]["cwd"] = "../outside"
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaises(runner.Refused):
            runner.read_contract(path)

    def test_contract_rejects_bad_output_budget(self):
        self.doc["obligations"][0]["max_output_bytes"] = -1
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        with self.assertRaises(runner.Refused):
            runner.read_contract(path)

    def test_manifest_is_content_addressed(self):
        path = self.root / "contract.json"
        raw = json.dumps(self.doc, sort_keys=True).encode()
        path.write_bytes(raw)
        manifest, sha = runner.read_contract(path)
        self.assertEqual(manifest, self.doc)
        self.assertEqual(sha, hashlib.sha256(raw).hexdigest())

    def test_cli_refuses_without_trust_opt_in(self):
        path = self.root / "contract.json"
        path.write_text(json.dumps(self.doc))
        process = subprocess.run(
            ["python3", str(SRC), "--repo", str(self.repo),
             "--base", self.base, "--candidate", self.candidate,
             "--contract", str(path), "--out", str(self.root / "receipt.json")],
            capture_output=True,
        )
        self.assertNotEqual(process.returncode, 0)
        self.assertFalse((self.root / "receipt.json").exists())


if __name__ == "__main__":
    unittest.main()
