#!/usr/bin/env python3
"""Gate a release on independently checked real source using its extracted binary.

Uses the existing h11/Axios source-oracle implementation, not an alternate
parser or Excession's own SQLite rows. A pass is deliberately scoped to
statically resolved in-repository Python and JS/TS imports only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import resource
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "benchmarks/experiments"))
from real_fixtures import checkout, git_blobs, verify_product_workflow  # noqa: E402
from tools.release_manifest import build_manifest  # noqa: E402

TARGET = "x86_64-unknown-linux-gnu"
SELECTED = ("h11", "axios")


def command_version(*args: str) -> str:
    result = subprocess.run(args, capture_output=True, text=True, timeout=30, check=True)
    return result.stdout.strip().splitlines()[0]


def run_archive(dist: Path, fixture_file: Path) -> dict:
    version = tomllib.loads((ROOT / "core/Cargo.toml").read_text())["package"]["version"]
    source_commit = command_version("git", "-C", str(ROOT), "rev-parse", "HEAD")
    manifest = build_manifest(dist, version, source_commit)
    assets = [asset for asset in manifest["assets"] if asset["target"] == TARGET]
    if len(assets) != 1 or len(manifest["assets"]) != 1:
        raise ValueError(f"expected exactly one {TARGET} platform archive")
    asset = assets[0]
    archive = dist / asset["filename"]
    declared = json.loads(fixture_file.read_text(encoding="utf-8"))
    if declared.get("schema") != "excession.real-projection-fixtures.v1":
        raise ValueError("unknown independent fixture specification")
    fixtures = {fixture["name"]: fixture for fixture in declared["fixtures"]}
    if not all(name in fixtures for name in SELECTED):
        raise ValueError("required h11 and Axios fixtures not pinned")
    start = time.perf_counter()
    results = []
    with tempfile.TemporaryDirectory(prefix="excession-installed-acceptance-") as dirname:
        work = Path(dirname)
        installed = work / "installed"
        installed.mkdir()
        with tarfile.open(archive, "r:gz") as package:
            package.extractall(installed, filter="data")
        binary = installed / "excession/excession"
        if not binary.is_file():
            raise ValueError("portable archive does not contain an executable")
        if not (installed / "excession/lib/typescript.js").is_file():
            raise ValueError("portable archive lacks its TypeScript parser sidecar")
        binary_version = command_version(str(binary), "--version")
        if binary_version != f"excession {version}":
            raise ValueError(f"binary version differs from archive version: {binary_version}")
        doctor = json.loads(subprocess.check_output(
            [str(binary), "doctor", "--json"], text=True, timeout=30,
        ))
        if doctor["schema"] != "excession.capabilities.v1" or not doctor["required_ready"]:
            raise ValueError("installed binary lacks mandatory Git capability")
        if not doctor["capabilities"]["python_index"] or not doctor["capabilities"]["javascript_index"]:
            raise ValueError("independent release oracle requires Python and JS analyzers")
        for name in SELECTED:
            fixture = fixtures[name]
            location = work / f"source-{name}"
            commit, tree = checkout(fixture, location)
            inventory = git_blobs(location, commit)
            case = verify_product_workflow(
                binary, location, work, fixture, commit, tree, inventory,
            )
            results.append({
                "name": name, "repository": fixture.get("repository", "bundled h11"),
                "source_commit": commit, "source_tree": tree,
                **case,
            })
    return {
        "schema": "excession.installed-real-repository-acceptance.v1",
        "status": "PASS",
        "source_commit": source_commit,
        "archive": asset,
        "binary_version": binary_version,
        "fixture_manifest_sha256": hashlib.sha256(fixture_file.read_bytes()).hexdigest(),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "git": command_version("git", "--version"),
        "node": command_version("node", "--version"),
        "cases": results,
        "wall_seconds_including_extract_tool_discovery_and_oracles": round(time.perf_counter() - start, 3),
        "peak_child_rss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
        "peak_rss_scope": "maximum child-process RSS observed across Git, interpreter, and CLI; not a sum or per-step peak",
        "oracle_scope": "only statically resolved in-repository Python and JS/TS import paths at pinned Git commits",
        "excluded": "dynamic imports, external packages, runtime behavior, native build semantics, and unconfigured paths",
        "semantic_correctness": "NOT_EVALUATED",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    try:
        report = run_archive(args.dist.resolve(), args.manifest.resolve())
    except Exception as error:
        message = f"{type(error).__name__}: {error}"
        report = {
            "schema": "excession.installed-real-repository-acceptance.v1",
            "status": "BLOCKED",
            "error": message,
            "failure_type": (
                "UPSTREAM_OR_NETWORK" if "fetch" in message or "Network" in message
                else "PRODUCT_OR_ENVIRONMENT"
            ),
        }
    args.out.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
