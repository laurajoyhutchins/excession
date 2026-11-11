#!/usr/bin/env python3
"""Independent Git/SQLite replay verifier and paired corruption experiment.

This verifier does NOT import or invoke Excession to decide authenticity.
It independently reads Git objects, checks all indexed tree rows, rederives
the dependency projection, and checks its canonical manifest commitment.
It does not verify that dependency edges are true in the source program.
"""
from __future__ import annotations

import argparse
from collections import deque
import hashlib
import json
from pathlib import Path
import random
import resource
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from typing import Any

FORMAT = "excession.repository-view.v1"
FIELDS = {"schema", "kind", "source_commit", "source_tree", "focus", "entries", "view_id"}
ENTRY_FIELDS = {"virtual_path", "source_path", "blob", "mode", "role", "detail"}


class Refused(ValueError):
    pass


def require(condition: bool, error: str) -> None:
    if not condition:
        raise Refused(error)


def unique_object(pairs: list[tuple[str, Any]]) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def canonical(data: dict) -> bytes:
    return json.dumps(data, sort_keys=True, ensure_ascii=True,
                      separators=(",", ":"), allow_nan=False).encode("utf-8")


def signature(data: dict) -> str:
    return hashlib.sha256(canonical(data)).hexdigest()


def safe_path(path: str) -> bool:
    return (isinstance(path, str) and bool(path) and
            not path.startswith("/") and "\\" not in path and "\0" not in path and
            all(component not in ("", ".", "..") for component in path.split("/")))


def oid(value: str) -> bool:
    return (isinstance(value, str) and len(value) == 40 and
            all(c in "0123456789abcdef" for c in value))


def read_manifest(path: Path) -> dict:
    raw = path.read_bytes()
    require(len(raw) <= 5 * 1024 * 1024, "manifest exceeds size limit")
    try:
        doc = json.loads(raw, object_pairs_hook=unique_object)
    except (ValueError, UnicodeDecodeError) as exc:
        raise Refused("malformed manifest JSON") from exc
    require(isinstance(doc, dict) and set(doc) == FIELDS, "invalid manifest top-level fields")
    require(doc["schema"] == FORMAT and doc["kind"] == "dependencies", "unsupported manifest kind")
    require(oid(doc["source_commit"]) and oid(doc["source_tree"]), "invalid source oid")
    require(isinstance(doc["focus"], str) and bool(doc["focus"]), "missing focus")
    require(isinstance(doc["entries"], list) and 1 <= len(doc["entries"]) <= 5000,
            "bad entry count")
    paths = []
    for entry in doc["entries"]:
        require(isinstance(entry, dict) and set(entry) == ENTRY_FIELDS, "invalid entry keys")
        vpath, source = entry["virtual_path"], entry["source_path"]
        require(safe_path(vpath) and vpath.startswith("views/") and safe_path(source),
                "unsafe path")
        require(oid(entry["blob"]) and entry["mode"] in {"100644", "100755"},
                "invalid blob identity/mode")
        require(isinstance(entry["role"], str) and 0 < len(entry["role"]) <= 80,
                "invalid role")
        require(isinstance(entry["detail"], str) and len(entry["detail"]) <= 512,
                "invalid detail")
        paths.append(vpath)
    require(paths == sorted(set(paths)), "noncanonical or duplicate virtual paths")
    all_paths = set(paths)
    for path in paths:
        parts = path.split("/")
        require(not any("/".join(parts[:i]) in all_paths for i in range(1, len(parts))),
                "virtual directory/file overlap")
    require(isinstance(doc["view_id"], str) and len(doc["view_id"]) == 64,
            "invalid manifest commitment")
    require(signature({key: val for key, val in doc.items() if key != "view_id"}) ==
            doc["view_id"], "manifest commitment mismatch")
    return doc


def git(repo: Path, *argv: str) -> bytes:
    result = subprocess.run(["git", "-C", str(repo), *argv],
                            capture_output=True, timeout=90, check=False)
    require(result.returncode == 0,
            f"Git refused {argv[0]}: {result.stderr.decode(errors='replace')[:250]}")
    return result.stdout


def git_entries(repo: Path, commit: str) -> dict[str, tuple[str, str, str]]:
    raw = git(repo, "ls-tree", "-rtz", "--full-tree", commit)
    require(not raw or raw.endswith(b"\0"), "unterminated Git tree")
    result = {}
    for line in raw.split(b"\0"):
        if not line:
            continue
        try:
            header, suffix = line.split(b"\t", 1)
            mode, typ, sha = header.decode("ascii").split(" ")
            path = suffix.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as exc:
            raise Refused("invalid Git tree row") from exc
        require(safe_path(path) and oid(sha), "invalid Git tree identity")
        require(path not in result, "duplicate Git path")
        result[path] = (mode, typ, sha)
    return result


