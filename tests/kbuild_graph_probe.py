#!/usr/bin/env python3
"""Acceptance for one REAL Linux v6.12 Kbuild archive, not a fixture."""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess

LINUX = "adc218676eef25575469234709c2d87185ca223a"

def run(argv, *, okay=True):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=120)
    if okay and result.returncode:
        raise AssertionError(f"failed {argv!r}: {result.stderr[-4000:]}")
    return result

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--build", required=True, type=Path)
    ap.add_argument("--binary", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()
    repo, build = args.repo.resolve(), args.build.resolve()
    target = "init/built-in.a"
    assert run(["git", "-C", str(repo), "rev-parse", "HEAD"]).stdout.strip() == LINUX
    assert (build / target).is_file(), "real built-in archive was not built"
    raw_members = run(["ar", "t", str(build / target)]).stdout.splitlines()
    members = [
        (Path(name).resolve().relative_to(build).as_posix()
         if Path(name).is_absolute() else name)
        for name in raw_members
    ]
    assert members and "init/main.o" in members, members
    assert (build / "init/.main.o.cmd").is_file()
    db = args.out.resolve() / "configured.sqlite"
    db.parent.mkdir(parents=True, exist_ok=True)
    command = [
        str(args.binary.resolve()), "graph", "kbuild",
        "--repo", str(repo), "--rev", LINUX, "--build", str(build),
        "--archive", target, "--ar", "ar",
        "--db", str(db), "--trusted-local", "true",
    ]
    observed = json.loads(run(command).stdout)
    again = json.loads(run(command).stdout)
    assert observed["schema"] == "excession.kbuild-graph.v1"
    assert observed["source_commit"] == LINUX
    assert observed["archive"] == target
    assert observed["archive_member_count"] == len(members)
    assert observed["graph_id"] == again["graph_id"]
    assert observed["git_verified_source_count"] >= 1
    assert observed["recorded_compile_action_count"] >= 1
    edges = observed["edges"]
    assert sum(x["kind"] == "CONTAINS_OBJECT" for x in edges) == len(members)
    assert any(x["from"] == "archive:init/built-in.a" and x["to"] == "object:init/main.o"
               and x["kind"] == "CONTAINS_OBJECT" for x in edges)
    assert any(x["to"] == "source:init/main.c" and x["status"] == "GIT_SOURCE_VERIFIED"
               for x in edges), edges
    assert any(x["kind"] == "HAS_BUILD_RECORD" for x in edges)
    # Same generic API as the CMake graph, with no Kbuild-specific query.
    imported = json.loads(run([
        str(args.binary.resolve()), "graph", "import",
        "--repo", str(repo), "--build", str(build),
        "--target", target, "--db", str(db),
        "--trusted-local", "true",
    ]).stdout)
    assert imported["schema"] == "excession.graph-summary.v1"
    assert imported["provider"] == "kbuild"
    assert imported["graph_id"] == observed["graph_id"]
    listed = json.loads(run([
        str(args.binary.resolve()), "graph", "ls", "--db", str(db),
    ]).stdout)
    assert any(row["graph_id"] == imported["graph_id"] for row in listed["graphs"])
    neighborhood = json.loads(run([
        str(args.binary.resolve()), "graph", "deps", "--db", str(db),
        "--node", target, "--id", imported["graph_id"],
    ]).stdout)
    assert any(e["relation"] == "contains" and e["to"] == "object:init/main.o"
               for e in neighborhood["edges"])
    evidence_path = json.loads(run([
        str(args.binary.resolve()), "graph", "path", "--db", str(db),
        "--from", target, "--to", "init/main.c", "--id", imported["graph_id"],
    ]).stdout)
    assert evidence_path["status"] == "RECORDED_EVIDENCE_PATH"
    assert evidence_path["edges"][-1]["relation"] == "reads"
    assert any(n["role"] == "source" for n in evidence_path["nodes"])
    with sqlite3.connect(db) as connection:
        stored, = connection.execute(
            "SELECT COUNT(*) FROM configured_graph WHERE graph_id=?",
            (observed["graph_id"],)).fetchone()
        assert stored == 1
        nodes, = connection.execute(
            "SELECT COUNT(*) FROM configured_entity WHERE graph_id=?",
            (observed["graph_id"],)).fetchone()
        assert nodes == len(observed["nodes"])
        stored_edges, = connection.execute(
            "SELECT COUNT(*) FROM configured_edge WHERE graph_id=?",
            (observed["graph_id"],)).fetchone()
        assert stored_edges == len(edges)
        exported, = connection.execute(
            "SELECT COUNT(*) FROM configured_observation WHERE graph_id=?",
            (observed["graph_id"],)).fetchone()
        assert exported == 1
    # Mutating the tracked source is not accepted as equivalent historical input.
    original = repo / "init/main.c"
    raw = original.read_bytes()
    try:
        original.write_bytes(raw + b"\n/* modified for acceptance */\n")
        altered = run(command, okay=False)
        assert altered.returncode != 0, "mismatched Git source unexpectedly accepted"
        assert "Git" in altered.stderr or "git" in altered.stderr
    finally:
        original.write_bytes(raw)
    denied = run(command[:-1] + ["false"], okay=False)
    assert denied.returncode != 0
    assert "trusted-local" in denied.stderr
    # An out-of-tree Linux build may leave untracked generated files in the
    # source directory. The authoritative tracked source must remain unchanged.
    tracked = run(["git", "-C", str(repo), "status", "--porcelain",
                   "--untracked-files=no"]).stdout.strip()
    assert not tracked, f"tracked source changed after acceptance: {tracked}"
    report = {
        "schema": "excession.kbuild-acceptance.v1",
        "source_commit": LINUX,
        "archive": target,
        "archive_members": len(members),
        "recorded_compile_actions": observed["recorded_compile_action_count"],
        "git_verified_sources": observed["git_verified_source_count"],
        "unresolved_objects": observed["unresolved_object_count"],
        "nodes": len(observed["nodes"]),
        "edges": len(edges),
        "graph_id": observed["graph_id"],
        "independent_ar_membership_matches": True,
        "sqlite_receipt_and_graph_verified": True,
        "historical_source_mutation_refused": True,
        "trust_consent_refused": True,
        "scope": "one Kbuild archive, not all configured Linux objects or runtime dependencies",
    }
    args.out.joinpath("summary.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))

if __name__ == "__main__":
    main()
