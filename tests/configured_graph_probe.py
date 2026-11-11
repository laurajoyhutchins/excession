#!/usr/bin/env python3
"""Configured graph acceptance against real zlib CMake/Ninja Debug/Release builds.

An original source file may be compiled into multiple targets. Select the
actual example target compile action explicitly, without rewriting the build's
compilation database or pretending an arbitrary command was executed.
"""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess

COMMIT = "51b7f2abdade71cd9bb0e7a373ef2610ec6f9daf"
TARGET = "example"
SOURCE = "test/example.c"


def run(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f"command failed: {args}\n{result.stderr[-6000:]}")
    return result.stdout.strip()


def cmake_target(build, repo):
    reply = build / ".cmake/api/v1/reply"
    indexes = sorted(reply.glob("index-*.json"))
    assert indexes, "CMake File API did not reply"
    header = json.loads(indexes[-1].read_text())
    codemodels = [o for o in header["objects"]
                  if o["kind"] == "codemodel" and o["version"]["major"] == 2]
    assert len(codemodels) == 1
    codemodel = json.loads((reply / codemodels[0]["jsonFile"]).read_text())
    assert Path(codemodel["paths"]["source"]).resolve() == repo
    config, = codemodel["configurations"]
    found = [target for target in config["targets"] if target["name"] == TARGET]
    assert len(found) == 1
    target = json.loads((reply / found[0]["jsonFile"]).read_text())
    assert target["name"] == TARGET
    sources = [s["path"] for s in target["sources"]
               if s["path"].endswith((".c", ".cc", ".cpp", ".cxx", ".C"))]
    assert sources == [SOURCE], sources
    assert target.get("artifacts"), target
    assert target.get("dependencies"), "example must depend on zlib"
    return target