def bfs(seed: str, graph: dict[str, set[str]]) -> dict[str, int]:
    distances = {seed: 0}
    pending = deque([seed])
    while pending:
        source = pending.popleft()
        for target in sorted(graph.get(source, ())):
            if target not in distances:
                distances[target] = distances[source] + 1
                pending.append(target)
    return distances


def independently_replay(repo: Path, db: Path, name: str,
                         commit: str, seed: str, manifest_path: Path) -> dict:
    """Reconstruct the complete indexed query without Excession code."""
    doc = read_manifest(manifest_path)
    require(doc["source_commit"] == commit and doc["focus"] == seed,
            "manifest belongs to another query")
    require(oid(commit) and repo.is_dir(), "invalid input repository/commit")
    resolved = git(repo, "rev-parse", "--verify", f"{commit}^{{commit}}").decode().strip()
    require(resolved == commit, "commit mismatch")
    tree = git(repo, "rev-parse", "--verify", f"{commit}^{{tree}}").decode().strip()
    require(tree == doc["source_tree"], "Git root-tree mismatch")
    reference = git_entries(repo, commit)
    with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as connection:
        connection.execute("PRAGMA query_only=ON")
        repository = connection.execute(
            "SELECT id,git_dir FROM repository WHERE name=?", (name,)
        ).fetchall()
        require(len(repository) == 1 and Path(repository[0][1]).resolve() == repo.resolve(),
                "repository identity mismatch")
        repo_id = repository[0][0]
        rev = connection.execute(
            "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
            (repo_id, commit)
        ).fetchall()
        require(len(rev) == 1 and rev[0][0] == tree, "indexed revision tree mismatch")
        rows = connection.execute(
            "SELECT path,mode,kind,oid FROM tree_entry "
            "WHERE repo_id=? AND commit_oid=?", (repo_id, commit)
        ).fetchall()
        indexed_tree = {}
        for path, mode, typ, sha in rows:
            require(path not in indexed_tree, "duplicate SQL tree path")
            indexed_tree[path] = (mode, typ, sha)
        # Checking every row against an independently enumerated Git tree
        # includes unrelated paths; a selected-only path audit is weaker.
        require(indexed_tree == reference, "indexed tree differs from Git objects")
        records = connection.execute(
            "SELECT name,path,blob_oid,is_test FROM module "
            "WHERE repo_id=? AND commit_oid=?", (repo_id, commit)
        ).fetchall()
        modules = {}
        for mod, path, blob, test in records:
            require(mod not in modules and isinstance(mod, str), "duplicate module name")
            require(path in reference and reference[path][1] == "blob"
                    and reference[path][2] == blob and test in (0, 1),
                    "module binding differs from Git source")
            modules[mod] = (path, blob, reference[path][0], bool(test))
        require(seed in modules, "seed absent")
        relations = connection.execute(
            "SELECT source_name,target_name FROM dependency "
            "WHERE repo_id=? AND commit_oid=?", (repo_id, commit)
        ).fetchall()
        forward: dict[str, set[str]] = {}
        backward: dict[str, set[str]] = {}
        for from_, to in relations:
            require(from_ in modules and to in modules, "dangling dependency")
            forward.setdefault(from_, set()).add(to)
            backward.setdefault(to, set()).add(from_)
        fk = connection.execute("PRAGMA foreign_key_check").fetchall()
        require(not fk, "SQLite foreign-key violations")
    entries = []
    for direction, graph in (("dependencies", forward), ("dependents", backward)):
        for module, depth in bfs(seed, graph).items():
            if depth == 0:
                continue
            path, blob, mode, is_test = modules[module]
            tier = "direct" if depth == 1 else "transitive"
            group = "10-dependencies" if direction == "dependencies" else "20-dependents"
            entries.append({
                "virtual_path": f"views/{group}/{tier}/{path}",
                "source_path": path, "blob": blob, "mode": mode,
                "role": f"{tier}-{direction}",
                "detail": f"module={module};depth={depth}",
            })
            if is_test:
                entries.append({
                    "virtual_path": f"views/30-tests/{direction}/{path}",
                    "source_path": path, "blob": blob, "mode": mode,
                    "role": "indexed-test",
                    "detail": f"module={module};direction={direction}",
                })
    path, blob, mode, _ = modules[seed]
    entries.append({
        "virtual_path": f"views/00-focus/{path}",
        "source_path": path, "blob": blob, "mode": mode,
        "role": "focus", "detail": f"module={seed}",
    })
    payload = {
        "schema": FORMAT, "kind": "dependencies",
        "source_commit": commit, "source_tree": tree,
        "focus": seed,
        "entries": sorted(entries, key=lambda e: e["virtual_path"]),
    }
    require(doc == {**payload, "view_id": signature(payload)},
            "manifest is not complete deterministic graph query")
    return {
        "status": "QUERY_REPLAY_MATCHED",
        "view_id": doc["view_id"],
        "entries_replayed": len(entries),
    }


