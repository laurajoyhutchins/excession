"""Independent, two-revision CLI acceptance for Excession's Python impact scope.

The expected graph comes from CPython AST over Git blobs, not Excession SQL
or its import adapter. This is *not* a runtime dependency completeness test.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "core/target/debug/excession"
sys.path.insert(0, str(ROOT / "benchmarks" / "experiments"))
from dependency_oracle import module_name, python_edges  # noqa: E402


def command(*argv: object, check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        [str(arg) for arg in argv], text=True, capture_output=True,
        check=False, timeout=90,
    )
    if check and completed.returncode != 0:
        raise AssertionError(
            f"{' '.join(map(str, argv))} returned {completed.returncode}\n"
            f"stdout: {completed.stdout[-2000:]}\nstderr: {completed.stderr[-3000:]}"
        )
    return completed


def git(repo: Path, *args: str) -> str:
    return command("git", "-C", repo, *args).stdout.strip()


def git_inventory(repo: Path, commit: str) -> tuple[str, dict[str, tuple[str, str]]]:
    """Read Git's own mode/blob manifest, without Excession's change inventory."""
    output = subprocess.run(
        ["git", "-C", str(repo), "ls-tree", "-r", "-z", commit],
        capture_output=True, check=True, timeout=30,
    ).stdout
    files: dict[str, tuple[str, str]] = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        metadata, path = record.split(b"\t", 1)
        mode, kind, oid = metadata.decode("ascii").split(" ")
        if kind == "blob":
            files[path.decode("utf-8")] = (oid, mode)
    return git(repo, "rev-parse", f"{commit}^{{tree}}"), files


def oracle_paths(
    repo: Path, commit: str, changed: set[str],
) -> tuple[set[str], set[tuple[str, str]], str, dict[str, tuple[str, str]]]:
    """Compute reverse reachability from a separately parsed Python graph."""
    tree, files = git_inventory(repo, commit)
    modules = {module_name(p): p for p in files if p.startswith("pkg/") and p.endswith(".py")}
    sources = {p: git(repo, "show", f"{commit}:{p}") for p in modules.values()}
    references, unresolved = python_edges(sources, modules)
    if unresolved:
        raise AssertionError(f"independent fixture must have no unresolved imports: {unresolved}")
    edges = {(modules[source], modules[target]) for source, target, _, _ in references}
    reverse: dict[str, set[str]] = {}
    for source, target in edges:
        reverse.setdefault(target, set()).add(source)
    reached = set(changed)
    queue = sorted(changed)
    affected: set[str] = set()
    while queue:
        target = queue.pop(0)
        for source in sorted(reverse.get(target, ())):
            if source not in reached:
                reached.add(source)
                affected.add(source)
                queue.append(source)
    return affected, edges, tree, files


class ImpactIndependentAcceptance(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not BIN.is_file():
            if os.environ.get("EXCESSION_REQUIRE_BIN") == "1":
                raise AssertionError("mandatory native CLI is missing; build excession --bin first")
            raise unittest.SkipTest("native Excession binary not built")

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory(prefix="excession-impact-oracle-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        command("git", "init", "-q", "-b", "main", self.repo)
        git(self.repo, "config", "user.name", "Impact acceptance")
        git(self.repo, "config", "user.email", "impact@example.invalid")
        initial = {
            "pkg/__init__.py": "",
            "pkg/core.py": "VALUE = 1\n",
            "pkg/mid.py": "import pkg.core\n",
            "pkg/top.py": "import pkg.mid\n",
            "pkg/core_extra.py": "VALUE = 100\n",
            "pkg/unrelated.py": "import pkg.core_extra\n",
            "src/main.rs": "fn main() { println!(\"old\"); }\n",
        }
        for path, content in initial.items():
            self.write(path, content)
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "independent baseline")
        self.base = git(self.repo, "rev-parse", "HEAD")
        self.write("pkg/core.py", "VALUE = 2\n")
        self.write("src/main.rs", "fn main() { println!(\"new\"); }\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-qm", "change one Python dependency and an unsupported Rust file")
        self.head = git(self.repo, "rev-parse", "HEAD")
        self.db = self.root / "evidence.sqlite"
        self.report_path = self.root / "impact.json"

    def write(self, path: str, content: str) -> None:
        destination = self.repo / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content, encoding="utf-8")

    def run_impact(self) -> dict:
        process = command(
            BIN, "impact", self.repo, "--base", self.base, "--head", self.head,
            "--roots", "pkg", "--db", self.db, "--json",
        )
        return json.loads(process.stdout)

    def evidence_matches_git(self, source: dict, commit: str,
                             tree: str, files: dict[str, tuple[str, str]]) -> None:
        self.assertEqual(source["source_commit"], commit)
        self.assertEqual(source["source_tree"], tree)
        self.assertIn(source["path"], files)
        self.assertEqual((source["blob"], source["mode"]), files[source["path"]])

    def test_end_to_end_impact_matches_independent_git_and_ast(self) -> None:
        base_tree, before = git_inventory(self.repo, self.base)
        head_tree, after = git_inventory(self.repo, self.head)
        changed = {p for p in before.keys() | after.keys() if before.get(p) != after.get(p)}
        self.assertEqual(changed, {"pkg/core.py", "src/main.rs"})
        changes = json.loads(command(
            BIN, "changes", self.repo, "--base", self.base, "--head", self.head,
        ).stdout)
        self.assertEqual(
            {x["after"]["path"] if x["after"] else x["before"]["path"]
             for x in changes["changes"]}, changed,
        )

        expected: dict[str, set[str]] = {}
        native_edges: dict[str, set[tuple[str, str]]] = {}
        trees: dict[str, str] = {}
        inventories: dict[str, dict[str, tuple[str, str]]] = {}
        for revision, tree, inventory in (
            (self.base, base_tree, before), (self.head, head_tree, after)
        ):
            affected, edges, oracle_tree, files = oracle_paths(
                self.repo, revision, changed & inventory.keys(),
            )
            self.assertEqual(oracle_tree, tree)
            expected[revision] = affected
            native_edges[revision] = edges
            trees[revision] = tree
            inventories[revision] = files
        # A fixed independent expected outcome protects against a shared oracle bug.
        self.assertEqual(expected[self.base], {"pkg/mid.py", "pkg/top.py"})
        self.assertEqual(expected[self.head], {"pkg/mid.py", "pkg/top.py"})

        report = self.run_impact()
        self.assertEqual(report, self.run_impact(), "reports should be deterministic")
        self.assertEqual(report["schema"], "excession.impact.v1")
        self.assertEqual((report["base_commit"], report["head_commit"]),
                         (self.base, self.head))
        self.assertFalse(report["safe_to_skip_unlisted_tests"])
        self.assertEqual(report["coverage"]["status"], "PARTIAL")
        self.assertTrue(
            any(x["reason"] == "UNSUPPORTED_LANGUAGE" and x.get("path") == "src/main.rs"
                for x in report["unknown"]),
            "Rust changes must be explicit unknowns, not silently clean",
        )

        actual: dict[str, set[str]] = {self.base: set(), self.head: set()}
        self.assertTrue(report["effects"], "fixture should produce candidate impacts")
        for effect in report["effects"]:
            self.assertEqual(effect["assessment"], "CANDIDATE_PATH")
            entity = effect["entity"]
            rev = entity["source_commit"]
            self.assertIn(rev, actual)
            self.evidence_matches_git(entity, rev, trees[rev], inventories[rev])
            actual[rev].add(entity["path"])
            hops = effect["hops"]
            self.assertGreater(len(hops), 0)
            self.assertIn(hops[-1]["target"]["path"], changed)
            for hop in hops:
                self.assertEqual(hop["relation"], "STATIC_IMPORT")
                source, target = hop["source"], hop["target"]
                self.evidence_matches_git(source, rev, trees[rev], inventories[rev])
                self.evidence_matches_git(target, rev, trees[rev], inventories[rev])
                self.assertIn((source["path"], target["path"]), native_edges[rev])
                self.assertTrue(hop["evidence"])
                self.assertTrue(all(
                    e["assessment"] == "CANDIDATE" and e["receipt_id"]
                    for e in hop["evidence"]
                ))
        self.assertEqual(actual, expected)
        self.assertNotIn("pkg/unrelated.py", actual[self.head])
        self.assertEqual(git(self.repo, "status", "--porcelain"), "")

        self.report_path.write_text(json.dumps(report), encoding="utf-8")
        claim = report["effects"][0]["claim_id"]
        explanation = json.loads(command(
            BIN, "explain", self.repo, "--report", self.report_path, "--id", claim,
            "--db", self.db, "--roots", "pkg", "--json",
        ).stdout)
        self.assertEqual(explanation["claim_id"], claim)
        self.assertEqual(explanation["steps"], report["effects"][0]["hops"])
        audit = json.loads(command(
            BIN, "explain", self.repo, "--report", self.report_path, "--id", claim,
            "--db", self.db, "--roots", "pkg", "--audit",
        ).stdout)
        self.assertEqual(audit["replay"], "PASS")
        self.assertEqual(audit["semantic_correctness"], "NOT_EVALUATED")

    def test_explanation_rejects_tampered_report(self) -> None:
        report = self.run_impact()
        self.assertTrue(report["effects"])
        claim = report["effects"][0]["claim_id"]
        report["effects"][0]["hops"][0]["target"]["blob"] = "f" * 40
        self.report_path.write_text(json.dumps(report), encoding="utf-8")
        proc = command(
            BIN, "explain", self.repo, "--report", self.report_path, "--id", claim,
            "--db", self.db, "--roots", "pkg", "--json", check=False,
        )
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("REFUSED", proc.stderr.upper())


if __name__ == "__main__":
    unittest.main()
