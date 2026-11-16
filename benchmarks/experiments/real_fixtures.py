#!/usr/bin/env python3
"""Cross-repository, source-bound projection checks on immutable Git fixtures.

This is a verification/latency fixture suite, not a semantic-dependency oracle.
Every source binding is compared with Git independently of the SQLite index.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sqlite3
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "core/fixtures/h11-crossrepo.bundle"
ROLES = (
    "focus", "direct-dependencies", "transitive-dependencies",
    "direct-dependents", "transitive-dependents", "indexed-test",
)
GRAPH_ROLES = {
    "focus": "focus",
    "direct-dependencies": "dependencies_direct",
    "direct-dependents": "dependents_direct",
    "indexed-test": "tests",
}


def call(args: list[str], *, timeout: int = 120, check: bool = True) -> tuple[subprocess.CompletedProcess[bytes], float]:
    start = time.perf_counter_ns()
    proc = subprocess.run(args, capture_output=True, timeout=timeout, check=False)
    elapsed_ms = (time.perf_counter_ns() - start) / 1_000_000
    if check and proc.returncode:
        raise RuntimeError(
            f"{args[:6]!r} exited {proc.returncode}: "
            f"{proc.stderr.decode('utf-8', 'replace')[:1200]}"
        )
    return proc, round(elapsed_ms, 3)


def output(args: list[str], timeout: int = 120) -> bytes:
    return call(args, timeout=timeout)[0].stdout


def git(repo: Path, *args: str) -> str:
    return output(["git", "-C", str(repo), *args]).decode().strip()


def canonical_sha256(data: dict) -> str:
    return hashlib.sha256(
        json.dumps(data, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    ).hexdigest()


def git_blobs(repo: Path, commit: str) -> dict[str, tuple[str, str]]:
    raw = output(["git", "-C", str(repo), "ls-tree", "-r", "-z", commit])
    entries = {}
    for item in raw.split(b"\0"):
        if not item:
            continue
        header, path = item.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split()
        if kind == "blob":
            entries[path.decode("utf-8")] = (mode, oid)
    return entries


SYNTHETIC_IMPORTS = {
    "src/pkg/__init__.py": "# package initializer\n",
    "src/pkg/first.py": (
        "from pkg import second\n"
        "from . import third\n"
        "import pkg.third\n"
    ),
    "src/pkg/second.py": "VALUE = 2\n",
    "src/pkg/third.py": "VALUE = 3\n",
    "lib/main.js": (
        "import util from './util.cjs';\n"
        "import {value} from './sub/index.ts';\n"
        "export {other} from './alt/index.cjs';\n"
        "const a = require('./alt');\n"
        "const b = import('./sub');\n"
    ),
    "lib/util.cjs": "module.exports = 1;\n",
    "lib/sub/index.ts": "export const value = 2;\n",
    "lib/alt/index.cjs": "exports.other = 3;\n",
    "README.md": "An intentionally small import-resolution adversary.\n",
}


def checkout(fixture: dict, destination: Path) -> tuple[str, str]:
    if fixture["source"] == "synthetic":
        destination.mkdir()
        call(["git", "init", "-q", str(destination)])
        for path, contents in SYNTHETIC_IMPORTS.items():
            target = destination / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(contents)
        call(["git", "-C", str(destination), "add", "."])
        call(["git", "-C", str(destination), "-c", "user.name=Excession fixture",
              "-c", "user.email=fixture@example.invalid", "commit", "-qm",
              "Pinned synthetic import cases"])
    elif fixture["source"] == "bundle":
        call(["git", "clone", "-q", str(BUNDLE), str(destination)])
    elif fixture["source"] == "github":
        repo = fixture["repository"]
        if not all(part and part.replace("-", "").replace("_", "").replace(".", "").isalnum()
                   for part in repo.split("/")) or len(repo.split("/")) != 2:
            raise ValueError(f"invalid upstream repository: {repo!r}")
        commit = fixture["commit"]
        if len(commit) != 40 or any(ch not in "0123456789abcdef" for ch in commit):
            raise ValueError("fixtures must pin lowercase SHA-1 commits")
        destination.mkdir()
        call(["git", "init", "-q", str(destination)])
        call(["git", "-C", str(destination), "fetch", "-q", "--depth=1",
              f"https://github.com/{repo}.git", commit], timeout=240)
        call(["git", "-C", str(destination), "checkout", "-q", "--detach", "FETCH_HEAD"])
    else:
        raise ValueError(f"unsupported fixture source {fixture['source']!r}")
    commit = git(destination, "rev-parse", "HEAD")
    if fixture.get("commit") and commit != fixture["commit"]:
        raise ValueError("pinned upstream commit did not match checkout")
    if git(destination, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("fixture checkout is not clean")
    return commit, git(destination, "rev-parse", "HEAD^{tree}")


def verify_seed(binary: Path, db: Path, repo: Path, name: str,
                commit: str, tree: str, seed: str,
                source: dict[str, tuple[str, str]], repetitions: int) -> dict:
    fixed = ["--db", str(db), "--name", name, "--rev", commit, "--seed", seed]
    manifest = json.loads(output([str(binary), "view", "deps", *fixed]))
    payload = {key: value for key, value in manifest.items() if key != "view_id"}
    if manifest["view_id"] != canonical_sha256(payload):
        raise ValueError("view ID differs from independently canonicalized payload")
    if manifest["source_commit"] != commit or manifest["source_tree"] != tree:
        raise ValueError("view selected another source revision")
    if manifest["focus"] != seed or not manifest["entries"]:
        raise ValueError("view has wrong focus or no entries")
    virtual = [e["virtual_path"] for e in manifest["entries"]]
    if virtual != sorted(set(virtual)):
        raise ValueError("view paths are not strictly canonical")
    for entry in manifest["entries"]:
        actual = source.get(entry["source_path"])
        if actual != (entry["mode"], entry["blob"]):
            raise ValueError(f"view entry does not match independent Git tree: {entry['source_path']}")

    profile = json.loads(output([str(binary), "view", "profile-deps", *fixed]))
    if (profile["view_id"] != manifest["view_id"]
            or profile["source_commit"] != commit
            or profile["source_tree"] != tree
            or profile["entry_count"] != len(manifest["entries"])):
        raise ValueError("profiled query changed full view identity")
    phases = [item["phase"] for item in profile["phases"]]
    if "reconstruct_index_tree" not in phases or "validate_source_bindings" not in phases:
        raise ValueError("source authentication phases are absent")
    if sum(item["elapsed_us"] for item in profile["phases"]) > profile["total_us"]:
        raise ValueError("profile phase durations exceed total")

    by_role = {}
    timings = {}
    for role in ROLES:
        expected = sorted({e["source_path"] for e in manifest["entries"] if e["role"] == role})
        observed_ms = []
        for _ in range(repetitions):
            proc, elapsed = call([str(binary), "view", "paths", *fixed, "--role", role])
            result = json.loads(proc.stdout)
            if (result["source_paths"] != expected
                    or result["view_id"] != manifest["view_id"]
                    or result["source_commit"] != commit
                    or result["source_tree"] != tree):
                raise ValueError(f"compact view mismatch: {name}/{seed}/{role}")
            observed_ms.append(elapsed)
        if role in GRAPH_ROLES:
            graph = json.loads(output([str(binary), "view", "graph", *fixed,
                                       "--kind", GRAPH_ROLES[role]]))
            if graph["source_paths"] != expected or graph["source_commit"] != commit:
                raise ValueError(f"direct graph parity mismatch: {name}/{seed}/{role}")
        by_role[role] = len(expected)
        timings[role] = round(statistics.median(observed_ms), 3)
    return {
        "seed": seed, "view_id": manifest["view_id"],
        "entry_count": len(manifest["entries"]),
        "source_paths": sorted({e["source_path"] for e in manifest["entries"]}),
        "role_path_counts": by_role, "median_paths_wall_ms": timings,
        "profile_total_us": profile["total_us"],
        "phase_us": {x["phase"]: x["elapsed_us"] for x in profile["phases"]},
    }


def expected_refusal(binary: Path, damaged_db: Path, name: str, commit: str, seed: str) -> str:
    proc, _ = call([str(binary), "view", "paths", "--db", str(damaged_db),
                    "--name", name, "--rev", commit, "--seed", seed, "--role", "focus"],
                   check=False)
    if proc.returncode == 0:
        raise ValueError("tampered SQLite index was silently accepted")
    return proc.stderr.decode("utf-8", "replace")[:300]


def tamper_checks(binary: Path, db: Path, work: Path, name: str,
                  commit: str, seed: str, source_paths: set[str]) -> dict:
    with sqlite3.connect(db) as conn:
        modules = conn.execute("SELECT name,blob_oid FROM module ORDER BY name").fetchall()
        original = next(blob for module, blob in modules if module == seed)
        substitute = next((blob for module, blob in modules if blob != original), None)
        entries = conn.execute(
            "SELECT path,mode FROM tree_entry WHERE kind='blob' AND mode IN ('100644','100755') "
            "ORDER BY path"
        ).fetchall()
    if substitute is None:
        raise ValueError("fixture has no alternative Git blob to tamper module binding")
    outside = next(((path, mode) for path, mode in entries if path not in source_paths), None)
    if outside is None:
        raise ValueError("fixture has no non-projected source to test full-tree authentication")

    bad_module = work / f"{name}-bad-module.sqlite"
    shutil.copyfile(db, bad_module)
    with sqlite3.connect(bad_module) as conn:
        conn.execute("UPDATE module SET blob_oid=? WHERE name=?", (substitute, seed))
        if conn.total_changes != 1:
            raise ValueError("module tampering did not change exactly one row")
    module_error = expected_refusal(binary, bad_module, name, commit, seed)

    path, mode = outside
    bad_tree = work / f"{name}-bad-tree.sqlite"
    shutil.copyfile(db, bad_tree)
    with sqlite3.connect(bad_tree) as conn:
        conn.execute("UPDATE tree_entry SET mode=? WHERE path=?",
                     ("100755" if mode == "100644" else "100644", path))
        if conn.total_changes != 1:
            raise ValueError("tree tampering did not change exactly one row")
    tree_error = expected_refusal(binary, bad_tree, name, commit, seed)
    return {
        "module_blob_mismatch": {"refused": True, "diagnostic": module_error},
        "unrelated_tree_mode_mismatch": {
            "refused": True, "path": path, "diagnostic": tree_error,
        },
    }


def dependency_mutation_checks(
    db: Path, repo: Path, commit: str, source: dict[str, tuple[str, str]],
    work: Path, fixture_name: str,
) -> dict:
    """Prove the source oracle rejects omissions and invented SQL relations.

    Full view and direct graph would agree when both use a damaged dependency
    table. These checks must fail independently of their shared index.
    """
    from dependency_oracle import verify_dependencies

    with sqlite3.connect(db) as con:
        edges = con.execute(
            "SELECT repo_id,commit_oid,source_name,target_name,line,syntax "
            "FROM dependency ORDER BY source_name,target_name,line,syntax"
        ).fetchall()
        modules = [r[0] for r in con.execute("SELECT name FROM module ORDER BY name")]
        repo_id = con.execute("SELECT id FROM repository").fetchone()[0]
    if not edges or len(modules) < 2:
        raise ValueError("dependency corruption fixture needs real edges and multiple modules")

    missing_db = work / f"{fixture_name}-missing-dependency.sqlite"
    shutil.copyfile(db, missing_db)
    victim = edges[0]
    with sqlite3.connect(missing_db) as con:
        con.execute(
            "DELETE FROM dependency WHERE repo_id=? AND commit_oid=? "
            "AND source_name=? AND target_name=? AND line=? AND syntax=?", victim,
        )
        if con.total_changes != 1:
            raise ValueError("could not remove precisely one dependency")
    missing = verify_dependencies(missing_db, repo, commit, source)
    if missing["counts"]["missing_edges"] != 1 or missing["counts"]["unexpected_edges"] != 0:
        raise ValueError("independent oracle failed to detect one omitted source edge")

    recorded = {(r[2], r[3], r[4], r[5]) for r in edges}
    invented = next(
        ((a, b, 1, "import") for a in modules for b in modules
         if a != b and (a, b, 1, "import") not in recorded),
        None,
    )
    if invented is None:
        raise ValueError("no valid nonexisting module edge for invention test")
    extra_db = work / f"{fixture_name}-invented-dependency.sqlite"
    shutil.copyfile(db, extra_db)
    with sqlite3.connect(extra_db) as con:
        con.execute(
            "INSERT INTO dependency(repo_id,commit_oid,source_name,target_name,line,syntax)"
            " VALUES(?,?,?,?,?,?)", (repo_id, commit, *invented),
        )
        if con.total_changes != 1:
            raise ValueError("could not insert precisely one false dependency")
    extra = verify_dependencies(extra_db, repo, commit, source)
    if extra["counts"]["unexpected_edges"] != 1 or extra["counts"]["missing_edges"] != 0:
        raise ValueError("independent oracle failed to detect an invented source edge")
    return {"removed_edge_detected": True, "invented_edge_detected": True}



def choose_distinct_impact_edge(
    expected: set[tuple[str, str, int, str]],
    path_by_module: dict[str, str],
) -> tuple[tuple[str, str, int, str], int]:
    """Choose a deterministic observable dependent, not a package self-relation.

    The source inventory may contain a package-to-itself candidate (for
    `from . import sibling`), but changing that package does not make it
    a *distinct* impacted source. Do not silently exclude those self edges
    from the source inventory denominator: report their count separately.
    """
    distinct = [edge for edge in sorted(expected)
                if path_by_module[edge[0]] != path_by_module[edge[1]]]
    if not distinct:
        raise ValueError("fixture has no independently observed distinct-source import")
    return distinct[0], len(expected) - len(distinct)


def verify_product_workflow(
    binary: Path, repo: Path, work: Path, fixture: dict, commit: str,
    tree: str, objects: dict[str, tuple[str, str]],
    source_replay: bool = False,
) -> dict:
    """Held-out Git-source oracle for the public CLI on a pinned upstream repo.

    The expected static relationships are reconstructed from Git blobs by the
    existing independent AST/TypeScript oracle, not from Excession SQL rows.
    A local follow-on commit changes a true imported source and tests the
    explanation against the original upstream revision.
    """
    from dependency_oracle import (
        javascript_edges, module_name, python_edges,
    )

    prefixes = tuple(root.strip("/") + "/" for root in fixture["roots"].split(","))
    paths = sorted(
        path for path, (mode, _) in objects.items()
        if mode in ("100644", "100755")
        and path.startswith(prefixes)
        and path.endswith((".py", ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"))
        and not any(part in ("node_modules", ".venv") for part in path.split("/"))
    )
    path_by_module = {module_name(path): path for path in paths}
    if len(path_by_module) != len(paths):
        raise ValueError("independent source inventory has duplicate module names")
    sources = {
        path: output(["git", "-C", str(repo), "show", f"{commit}:{path}"]).decode("utf-8")
        for path in paths
    }
    py, py_unresolved = python_edges(sources, path_by_module)
    js, js_unresolved = javascript_edges(sources, path_by_module)
    expected = py | js
    expected_paths = {
        (path_by_module[a], path_by_module[b])
        for a, b, _line, _syntax in expected
    }
    if not expected:
        raise ValueError("product acceptance requires independent source dependencies")
    product_db = work / f"{fixture['name']}-product.sqlite"
    args = [
        str(binary), "analyze", str(repo),
        "--rev", commit, "--roots", fixture["roots"],
        "--db", str(product_db), "--json",
    ]
    proc, analyze_ms = call(args, timeout=240)
    report = json.loads(proc.stdout)
    if (report["schema"] != "excession.analysis.v1"
            or report["source_commit"] != commit
            or report["source_tree"] != tree
            or report["coverage"]["status"] != "PARTIAL"):
        raise ValueError("product analyze revision or coverage mismatch")
    observed_paths = set()
    for relation in report["relationships"]:
        left, right = relation["source"], relation["target"]
        for node in (left, right):
            if (node["source_commit"] != commit
                    or node["source_tree"] != tree
                    or objects.get(node["path"]) != (node["mode"], node["blob"])):
                raise ValueError("public analysis relation does not match independent Git objects")
        if (relation["kind"] != "STATIC_IMPORT"
                or not relation["evidence"]
                or any(e["assessment"] != "CANDIDATE" for e in relation["evidence"])):
            raise ValueError("candidate source relation was promoted to proof")
        observed_paths.add((left["path"], right["path"]))
    if observed_paths != expected_paths:
        missing = sorted(expected_paths - observed_paths)
        extra = sorted(observed_paths - expected_paths)
        raise ValueError(
            f"independent product import oracle mismatch: missing={missing[:5]}, extra={extra[:5]}"
        )

    # Choose an independently observed edge, not an Excession-selected impact.
    (source_module, target_module, _line, _kind), self_edges = (
        choose_distinct_impact_edge(expected, path_by_module)
    )
    source_path = path_by_module[source_module]
    target_path = path_by_module[target_module]
    comment = "# Excession pinned fixture change\n" if target_path.endswith(".py") \
        else "// Excession pinned fixture change\n"
    target = repo / target_path
    with target.open("a", encoding="utf-8") as stream:
        stream.write("\n" + comment)
    call(["git", "-C", str(repo), "add", "--", target_path])
    call([
        "git", "-C", str(repo),
        "-c", "user.name=Excession acceptance",
        "-c", "user.email=acceptance@example.invalid",
        "commit", "-qm", "Independent imported-source change",
    ])
    head = git(repo, "rev-parse", "HEAD")
    if head == commit:
        raise ValueError("follow-on fixture mutation did not produce a new revision")
    saved = work / f"{fixture['name']}-impact.json"
    proc, impact_ms = call([
        str(binary), "impact", str(repo), "--base", commit, "--head", head,
        "--roots", fixture["roots"], "--db", str(product_db), "--json",
        "--output", str(saved),
    ], timeout=240)
    impact_bytes = len(proc.stdout)
    impact = json.loads(proc.stdout)
    if json.loads(saved.read_text(encoding="utf-8")) != impact:
        raise ValueError("the CLI saved a different report from its JSON output")
    if (impact["schema"] != "excession.impact.v3"
            or "effects" in impact
            or "path_effects_status" in impact
            or impact["base_commit"] != commit
            or impact["head_commit"] != head
            or impact["safe_to_skip_unlisted_tests"] is not False
            or impact["coverage"]["status"] != "PARTIAL"):
        raise ValueError("source-only impact identity or coverage contract mismatch")
    candidate = next((
        witness for witness in impact["candidate_source_impacts"]
        if witness["entity"]["path"] == source_path
        and witness["assessment"] == "CANDIDATE_PATH"
        and witness["next_hop"]["target"]["path"] == target_path
        and witness["next_hop"]["evidence"]
    ), None)
    if candidate is None:
        diagnostic = {
            "schema": "excession.independent-impact-mismatch.v2",
            "fixture": fixture["name"],
            "source_commit": commit,
            "mutation_commit": head,
            "importing_source": source_path,
            "imported_source": target_path,
            "import_line": _line,
            "import_kind": _kind,
            "independent_expected_paths": len(expected_paths),
            "package_self_edges_excluded_only_from_impact_witness": self_edges,
            "analyze_observed_paths": len(observed_paths),
            "impact_report_schema": impact.get("schema"),
            "impact_coverage": impact.get("coverage"),
            "source_witness_count": len(impact["candidate_source_impacts"]),
            "matching_source_witnesses": [
                {"assessment": witness.get("assessment"),
                 "rank": witness.get("distance_to_change"),
                 "next_target": witness.get("next_hop", {}).get("target", {}).get("path")}
                for witness in impact["candidate_source_impacts"]
                if witness.get("entity", {}).get("path") == source_path
            ][:100],
        }
        (work / f"{fixture['name']}-impact-diagnostic.json").write_text(
            json.dumps(diagnostic, sort_keys=True, indent=2) + "\n", encoding="utf-8"
        )
        raise ValueError(
            "independent imported-source change had no ranked dependent: "
            f"{source_path} -> {target_path} ({_kind}, line {_line}); "
            f"sources={len(impact['candidate_source_impacts'])}; see preserved diagnostic"
        )
    selected_commit = candidate["entity"]["source_commit"]
    proc, explain_ms = call([
        str(binary), "explain", str(repo), "--report", str(saved),
        "--source", source_path, "--source-commit", selected_commit,
        "--db", str(product_db), "--roots", fixture["roots"], "--json",
    ], timeout=240)
    explanation = json.loads(proc.stdout)
    source_witness_id = canonical_sha256(candidate)
    if (explanation["schema"] != "excession.explanation.v1"
            or explanation["selection_kind"] != "DISTINCT_SOURCE_WITNESS"
            or explanation["claim_id"] != source_witness_id
            or explanation["source_commit"] != selected_commit
            or not explanation["steps"]
            or explanation["steps"][0]["source"]["path"] != source_path
            or explanation["steps"][0]["target"]["path"] != target_path):
        raise ValueError("source-bound impact explanation does not match the independent edge")
    proc, audit_ms = call([
        str(binary), "explain", str(repo), "--report", str(saved),
        "--source", source_path, "--source-commit", selected_commit,
        "--db", str(product_db), "--roots", fixture["roots"], "--audit",
    ], timeout=240)
    audit = json.loads(proc.stdout)
    if (audit["git_authentication"] != "PASS"
            or audit["replay"] != "PASS"
            or audit["semantic_correctness"] != "NOT_EVALUATED"):
        raise ValueError("source replay promoted an observation to semantic correctness")
    # Optional manually invoked source-batch acceptance. Each selected witness
    # is reconstructed against one fully authenticated source-only report.
    source_replay_results = None
    if source_replay:
        batch_out, batch_ms = call([
            str(binary), "explain", str(repo), "--report", str(saved),
            "--all-sources", "--db", str(product_db),
            "--roots", fixture["roots"], "--json",
        ], timeout=240)
        batch = json.loads(batch_out.stdout)
        witnesses = impact["candidate_source_impacts"]
        expected_witnesses = {
            (w["entity"]["source_commit"], w["entity"]["path"])
            for w in witnesses
        }
        actual_witnesses = {
            (e["source_commit"], e["steps"][0]["source"]["path"])
            for e in batch["explanations"]
        }
        if (batch["schema"] != "excession.source-explanations.v1"
                or batch["replayed_impact_report_id"] != impact["report_id"]
                or batch["source_inventory_complete"]
                   != impact["candidate_source_enumeration_complete"]
                or batch["semantic_correctness"] != "NOT_EVALUATED"
                or batch["count"] != len(witnesses)
                or expected_witnesses != actual_witnesses):
            raise ValueError("batched distinct-source explanations lost Git-bound inventory")
        expected_ranks = {
            (w["entity"]["source_commit"], w["entity"]["path"]):
            w["distance_to_change"] for w in witnesses
        }
        for item in batch["explanations"]:
            edges = item["steps"]
            key = (item["source_commit"], edges[0]["source"]["path"])
            if len(edges) != expected_ranks[key]:
                raise ValueError("batched source path failed rank certificate")
            if any(
                hop["source"]["source_commit"] != item["source_commit"]
                or hop["target"]["source_commit"] != item["source_commit"]
                or not hop["evidence"]
                or (i and edges[i-1]["target"] != hop["source"])
                for i, hop in enumerate(edges)
            ):
                raise ValueError("batched source path has discontinuous or missing Git evidence")

        selected_commit = candidate["entity"]["source_commit"]
        selected, selected_ms = call([
            str(binary), "explain", str(repo), "--report", str(saved),
            "--source", source_path, "--source-commit", selected_commit,
            "--db", str(product_db), "--roots", fixture["roots"], "--json",
        ], timeout=240)
        selected_report = json.loads(selected.stdout)
        if (selected_report["selection_kind"] != "DISTINCT_SOURCE_WITNESS"
                or selected_report["source_commit"] != selected_commit
                or selected_report["steps"][0]["source"]["path"] != source_path
                or not any(
                    e["claim_id"] == selected_report["claim_id"]
                    for e in batch["explanations"]
                )):
            raise ValueError("selected source explanation disagrees with batched replay")

        source_audit, source_audit_ms = call([
            str(binary), "explain", str(repo), "--report", str(saved),
            "--source", source_path, "--source-commit", selected_commit,
            "--db", str(product_db), "--roots", fixture["roots"], "--audit",
        ], timeout=240)
        audited = json.loads(source_audit.stdout)
        if (audited["git_authentication"] != "PASS"
                or audited["replay"] != "PASS"
                or audited["semantic_correctness"] != "NOT_EVALUATED"):
            raise ValueError("source witness audit promoted static reachability into proof")
        source_replay_results = {
            "batch_source_count": batch["count"],
            "batch_source_complete": batch["source_inventory_complete"],
            "batched_source_explanation_wall_ms": batch_ms,
            "one_selected_source_wall_ms": selected_ms,
            "one_selected_source_audit_wall_ms": source_audit_ms,
            "selected_source": source_path,
            "selected_commit": selected_commit,
        }
    # Re-serialized JSON with a forged relation must fail the *native* replay.
    mutated = json.loads(json.dumps(impact))
    mutated["candidate_source_impacts"][0]["assessment"] = "OBSERVED"
    saved.write_text(json.dumps(mutated, sort_keys=True) + "\n", encoding="utf-8")
    failed, _ = call([
        str(binary), "explain", str(repo), "--report", str(saved),
        "--source", source_path, "--source-commit", selected_commit,
        "--db", str(product_db),
        "--roots", fixture["roots"], "--json",
    ], timeout=240, check=False)
    if failed.returncode == 0:
        raise ValueError("mutated source impact report was accepted")
    return {
        "upstream_commit": commit, "local_mutation_commit": head,
        "package_self_edges_excluded_only_from_impact_witness": self_edges,
        "source_count": len(paths),
        "expected_pair_count": len(expected_paths),
        "observed_pair_count": len(observed_paths),
        "not_proven_import_sites": len(py_unresolved) + len(js_unresolved),
        "changed_source": target_path,
        "independent_dependent": source_path,
        "source_witness_count": len(impact["candidate_source_impacts"]),
        "source_witness_id": source_witness_id,
        "impact_report_bytes": impact_bytes,
        "reported_coverage": impact["coverage"]["status"],
        "semantic_correctness": audit["semantic_correctness"],
        "forged_report_refused": True,
        "source_replay_acceptance": source_replay_results,
        "analyze_wall_ms": analyze_ms,
        "impact_wall_ms": impact_ms,
        "explain_wall_ms": explain_ms,
        "audit_wall_ms": audit_ms,
        "sqlite_bytes": product_db.stat().st_size,
    }


def inspect_fixture(binary: Path, fixture: dict, work: Path, repetitions: int,
                    product_fixtures: frozenset[str] = frozenset(("h11", "axios")),
                    source_replay: bool = False) -> dict:
    name = fixture["name"]
    repo = work / name
    commit, tree = checkout(fixture, repo)
    db = work / f"{name}.sqlite"
    start = time.perf_counter_ns()
    output([str(binary), "index", "--repo", str(repo), "--db", str(db),
            "--name", name, "--rev", commit, "--roots", fixture["roots"]], timeout=300)
    index_ms = round((time.perf_counter_ns() - start) / 1_000_000, 3)
    with sqlite3.connect(db) as conn:
        counts = {
            table: conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
            for table in ("tree_entry", "module", "dependency", "hazard")
        }
        available = {row[0] for row in conn.execute("SELECT name FROM module")}
    source = git_blobs(repo, commit)
    results = []
    for seed in fixture["seeds"]:
        if seed not in available:
            raise ValueError(f"declared seed {seed!r} was not indexed")
        results.append(verify_seed(binary, db, repo, name, commit, tree, seed,
                                   source, repetitions))
    viewed = {path for result in results for path in result["source_paths"]}
    refusals = tamper_checks(binary, db, work, name, commit, fixture["seeds"][0], viewed)
    from dependency_oracle import verify_dependencies
    dependency_audit = verify_dependencies(db, repo, commit, source)
    dependency_mutations = dependency_mutation_checks(db, repo, commit, source, work, name)
    # Explicitly selected product fixtures run the same independent source oracle.
    # Keep additional expensive end-to-end cases out of ordinary commit gates.
    product = (
        verify_product_workflow(
            binary, repo, work, fixture, commit, tree, source,
            source_replay=source_replay,
        )
        if name in product_fixtures else None
    )
    for result in results:
        del result["source_paths"]
    return {
        "name": name, "upstream": fixture.get("repository", "bundled h11"),
        "source_commit": commit, "source_tree": tree, "roots": fixture["roots"],
        "status": "passed" if dependency_audit["status"] == "matched" else "failed",
        "index_wall_ms": index_ms,
        "index_bytes": db.stat().st_size, "rows": counts,
        "git_blob_count": len(source), "seeds": results,
        "tamper_refusals": refusals, "dependency_audit": dependency_audit,
        "dependency_mutation_checks": dependency_mutations,
        "product_workflow": product,
    }


def select_fixtures(fixtures: list[dict], selected: list[str], products: list[str]) -> tuple[list[dict], frozenset[str]]:
    names = [f["name"] for f in fixtures]
    if len(names) != len(set(names)):
        raise ValueError("duplicate fixture names in manifest")
    if len(selected) != len(set(selected)) or len(products) != len(set(products)):
        raise ValueError("duplicate fixture selection")
    wanted = set(selected) if selected else set(names)
    unknown = sorted(wanted - set(names) | (set(products) - wanted))
    if unknown:
        raise ValueError("unrecognized or unselected product fixtures: " + ", ".join(unknown))
    # Preserve manifest order, not caller-selected or producer-derived order.
    return [f for f in fixtures if f["name"] in wanted], frozenset(products) if products else frozenset(("h11", "axios")) & wanted


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--fixture", action="append", default=[], help="Select a named pinned source fixture; repeatable")
    parser.add_argument("--product-fixture", action="append", default=[], help="Run full independent product workflow for this selected fixture")
    parser.add_argument("--source-replay", action="store_true",
                        help="Manually check and time source-witness batch vs existing replay")
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("repetitions must be positive")
    binary = args.bin.resolve(strict=True)
    fixtures_file = args.manifest.resolve(strict=True)
    fixtures = json.loads(fixtures_file.read_text())
    if fixtures["schema"] != "excession.real-projection-fixtures.v1":
        raise ValueError("unknown fixture declaration schema")
    selected, products = select_fixtures(fixtures["fixtures"], args.fixture, args.product_fixture)
    report = {
        "schema": "excession.real-projection-fixture-results.v2",
        "fixture_spec_sha256": hashlib.sha256(fixtures_file.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "repetitions": args.repetitions,
        "source_replay_enabled": args.source_replay,
        "selected_fixtures": [f["name"] for f in selected],
        "product_fixtures": sorted(products),
        "fixtures": [],
        "limitations": (
            "Source OIDs, source witness replay, and tamper refusal are checked. "
            "Pinned h11 and Axios also exercise exact-revision public analyze-impact-"
            "explain replay against an independent source-import oracle. "
            "Graph retrieval and view selection both consume the same semantic SQLite index; "
            "agreement alone does not validate dependency completeness. A separate Git-source "
            "AST/TypeScript import inventory checks statically resolvable indexed edges, but "
            "does not establish runtime or dynamically constructed dependencies. Git source tree and commit "
            "identity are authoritative. Timings are process wall time on one CI host."
        ),
    }
    # Create the receipt destination before any fixture runs so even a
    # failed product path can retain its native report and mismatch witness.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="excession-real-fixtures-") as tmp:
        work = Path(tmp)
        for fixture in selected:
            try:
                result = inspect_fixture(
                    binary, fixture, work, args.repetitions, products,
                    source_replay=args.source_replay,
                )
                report["fixtures"].append(result)
                audit = result["dependency_audit"]["counts"]
                print(f"{result['status'].upper()} {result['name']}: "
                      f"{audit['matched_edges']}/{audit['expected_in_scope_edges']} "
                      f"expected import edges, {audit['missing_edges']} missing, "
                      f"{audit['unexpected_edges']} unexpected; "
                      f"{audit['not_proven_import_sites']} unproven import sites; "
                      f"{len(result['seeds'])} seeds, 2 source integrity refusals, "
                      f"2 graph mutation detections", flush=True)
            except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired,
                    sqlite3.Error) as exc:
                fixture_name = fixture.get("name", "<unknown>")
                preserved = []
                for suffix in ("impact-diagnostic.json", "impact.json"):
                    candidate_receipt = work / f"{fixture_name}-{suffix}"
                    if candidate_receipt.is_file() and candidate_receipt.stat().st_size <= 2_000_000:
                        destination = args.out.parent / candidate_receipt.name
                        shutil.copyfile(candidate_receipt, destination)
                        preserved.append(destination.name)
                report["fixtures"].append({
                    "name": fixture_name,
                    "status": "failed", "error": str(exc)[:1600],
                    "preserved_evidence": preserved,
                })
                print(f"FAIL {fixture_name}: {exc}", flush=True)
    report["passed"] = sum(f["status"] == "passed" for f in report["fixtures"])
    report["failed"] = len(report["fixtures"]) - report["passed"]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Fixture summary: {report['passed']} passed, {report['failed']} failed", flush=True)
    if report["failed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