def invocation(argv: list[str]) -> tuple[subprocess.CompletedProcess[bytes], dict]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter_ns()
    proc = subprocess.run(argv, capture_output=True, check=False, timeout=90)
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    return proc, {
        "wall_ms": round((time.perf_counter_ns() - start) / 1e6, 3),
        "child_cpu_ms": round(1000 * (
            after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime), 3),
        "stdout_bytes": len(proc.stdout),
        "stdout_sha256": hashlib.sha256(proc.stdout).hexdigest(),
    }


def checked(argv: list[str]) -> bytes:
    proc, _ = invocation(argv)
    if proc.returncode:
        raise RuntimeError(f"{argv[:5]!r}: {proc.stderr.decode(errors='replace')[:400]}")
    return proc.stdout


def clone_sqlite(source: Path, destination: Path) -> None:
    with sqlite3.connect(source) as src, sqlite3.connect(destination) as dst:
        src.backup(dst)


def altered_manifest(doc: dict, path: Path, modification: str) -> None:
    obj = json.loads(json.dumps(doc))
    if modification == "different_blob":
        obj["entries"][0]["blob"] = "0" * 40
    elif modification == "missing_entry":
        require(len(obj["entries"]) > 1, "no entry to remove")
        obj["entries"].pop()
    elif modification == "wrong_role":
        obj["entries"][0]["role"] = "misidentified-role"
    elif modification == "stale_commit":
        obj["source_commit"] = "f" * 40
    elif modification == "invalid_commitment":
        obj["view_id"] = "0" * 64
    else:
        raise ValueError("unknown manifest mutation")
    if modification != "invalid_commitment":
        obj["view_id"] = signature({k: v for k, v in obj.items() if k != "view_id"})
    path.write_bytes(canonical(obj) + b"\n")


def mutation_database(db: Path, dest: Path, name: str, commit: str,
                      seed: str, mutation: str) -> None:
    clone_sqlite(db, dest)
    with sqlite3.connect(dest) as connection:
        connection.execute("PRAGMA foreign_keys=OFF")
        rid = connection.execute(
            "SELECT id FROM repository WHERE name=?", (name,)
        ).fetchone()[0]
        if mutation == "module_blob":
            others = connection.execute(
                "SELECT blob_oid FROM module WHERE repo_id=? AND commit_oid=? "
                "AND name<>? AND blob_oid<>(SELECT blob_oid FROM module "
                "WHERE repo_id=? AND commit_oid=? AND name=?) LIMIT 1",
                (rid, commit, seed, rid, commit, seed)
            ).fetchone()
            require(others is not None, "need alternate valid blob")
            result = connection.execute(
                "UPDATE module SET blob_oid=? WHERE repo_id=? AND commit_oid=? AND name=?",
                (others[0], rid, commit, seed))
        elif mutation == "tree_mode":
            result = connection.execute(
                "UPDATE tree_entry SET mode=CASE mode WHEN '100644' THEN '100755' "
                "ELSE '100644' END WHERE rowid IN "
                "(SELECT rowid FROM tree_entry WHERE repo_id=? AND commit_oid=? "
                "AND kind='blob' AND mode IN ('100644','100755') LIMIT 1)",
                (rid, commit))
        elif mutation == "removed_edge":
            chosen = connection.execute(
                "SELECT target_name FROM dependency WHERE repo_id=? AND commit_oid=? "
                "AND source_name=? LIMIT 1", (rid, commit, seed)
            ).fetchone()
            require(chosen is not None, "seed has no outgoing dependency")
            result = connection.execute(
                "DELETE FROM dependency WHERE repo_id=? AND commit_oid=? "
                "AND source_name=? AND target_name=?",
                (rid, commit, seed, chosen[0]))
        else:
            raise ValueError("unknown SQLite mutation")
        require(result.rowcount > 0, "corruption fixture made no change")
        connection.commit()


