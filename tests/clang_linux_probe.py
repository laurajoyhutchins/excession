#!/usr/bin/env python3
"""Pinned Linux v6.12 C AST probe.

Linux's ordinary C feature test is self-contained; the synthetic compilation
database here exercises Clang ingestion but is NOT a kernel .cmd build receipt.
A real Kbuild database is supplied separately using Linux's upstream tool.
"""
import argparse
import json
from pathlib import Path
import subprocess

COMMIT = "adc218676eef25575469234709c2d87185ca223a"
PATH = "tools/build/feature/test-hello.c"


def run(args, *, success=True):
    result = subprocess.run(args, capture_output=True, text=True)
    if success and result.returncode:
        raise RuntimeError(result.stderr[-5000:])
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--binary", required=True, type=Path)
    ap.add_argument("--output", required=True, type=Path)
    args = ap.parse_args()
    repo, output = args.repo.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    commit = run(["git", "-C", str(repo), "rev-parse", "HEAD"]).stdout.strip()
    assert commit == COMMIT

    compile_db = output / "compile_commands.json"
    compile_db.write_text(json.dumps([{
        "directory": str(repo),
        "file": PATH,
        "arguments": ["clang", "-c", PATH, "-o", "test-hello.o"]
    }], sort_keys=True))
    command = [
        str(args.binary.resolve()), "kernel", "clang",
        "--repo", str(repo), "--rev", COMMIT,
        "--source", PATH, "--compile-db", str(compile_db),
        "--compiler", "clang", "--trusted-local", "true"
    ]
    result = json.loads(run(command).stdout)
    assert result["schema"] == "excession.clang-tu.v1"
    assert result["source_commit"] == COMMIT
    obs = result["observation"]
    assert obs["status"] == "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED"
    assert obs["source_path"] == PATH
    git_oid = run(["git", "-C", str(repo), "rev-parse", f"{COMMIT}:{PATH}"]).stdout.strip()
    assert obs["source_blob"] == git_oid
    assert any(x["name"] == "main" and x["definition"] for x in obs["symbols"])
    assert any(x["caller"] == "main" and x["callee"] == "puts" for x in obs["calls"])
    assert any(x["path"].endswith("stdio.h") for x in obs["dependencies"])
    assert any(x["repo_path"] == PATH for x in obs["dependencies"])
    assert result["report_id"] == json.loads(run(command).stdout)["report_id"]

    # Fail closed if the source no longer matches its Git blob.
    source = repo / PATH
    original = source.read_bytes()
    try:
        source.write_bytes(original + b"\n/* locally modified */\n")
        rejection = run(command, success=False)
        assert rejection.returncode != 0
        assert "pinned Git blob" in rejection.stderr
    finally:
        source.write_bytes(original)

    refusal = run(command[:-2] + ["--trusted-local", "false"], success=False)
    assert refusal.returncode != 0
    assert "trusted-local" in refusal.stderr

    summary = {
        "source_commit": COMMIT,
        "source_path": PATH,
        "source_blob": git_oid,
        "report_id": result["report_id"],
        "compiler_version": obs["compiler_version"],
        "dependency_files": len(obs["dependencies"]),
        "definitions": sum(x["definition"] for x in obs["symbols"]),
        "calls": len(obs["calls"]),
        "status": obs["status"],
        "provenance_note": "An existing Linux C feature test, NOT a kernel configuration build",
    }
    (output / "clang-summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
