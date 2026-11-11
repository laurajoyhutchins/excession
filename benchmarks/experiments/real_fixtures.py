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


def checkout(fixture: dict, destination: Path) -> tuple[str, str]:
    if fixture["source"] == "bundle":
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


def inspect_fixture(binary: Path, fixture: dict, work: Path, repetitions: int) -> dict:
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
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("repetitions must be positive")
    binary = args.bin.resolve(strict=True)
    fixtures_file = args.manifest.resolve(strict=True)
    fixtures = json.loads(fixtures_file.read_text())
    if fixtures["schema"] != "excession.real-projection-fixtures.v1":
        raise ValueError("unknown fixture declaration schema")
    report = {
        "schema": "excession.real-projection-fixture-results.v1",
        "fixture_spec_sha256": hashlib.sha256(fixtures_file.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "repetitions": args.repetitions,
        "fixtures": [],
        "limitations": (
            "Source OIDs, compact/full parity, and tamper refusal are checked. "
            "Graph retrieval and view selection both consume the same semantic SQLite index; "
            "agreement alone does not validate dependency completeness. A separate Git-source "
            "AST/TypeScript import inventory checks statically resolvable indexed edges, but "
            "does not establish runtime or dynamically constructed dependencies. Git source tree and commit "
            "identity are authoritative. Timings are process wall time on one CI host."
        ),
    }
    with tempfile.TemporaryDirectory(prefix="excession-real-fixtures-") as tmp:
        work = Path(tmp)
        for fixture in fixtures["fixtures"]:
            try:
                result = inspect_fixture(binary, fixture, work, args.repetitions)
                report["fixtures"].append(result)
                audit = result["dependency_audit"]["counts"]
                print(f"{result['status'].upper()} {result['name']}: "
                      f"{audit['matched_edges']}/{audit['expected_in_scope_edges']} "
                      f"expected import edges, {audit['missing_edges']} missing, "
                      f"{audit['unexpected_edges']} unexpected; "
                      f"{len(result['seeds'])} seeds and 2 tamper refusals", flush=True)
            except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired,
                    sqlite3.Error) as exc:
                report["fixtures"].append({
                    "name": fixture.get("name", "<unknown>"),
                    "status": "failed", "error": str(exc)[:1600],
                })
                print(f"FAIL {fixture.get('name')}: {exc}", flush=True)
    report["passed"] = sum(f["status"] == "passed" for f in report["fixtures"])
    report["failed"] = len(report["fixtures"]) - report["passed"]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Fixture summary: {report['passed']} passed, {report['failed']} failed", flush=True)
    if report["failed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
