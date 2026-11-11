#!/usr/bin/env python3
"""Reproducible, read-only Excession probe against a pinned Linux kernel checkout.

Keeps all databases, manifests, and measurements outside the Linux worktree.
Does not build or execute Linux kernel code.
"""
import argparse
import ast
import json
import sqlite3
import subprocess
import time
from pathlib import Path

LINUX_COMMIT = "adc218676eef25575469234709c2d87185ca223a"
SEED = "linux.tasks"
SOURCE = "scripts/gdb/linux/tasks.py"


def run(argv, *, binary=False):
    started = time.perf_counter()
    result = subprocess.run(argv, capture_output=True, check=True)
    return (result.stdout if binary else result.stdout.decode("utf-8"),
            round(time.perf_counter() - started, 3))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repo = args.repo.resolve()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = str(args.binary.resolve())

    commit, _ = run(["git", "-C", str(repo), "rev-parse", "HEAD"])
    commit = commit.strip()
    if commit != LINUX_COMMIT:
        raise RuntimeError(f"Linux source is not pinned: {commit}")
    dirty, _ = run(["git", "-C", str(repo), "status", "--porcelain"])
    if dirty:
        raise RuntimeError("Linux source checkout is dirty")

    db = out / "index.sqlite"
    indexed, index_seconds = run([
        binary, "index", "--repo", str(repo), "--db", str(db),
        "--name", "linux", "--rev", commit, "--roots", "scripts/gdb",
        "--import-base", "scripts/gdb"
    ])
    indexed = json.loads(indexed)
    if indexed["source_commit"] != commit or indexed["status"] != "GIT_TREE_INDEXED":
        raise RuntimeError("Index receipt did not bind the requested commit")

    projected, project_seconds = run([
        binary, "view", "deps", "--db", str(db), "--name", "linux",
        "--rev", commit, "--seed", SEED
    ])
    manifest = json.loads(projected)
    if manifest["source_commit"] != commit or manifest["source_tree"] != indexed["source_tree"]:
        raise RuntimeError("Projection has the wrong source identity")
    manifest_path = out / "tasks-view.json"
    manifest_path.write_text(projected, encoding="utf-8")
    _, replay_seconds = run([
        binary, "view", "replay", "deps", "--db", str(db),
        "--name", "linux", "--rev", commit, "--seed", SEED,
        "--manifest", str(manifest_path)
    ])
    focus = next(e for e in manifest["entries"] if e["role"] == "focus")
    if focus["source_path"] != SOURCE:
        raise RuntimeError("Unexpected focus file")
    virtual_bytes, cat_seconds = run([
        binary, "view", "cat", "--repo", str(repo),
        "--manifest", str(manifest_path), "--path", focus["virtual_path"]
    ], binary=True)
    original_bytes, git_show_seconds = run([
        "git", "-C", str(repo), "show", f"{commit}:{SOURCE}"
    ], binary=True)
    if virtual_bytes != original_bytes:
        raise RuntimeError("Excession virtual source bytes differ from Git")

    _, git_grep_seconds = run([
        "git", "-C", str(repo), "grep", "-n", "-F", "from linux import", "--", SOURCE
    ])
    expected = set()
    tree = ast.parse(original_bytes, filename=SOURCE)
    for item in ast.walk(tree):
        if isinstance(item, ast.ImportFrom) and item.module == "linux":
            expected.update(
                "linux." + alias.name
                for alias in item.names if alias.name != "*"
            )
    with sqlite3.connect(db) as connection:
        metrics = {
            table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
            for table in ("tree_entry", "module", "dependency", "symbol", "hazard")
        }
        observed = {
            row[0] for row in connection.execute(
                "SELECT DISTINCT target_name FROM dependency WHERE source_name=?",
                (SEED,)
            )
        }
        source_files = connection.execute(
            "SELECT COUNT(*) FROM tree_entry WHERE kind='blob'"
        ).fetchone()[0]
        c_files = connection.execute(
            "SELECT COUNT(*) FROM tree_entry WHERE kind='blob' AND path LIKE '%.c'"
        ).fetchone()[0]
    if expected - observed:
        raise AssertionError(f"Unresolved direct imports: {sorted(expected - observed)}")
    if len(manifest["entries"]) < 3:
        raise AssertionError("View did not include the known dependent modules")
    result = {
        "target": "torvalds/linux v6.12",
        "source_commit": commit,
        "source_tree": indexed["source_tree"],
        "focused_source": SOURCE,
        "index_roots": ["scripts/gdb"],
        "python_import_base": "scripts/gdb",
        "simpler_baselines_seconds": {
            "git_show_source_blob": git_show_seconds,
            "git_grep_import_statement": git_grep_seconds
        },
        "timings_seconds": {
            "index": index_seconds,
            "view_deps": project_seconds,
            "replay": replay_seconds,
            "view_cat": cat_seconds
        },
        "indexed_rows": metrics,
        "git_files": source_files,
        "c_files_without_c_semantic_adapter": c_files,
        "view_entries": len(manifest["entries"]),
        "view_id": manifest["view_id"],
        "bytes_equal_to_git": True,
        "replay_succeeded": True,
        "direct_linux_python_imports_expected": sorted(expected),
        "direct_dependencies_observed": sorted(observed),
        "known_imports_missing_from_index": sorted(expected - observed),
        "limitations": [
            "No C or Kbuild/Kconfig semantic relationships are indexed.",
            "Python imports are resolved relative to the explicit --import-base.",
            "Git grep and show timings are retrieval-only, not equivalent workloads.",
            "A matching Git object and view replay do not establish executable equivalence."
        ]
    }
    (out / "metrics.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
