#!/usr/bin/env python3
"""Independent configured graph acceptance against two real fmt CMake/Ninja builds.

No source repository modifications. CMake/Ninja creates actual target artifacts;
Excession reparses the associated real compilation commands using Clang and
joins them with the file-API codemodel in its own SQLite database.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import time

COMMIT = "123913715afeb8a437e6388b4473fcc4753e1c9a"
TARGET = "fmt"


def run(args, *, capture=True):
    result = subprocess.run(args, check=False, text=True,
                            stdout=subprocess.PIPE if capture else None,
                            stderr=subprocess.PIPE if capture else None,
                            timeout=240)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {args}\n"
            + (result.stderr or "")[-4000:]
        )
    return (result.stdout or "").strip()


def locate_sources(build, repo):
    reply = build / ".cmake/api/v1/reply"
    indices = sorted(reply.glob("index-*.json"))
    assert indices, "CMake File API missing index"
    index = json.loads(indices[-1].read_text())
    models = [o for o in index["objects"] if o["kind"] == "codemodel"
              and o["version"]["major"] == 2]
    assert len(models) == 1
    model = json.loads((reply / models[0]["jsonFile"]).read_text())
    assert Path(model["paths"]["source"]).resolve() == repo.resolve()
    config, = model["configurations"]
    targets = [t for t in config["targets"] if t["name"] == TARGET]
    assert len(targets) == 1
    target = json.loads((reply / targets[0]["jsonFile"]).read_text())
    paths = []
    for source in target["sources"]:
        if Path(source["path"]).suffix not in (".c", ".cc", ".cpp", ".cxx"):
            continue
        physical = (repo / source["path"]).resolve()
        if physical.is_relative_to(repo) and physical.is_file():
            paths.append(physical.relative_to(repo).as_posix())
    assert paths and len(paths) == len(set(paths))
    assert target.get("artifacts")
    return sorted(paths), target


def build_and_assemble(repo, output, variant, excession, db):
    build = output / variant.lower()
    (build / ".cmake/api/v1/query").mkdir(parents=True)
    (build / ".cmake/api/v1/query/codemodel-v2").touch()
    run([
        "cmake", "-S", str(repo), "-B", str(build), "-G", "Ninja",
        "-DCMAKE_CXX_COMPILER=clang++",
        "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
        "-DCMAKE_BUILD_TYPE=" + variant,
        "-DFMT_TEST=OFF", "-DFMT_DOC=OFF", "-DFMT_INSTALL=OFF",
    ])
    run(["cmake", "--build", str(build), "--target", TARGET, "--parallel", "2"])
    sources, target = locate_sources(build, repo)
    native_graph = run(["ninja", "-C", str(build), "-t", "graph", TARGET])
    assert "digraph" in native_graph
    for path in sources:
        assert path in native_graph or Path(path).name in native_graph, (
            variant, "Ninja does not show selected CMake source", path
        )
    for artifact in target["artifacts"]:
        filename = Path(artifact["path"]).name
        assert filename in native_graph, (
            variant, "Ninja graph does not include target artifact", filename
        )
    receipts = output / (variant.lower() + "-observations")
    receipts.mkdir()
    start = time.monotonic()
    for index, source in enumerate(sources):
        report = json.loads(run([
            str(excession), "analyze", "clang",
            "--repo", str(repo), "--rev", COMMIT,
            "--source", source,
            "--compile-db", str(build / "compile_commands.json"),
            "--compiler", "clang++", "--trusted-local", "true",
        ]))
        assert report["source_commit"] == COMMIT
        assert report["observation"]["source_path"] == source
        (receipts / f"{index:03d}.json").write_text(
            json.dumps(report, sort_keys=True) + "\n"
        )

    args = [
        str(excession), "graph", "assemble",
        "--repo", str(repo), "--rev", COMMIT,
        "--build", str(build), "--target", TARGET,
        "--observations", str(receipts), "--db", str(db),
    ]
    assembled = json.loads(run(args))
    assert assembled["schema"] == "excession.configured-graph.v1"
    assert assembled["source_commit"] == COMMIT
    assert assembled["configuration"] == variant
    assert assembled["status"] == "TARGET_TUS_OBSERVED"
    assert assembled["expected_source_count"] == len(sources)
    assert assembled["observed_source_count"] == len(sources)
    assert not assembled["unobserved_sources"]
    assert assembled["graph_id"] == json.loads(run(args))["graph_id"]
    edge_types = {edge["kind"] for edge in assembled["edges"]}
    assert {
        "TARGET_SOURCE", "PRODUCES_ARTIFACT",
        "OBSERVED_AS_TU", "PREPROCESSOR_INPUT",
        "DECLARES", "CONTAINS_CALL",
    }.issubset(edge_types), edge_types
    for source in sources:
        assert any(edge["from"] == "target:fmt"
                   and edge["to"] == f"source:{source}"
                   for edge in assembled["edges"])
    for item in target["artifacts"]:
        assert any(edge["to"] == "artifact:" + item["path"]
                   for edge in assembled["edges"])

    # Partial mode: missing receipt does NOT imply a missing source edge.
    first = sorted(receipts.glob("*.json"))[0]
    partial_dir = output / (variant.lower() + "-partial")
    partial_dir.mkdir()
    for path in sorted(receipts.glob("*.json")):
        if path != first:
            (partial_dir / path.name).write_bytes(path.read_bytes())
    if len(sources) > 1:
        partial_args = args.copy()
        partial_args[partial_args.index("--observations") + 1] = str(partial_dir)
        partial = json.loads(run(partial_args))
        assert partial["status"] == "PARTIAL_TARGET_OBSERVATIONS"
        assert partial["observed_source_count"] == len(sources) - 1
        assert partial["expected_source_count"] == len(sources)

    # Mutating a receipt's claimed Git identity must be rejected.
    bad = json.loads(first.read_text())
    bad["source_commit"] = "0" * 40
    first.write_text(json.dumps(bad))
    check = subprocess.run(args, capture_output=True, text=True)
    assert check.returncode != 0, "mutated source identity was accepted"

    return {
        "configuration": variant,
        "target": TARGET,
        "source_commit": COMMIT,
        "graph_id": assembled["graph_id"],
        "sources": sources,
        "expected_tus": assembled["expected_source_count"],
        "observed_tus": assembled["observed_source_count"],
        "nodes": len(assembled["nodes"]),
        "edges": len(assembled["edges"]),
        "native_build_tool": "Ninja",
        "independent_ninja_source_and_artifact_checks": True,
        "elapsed_clang_and_assembly_seconds": round(time.monotonic() - start, 3),
        "scope": "configured graph and Clang observations, not complete C++ symbol binding",
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", type=Path, required=True)
    ap.add_argument("--binary", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    repo, binary, output = args.repo.resolve(), args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True)
    assert run(["git", "-C", str(repo), "rev-parse", "HEAD"]) == COMMIT
    assert run(["git", "-C", str(repo), "status", "--porcelain"]) == ""
    db = output / "configured-graphs.sqlite"
    cases = [
        build_and_assemble(repo, output, config, binary, db)
        for config in ("Debug", "Release")
    ]
    assert cases[0]["sources"] == cases[1]["sources"], "different target source set"
    assert cases[0]["graph_id"] != cases[1]["graph_id"], "configuration was collapsed"
    with sqlite3.connect(db) as connection:
        graph_count, = connection.execute("SELECT COUNT(*) FROM configured_graph").fetchone()
        assert graph_count >= 2
        for case in cases:
            stored_nodes, = connection.execute(
                "SELECT COUNT(*) FROM configured_entity WHERE graph_id=?",
                (case["graph_id"],)).fetchone()
            stored_edges, = connection.execute(
                "SELECT COUNT(*) FROM configured_edge WHERE graph_id=?",
                (case["graph_id"],)).fetchone()
            assert stored_nodes == case["nodes"]
            assert stored_edges == case["edges"]
    summary = {
        "schema": "excession.configured-graph-acceptance.v1",
        "cases": cases,
        "sqlite_snapshots": graph_count,
        "conclusion": "CONFIGURATION_SCOPED_TARGET_OBSERVATIONS",
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