def benchmark(binary: Path, output: Path, repetitions: int) -> dict:
    if output.exists():
        raise ValueError("output directory already exists")
    output.mkdir(parents=True)
    fixture = Path(__file__).resolve().parents[2] / "core/fixtures/h11-crossrepo.bundle"
    source = output / "source"
    checked(["git", "clone", "-q", str(fixture), str(source)])
    commit = checked(["git", "-C", str(source), "rev-parse", "HEAD"]).decode().strip()
    name, seed = "h11", "h11._connection"
    db = output / "index.sqlite"
    checked([str(binary), "index", "--repo", str(source), "--db", str(db),
             "--name", name, "--rev", commit, "--roots", "h11,tests"])
    manifest = output / "valid.json"
    manifest.write_bytes(checked([
        str(binary), "view", "deps", "--db", str(db), "--name", name,
        "--rev", commit, "--seed", seed
    ]))
    pristine = read_manifest(manifest)
    cases = [{"name": "valid", "db": db, "manifest": manifest, "expect": True}]
    for modification in ("different_blob", "missing_entry", "wrong_role",
                         "stale_commit", "invalid_commitment"):
        altered = output / f"{modification}.json"
        altered_manifest(pristine, altered, modification)
        cases.append({"name": modification, "db": db,
                      "manifest": altered, "expect": False})
    for modification in ("module_blob", "tree_mode", "removed_edge"):
        altered = output / f"{modification}.sqlite"
        mutation_database(db, altered, name, commit, seed, modification)
        cases.append({"name": modification, "db": altered,
                      "manifest": manifest, "expect": False})
    trials = []
    ordered = [(case, repeat) for repeat in range(repetitions) for case in cases]
    random.Random(20261111).shuffle(ordered)
    for i, (case, repeat) in enumerate(ordered):
        for arm in (("independent", "excession") if i % 2 == 0
                    else ("excession", "independent")):
            if arm == "independent":
                argv = [sys.executable, str(Path(__file__).resolve()), "audit-one",
                        "--repo", str(source), "--db", str(case["db"]),
                        "--name", name, "--commit", commit, "--seed", seed,
                        "--manifest", str(case["manifest"])]
            else:
                argv = [str(binary), "view", "replay", "deps",
                        "--db", str(case["db"]), "--name", name, "--rev", commit,
                        "--seed", seed, "--manifest", str(case["manifest"])]
            proc, measurement = invocation(argv)
            accepted = proc.returncode == 0
            trials.append({
                "case": case["name"], "repeat": repeat, "arm": arm,
                "expected_accept": case["expect"], "accepted": accepted,
                "matches_expectation": accepted == case["expect"],
                "exit_code": proc.returncode, **measurement,
                "diagnostic": ("" if accepted else proc.stderr.decode(
                    "utf-8", errors="replace")[:300]),
            })
    arm_names = ("independent", "excession")
    summary = {
        "schema": "excession.equivalent-query-replay.v1",
        "source_commit": commit,
        "source_tree": checked(["git", "-C", str(source),
                               "rev-parse", "HEAD^{tree}"]).decode().strip(),
        "bundle_sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "repetitions": repetitions,
        "cases": {c["name"]: c["expect"] for c in cases},
        "trials_per_arm": len(ordered),
        "matching_expectation": {
            arm: sum(t["matches_expectation"] for t in trials if t["arm"] == arm)
            for arm in arm_names
        },
        "median_wall_ms": {
            arm: round(statistics.median(t["wall_ms"] for t in trials
                                         if t["arm"] == arm), 3) for arm in arm_names
        },
        "median_child_cpu_ms": {
            arm: round(statistics.median(t["child_cpu_ms"] for t in trials
                                         if t["arm"] == arm), 3) for arm in arm_names
        },
        "limitations": (
            "Equivalent full indexed dependency-query replay is measured, not "
            "semantic truth or runtime reachability. Both trust Git's object "
            "database; the Python reference independently re-enumerates every "
            "Git tree row. Process startup and language differences confound "
            "latency. Plausible false semantic edges in SQLite remain outside "
            "both replay guarantees."
        ),
    }
    with (output / "trials.jsonl").open("w") as f:
        for item in trials:
            f.write(json.dumps(item, sort_keys=True) + "\n")
    (output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    if any(not t["matches_expectation"] for t in trials):
        raise RuntimeError("auditors disagreed with preregistered mutation expectation")
    return summary


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    audit = sub.add_parser("audit-one", help="independent single manifest replay")
    for key in ("repo", "db", "name", "commit", "seed", "manifest"):
        audit.add_argument("--" + key, required=True)
    bench = sub.add_parser("benchmark", help="paired normal and corrupt Git/SQLite cases")
    bench.add_argument("--bin", required=True)
    bench.add_argument("--out", required=True)
    bench.add_argument("--repetitions", type=int, default=2)
    args = parser.parse_args()
    if args.mode == "audit-one":
        try:
            value = independently_replay(
                Path(args.repo).resolve(strict=True), Path(args.db).resolve(strict=True),
                args.name, args.commit, args.seed, Path(args.manifest).resolve(strict=True))
            print(json.dumps(value, sort_keys=True))
        except (Refused, OSError, ValueError, sqlite3.Error,
                subprocess.TimeoutExpired) as exc:
            print(f"REFUSED: {str(exc)[:500]}", file=sys.stderr)
            raise SystemExit(2) from None
    else:
        if args.repetitions < 1:
            parser.error("repetitions must be positive")
        benchmark(Path(args.bin).resolve(strict=True),
                  Path(args.out).resolve(), args.repetitions)


if __name__ == "__main__":
    main()