def one_configuration(repo, base, variant, binary, db):
    # zlib 1.3.1's CMake intentionally renames tracked zconf.h in its
    # source directory. Isolate each build so it cannot alter the canonical
    # pinned checkout or contaminate the other configuration.
    source = base / (variant.lower() + "-source")
    run(["git", "clone", "--quiet", str(repo), str(source)])
    assert run(["git", "-C", str(source), "rev-parse", "HEAD"]) == COMMIT
    repo = source.resolve()
    build = base / variant.lower()
    (build / ".cmake/api/v1/query").mkdir(parents=True)
    (build / ".cmake/api/v1/query/codemodel-v2").touch()
    run([
        "cmake", "-S", str(repo), "-B", str(build), "-G", "Ninja",
        "-DCMAKE_C_COMPILER=clang", "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
        "-DCMAKE_BUILD_TYPE=" + variant,
        "-DZLIB_BUILD_EXAMPLES=ON",
    ])
    run(["cmake", "--build", str(build), "--target", TARGET, "--parallel", "2"])
    target = cmake_target(build, repo)
    native = run(["ninja", "-C", str(build), "-t", "graph", TARGET])
    assert "digraph" in native and SOURCE in native
    for output in target["artifacts"]:
        assert Path(output["path"]).name in native, output

    receipt_dir = base / (variant.lower() + "-receipts")
    receipt_dir.mkdir()
    command = [
        str(binary), "analyze", "clang", "--repo", str(repo), "--rev", COMMIT,
        "--source", SOURCE, "--compile-db", str(build / "compile_commands.json"),
        "--compiler", "clang",
        "--object-prefix", "CMakeFiles/example.dir/",
        "--trusted-local", "true",
    ]
    receipt = json.loads(run(command))
    assert receipt["source_commit"] == COMMIT
    assert receipt["observation"]["source_path"] == SOURCE
    assert receipt["observation"]["status"] == "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED"
    expected_blob = run(["git", "-C", str(repo), "rev-parse", f"{COMMIT}:{SOURCE}"])
    assert receipt["observation"]["source_blob"] == expected_blob
    (receipt_dir / "example.json").write_text(json.dumps(receipt, sort_keys=True) + "\n")

    assemble_cmd = [
        str(binary), "graph", "assemble",
        "--repo", str(repo), "--rev", COMMIT,
        "--build", str(build), "--target", TARGET,
        "--observations", str(receipt_dir), "--db", str(db),
    ]
    graph = json.loads(run(assemble_cmd))
    assert graph["schema"] == "excession.configured-graph.v1"
    assert graph["configuration"] == variant
    assert graph["source_commit"] == COMMIT
    assert graph["status"] == "TARGET_TUS_OBSERVED"
    assert graph["expected_source_count"] == 1
    assert graph["observed_source_count"] == 1
    assert graph["unobserved_sources"] == []
    assert graph["graph_id"] == json.loads(run(assemble_cmd))["graph_id"]

    # A generated header is a local dependency with exact byte evidence.
    # After it changes, reusing the old compiler receipt must be refused.
    generated = next(
        (Path(item["path"]) for item in receipt["observation"]["dependencies"]
         if Path(item["path"]).is_relative_to(build)
         and Path(item["path"]).is_file()),
        None,
    )
    assert generated is not None, "zlib build should provide a generated header"
    initial = generated.read_bytes()
    try:
        generated.write_bytes(initial + b"\\n/* stale dependency probe */\\n")
        stale = subprocess.run(assemble_cmd, capture_output=True, text=True)
        assert stale.returncode != 0
        assert "compiler dependency changed" in stale.stderr
    finally:
        generated.write_bytes(initial)


    # The generic interface gives this graph the same shape as Kbuild.
    generic_args = [
        str(binary), "graph", "import",
        "--repo", str(repo), "--rev", COMMIT,
        "--build", str(build), "--target", TARGET,
        "--observations", str(receipt_dir), "--db", str(db),
    ]
    imported = json.loads(run(generic_args))
    assert imported["schema"] == "excession.graph-summary.v1"
    assert imported["graph_id"] == graph["graph_id"]
    assert imported["provider"] == "cmake"
    assert imported["node_count"] == len(graph["nodes"])
    listed = json.loads(run([str(binary), "graph", "ls", "--db", str(db)]))
    assert any(x["graph_id"] == imported["graph_id"] for x in listed["graphs"])
    shown = json.loads(run([str(binary), "graph", "show", "--db", str(db),
                            "--id", graph["graph_id"][:12]]))
    assert shown["graph_id"] == graph["graph_id"]
    neighbors = json.loads(run([str(binary), "graph", "deps", "--db", str(db),
                                "--id", graph["graph_id"], "--node", TARGET]))
    assert any(x["relation"] == "uses" and x["to"] == f"source:{SOURCE}"
               for x in neighbors["edges"])
    assert any(x["role"] == "source" for x in neighbors["nodes"])
    path = json.loads(run([str(binary), "graph", "path", "--db", str(db),
                           "--id", graph["graph_id"], "--from", TARGET,
                           "--to", SOURCE]))
    assert path["status"] == "RECORDED_EVIDENCE_PATH"
    assert path["edges"][0]["relation"] == "uses"
    edge_types = {edge["kind"] for edge in graph["edges"]}
    required = {
        "TARGET_SOURCE", "DEPENDS_ON_TARGET", "PRODUCES_ARTIFACT",
        "OBSERVED_AS_TU", "PREPROCESSOR_INPUT", "DECLARES", "CONTAINS_CALL",
    }
    assert required.issubset(edge_types), edge_types
    assert any(e["from"] == "target:example" and e["to"] == "target:zlib"
               for e in graph["edges"]), "CMake target dependency not joined"
    assert any(e["from"] == "target:example" and e["to"] == f"source:{SOURCE}"
               for e in graph["edges"])
    assert any(n["id"] == f"source:{SOURCE}"
               and n["payload"]["blob"] == expected_blob
               for n in graph["nodes"])

    # Missing source observation must remain visibly incomplete.
    empty = base / (variant.lower() + "-missing")
    empty.mkdir()
    partial = assemble_cmd.copy()
    partial[partial.index("--observations") + 1] = str(empty)
    partial_graph = json.loads(run(partial))
    assert partial_graph["status"] == "PARTIAL_TARGET_OBSERVATIONS"
    assert partial_graph["expected_source_count"] == 1
    assert partial_graph["observed_source_count"] == 0
    assert partial_graph["unobserved_sources"] == [SOURCE]
    assert partial_graph["graph_id"] != graph["graph_id"]

    # An altered receipt cannot bind itself to the configured graph.
    bad = dict(receipt)
    bad["source_commit"] = "0" * 40
    (receipt_dir / "example.json").write_text(json.dumps(bad, sort_keys=True))
    failed = subprocess.run(assemble_cmd, capture_output=True, text=True)
    assert failed.returncode != 0
    assert "receipt" in failed.stderr.lower()

    # The fixture may be dirty from zlib's own build preparation. Only
    # the selected C source blob is claimed to be Git-authenticated.
    changed_checkout = run(["git", "-C", str(repo), "status", "--porcelain"])
    return {
        "configuration": variant,
        "source": SOURCE,
        "source_blob": expected_blob,
        "graph_id": graph["graph_id"],
        "target": TARGET,
        "edge_types": sorted(edge_types),
        "nodes": len(graph["nodes"]),
        "edges": len(graph["edges"]),
        "coverage": graph["status"],
        "native_ninja_graph_agrees_with_cmake_source_and_artifact": True,
        "direct_cmake_target_dependency": "zlib",
        "compiler_receipt": receipt["report_id"],
        "actual_build": True,
        "build_mutated_temporary_source_clone": bool(changed_checkout),
        "source_tree_scope": "selected file Git bytes; generated headers local hash only",
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    opts = parser.parse_args()
    repo = opts.repo.resolve()
    binary = opts.binary.resolve()
    output = opts.output.resolve()
    output.mkdir(parents=True)
    assert run(["git", "-C", str(repo), "rev-parse", "HEAD"]) == COMMIT
    assert run(["git", "-C", str(repo), "status", "--porcelain"]) == ""
    db = output / "graph.sqlite"
    cases = [one_configuration(repo, output, name, binary, db)
             for name in ("Debug", "Release")]
    assert cases[0]["source_blob"] == cases[1]["source_blob"]
    assert cases[0]["graph_id"] != cases[1]["graph_id"]
    with sqlite3.connect(db) as connection:
        count, = connection.execute("SELECT COUNT(*) FROM configured_graph").fetchone()
        assert count >= 4, "two full and two partial configuration-scoped graphs"
        for row in cases:
            nodes, = connection.execute(
                "SELECT COUNT(*) FROM configured_entity WHERE graph_id=?",
                (row["graph_id"],)).fetchone()
            edges, = connection.execute(
                "SELECT COUNT(*) FROM configured_edge WHERE graph_id=?",
                (row["graph_id"],)).fetchone()
            assert nodes == row["nodes"] and edges == row["edges"]
            receipts, = connection.execute(
                "SELECT COUNT(*) FROM configured_observation WHERE graph_id=?",
                (row["graph_id"],)).fetchone()
            assert receipts == 1
            raw_receipt, = connection.execute(
                "SELECT document FROM configured_observation WHERE graph_id=?",
                (row["graph_id"],)).fetchone()
            assert json.loads(raw_receipt)["report_id"] == row["compiler_receipt"]

    report = {
        "schema": "excession.configured-graph-acceptance.v1",
        "upstream": "madler/zlib v1.3.1",
        "source_commit": COMMIT,
        "cases": cases,
        "sqlite_snapshots": count,
        "verdict": "CONFIGURATION_SCOPED_GRAPH_ASSEMBLY_TESTED",
        "limits": "No whole-program call resolution or dynamic dependency completeness",
    }
    (output / "summary.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
