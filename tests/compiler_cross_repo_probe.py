#!/usr/bin/env python3
"""Exercise one shared Clang analysis path against independent upstream repos.

The compilation databases here are explicit fixture commands, NOT receipts
that zlib/fmt were built with those flags. Only Clang reparsing is performed.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

CASES = (
    {
        "name": "zlib",
        "commit": "51b7f2abdade71cd9bb0e7a373ef2610ec6f9daf",
        "source": "test/example.c",
        "compiler": "clang",
        "original_compiler": "riscv64-linux-gnu-gcc",
        "flags": ["-std=c11", "-I."],
        "language": "c",
        "definition": "main",
    },
    {
        "name": "fmt",
        "commit": "123913715afeb8a437e6388b4473fcc4753e1c9a",
        "source": "test/static-export-test/main.cc",
        "compiler": "clang++",
        "original_compiler": "x86_64-linux-gnu-g++",
        "flags": ["-std=c++17"],
        "language": "c++",
        "definition": "main",
    },
)


def execute(argv, *, success=True):
    started = time.monotonic()
    proc = subprocess.run(argv, capture_output=True, text=True, timeout=90)
    duration = round(time.monotonic() - started, 3)
    if success and proc.returncode:
        raise AssertionError(
            "command failed: " + repr(argv) + "\n" + proc.stderr[-3500:])
    return proc, duration


def verify_case(case, repo, binary, output):
    name = case["name"]
    commit, _ = execute(["git", "-C", str(repo), "rev-parse", "HEAD"])
    assert commit.stdout.strip() == case["commit"]
    dirty, _ = execute(["git", "-C", str(repo), "status", "--porcelain"])
    assert not dirty.stdout.strip(), f"{name}: checkout is dirty"

    compile_db = output / (name + "-compile_commands.json")
    compile_db.write_text(json.dumps([{
        "directory": str(repo),
        "file": case["source"],
        "arguments": [
            case["original_compiler"], *case["flags"],
            "-c", case["source"], "-o", str(output / (name + ".o"))
        ],
    }], sort_keys=True))
    cmd = [
        str(binary), "analyze", "clang", "--repo", str(repo),
        "--rev", case["commit"], "--source", case["source"],
        "--compile-db", str(compile_db), "--compiler", case["compiler"],
        "--trusted-local", "true",
    ]
    output_result, elapsed = execute(cmd)
    report = json.loads(output_result.stdout)
    observation = report["observation"]
    assert report["schema"] == "excession.clang-tu.v1"
    assert report["source_commit"] == case["commit"]
    assert observation["language"] == case["language"]
    assert observation["status"] == "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED"
    assert observation["source_path"] == case["source"]
    assert observation["original_compiler"] == case["original_compiler"]
    expected_oid, _ = execute([
        "git", "-C", str(repo), "rev-parse",
        f"{case['commit']}:{case['source']}"
    ])
    assert observation["source_blob"] == expected_oid.stdout.strip()
    definitions = [s["name"] for s in observation["symbols"] if s["definition"]]
    assert case["definition"] in definitions, (name, definitions)
    assert any(d["repo_path"] == case["source"]
               for d in observation["dependencies"]), name
    assert all(len(d["sha256"]) == 64 for d in observation["dependencies"])
    assert "clang" in observation["compiler_version"].lower(), name
    assert observation["effective_compiler"] == observation["effective_args"][0]
    if name == "fmt":
        assert any(c["callee"] == "foo" for c in observation["calls"]), (
            "the C++ call to foo should appear", observation["calls"]
        )
    else:
        # Legacy Linux spelling must use the exact same implementation.
        alias, _ = execute([str(binary), "kernel", "clang", *cmd[3:]])
        assert json.loads(alias.stdout)["report_id"] == report["report_id"]

    again, _ = execute(cmd)
    assert json.loads(again.stdout)["report_id"] == report["report_id"]
    no_trust, _ = execute(cmd[:-1] + ["false"], success=False)
    assert no_trust.returncode != 0
    assert "trusted-local" in no_trust.stderr
    return {
        "repository": name,
        "source_commit": case["commit"],
        "source_path": case["source"],
        "language": observation["language"],
        "source_blob": observation["source_blob"],
        "report_id": report["report_id"],
        "symbol_count": len(observation["symbols"]),
        "definition_count": len(definitions),
        "call_count": len(observation["calls"]),
        "dependency_count": len(observation["dependencies"]),
        "elapsed_seconds": elapsed,
        "status": observation["status"],
        "compilation_database_provenance": "constructed fixture, NOT an observed build",
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zlib", required=True, type=Path)
    ap.add_argument("--fmt", required=True, type=Path)
    ap.add_argument("--binary", required=True, type=Path)
    ap.add_argument("--output", required=True, type=Path)
    args = ap.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    repos = {"zlib": args.zlib.resolve(), "fmt": args.fmt.resolve()}
    rows = [verify_case(case, repos[case["name"]],
                        args.binary.resolve(), output) for case in CASES]
    report = {
        "schema": "excession.compiler-cross-repo-probe.v1",
        "cases": rows,
        "verdict": "COMPILER_OBSERVATIONS_ONLY",
    }
    (output / "summary.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps(report, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
