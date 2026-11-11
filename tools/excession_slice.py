#!/usr/bin/env python3
"""Compatibility CLI for Rust-owned executable-slice operations.

CPython's AST import semantics live in core/adapters/python_slice_imports.py.
The Rust excession CLI owns Git access, manifests, construction, audits,
execution, receipts, and historical evidence transport. The compatibility
functions below retain the existing test and scripting interface.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from typing import Any

SCHEMA = "excession.executable-slice.v1"
RECEIPT = "excession.slice-observation.v1"


class Refusal(ValueError):
    pass


def _binary() -> Path:
    override = os.environ.get("EXCESSION_BIN")
    if override:
        return Path(override)
    local = Path(__file__).resolve().parents[1] / "core/target/debug/excession"
    if local.is_file():
        return local
    found = shutil.which("excession")
    if found:
        return Path(found)
    raise Refusal("build the Rust excession CLI or set EXCESSION_BIN")


def _invoke(*args: str) -> dict[str, Any]:
    try:
        result = subprocess.run([str(_binary()), "slice", *args],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=100, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Refusal(f"Rust slice authority unavailable: {exc}") from exc
    if result.returncode:
        raise Refusal(result.stderr[:1800].decode("utf-8", errors="replace"))
    if len(result.stdout) > 1024 * 1024:
        raise Refusal("Rust slice output exceeds 1 MiB")
    try:
        value = json.loads(result.stdout)
    except (ValueError, UnicodeError) as exc:
        raise Refusal("Rust slice authority returned invalid JSON") from exc
    if not isinstance(value, dict):
        raise Refusal("Rust slice authority returned non-object")
    return value


def _manifest_call(verb: str, repo: Path, manifest: dict[str, Any],
                   *options: str) -> dict[str, Any]:
    # This is a transport wrapper, not the authority for canonical identity.
    with tempfile.TemporaryDirectory(prefix="excession-slice-client-") as root:
        path = Path(root) / "manifest.json"
        path.write_text(json.dumps(manifest))
        return _invoke(verb, "--repo", str(repo), "--manifest", str(path), *options)


def plan(repo: Path, entry: str, revision: str = "HEAD", assets: list[str] | None = None,
         history: bool = False) -> dict[str, Any]:
    return _invoke("plan", "--repo", str(repo), "--rev", revision,
                   "--entry", entry, "--assets", json.dumps(assets or []),
                   "--history", str(history).lower())


def validate_manifest(repo: Path, manifest: dict[str, Any]) -> None:
    _manifest_call("validate", repo, manifest)


def materialize(repo: Path, manifest: dict[str, Any], destination: Path) -> None:
    _manifest_call("materialize", repo, manifest, "--directory", str(destination))


def audit(repo: Path, manifest: dict[str, Any], directory: Path) -> None:
    _manifest_call("source-audit", repo, manifest, "--directory", str(directory))


def execute(repo: Path, manifest: dict[str, Any], directory: Path, timeout: int,
            isolated_linux: bool = False) -> dict[str, Any]:
    # Invoking this programmatic API is an explicit local execution request.
    # The command-line interface below still enforces --trusted-local.
    return _manifest_call("run", repo, manifest, "--directory", str(directory),
                          "--timeout", str(timeout), "--trusted-local", "true",
                          "--isolated-linux", str(isolated_linux).lower())


def carry(repo: Path, previous: dict[str, Any], receipt: dict[str, Any],
          revision: str = "HEAD") -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="excession-carry-client-") as root:
        manifest = Path(root) / "manifest.json"
        observation = Path(root) / "receipt.json"
        manifest.write_text(json.dumps(previous))
        observation.write_text(json.dumps(receipt))
        return _invoke("carry", "--repo", str(repo), "--manifest", str(manifest),
                       "--receipt", str(observation), "--rev", revision)


def read_json(path: str) -> dict[str, Any]:
    value = json.loads(Path(path).read_text())
    if not isinstance(value, dict):
        raise Refusal("JSON object required")
    return value


def cli() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("plan")
    p.add_argument("repo", type=Path)
    p.add_argument("entry")
    p.add_argument("--revision", default="HEAD")
    p.add_argument("--asset", action="append", default=[])
    p.add_argument("--history", action="store_true")
    p = sub.add_parser("materialize")
    p.add_argument("repo", type=Path)
    p.add_argument("manifest")
    p.add_argument("destination", type=Path)
    p = sub.add_parser("audit")
    p.add_argument("repo", type=Path)
    p.add_argument("manifest")
    p.add_argument("directory", type=Path)
    p = sub.add_parser("run")
    p.add_argument("repo", type=Path)
    p.add_argument("manifest")
    p.add_argument("directory", type=Path)
    p.add_argument("--timeout", type=int, default=10)
    p.add_argument("--trusted-local", action="store_true")
    p.add_argument("--isolated-linux", action="store_true")
    p = sub.add_parser("carry")
    p.add_argument("target_repo", type=Path)
    p.add_argument("source_manifest")
    p.add_argument("source_receipt")
    p.add_argument("--revision", default="HEAD")
    p.add_argument("--replay", action="store_true")
    p.add_argument("--trusted-local", action="store_true")
    p.add_argument("--isolated-linux", action="store_true")
    args = parser.parse_args()
    try:
        if args.action == "plan":
            result = plan(args.repo, args.entry, args.revision, args.asset, args.history)
        elif args.action == "materialize":
            materialize(args.repo, read_json(args.manifest), args.destination)
            result = {"status": "MATERIALIZED", "directory": str(args.destination)}
        elif args.action == "audit":
            audit(args.repo, read_json(args.manifest), args.directory)
            result = {"status": "AUDITED"}
        elif args.action == "run":
            if not args.trusted_local:
                raise Refusal("native execution requires explicit --trusted-local")
            result = execute(args.repo, read_json(args.manifest), args.directory,
                             args.timeout, args.isolated_linux)
        else:
            if args.replay and not args.trusted_local:
                raise Refusal("target execution requires explicit --trusted-local")
            result = _invoke("carry", "--repo", str(args.target_repo),
                             "--manifest", args.source_manifest,
                             "--receipt", args.source_receipt,
                             "--rev", args.revision,
                             "--replay", str(args.replay).lower(),
                             "--trusted-local", str(args.trusted_local).lower(),
                             "--isolated-linux", str(args.isolated_linux).lower())
        print(json.dumps(result, indent=2, sort_keys=True))
    except (Refusal, OSError, ValueError, KeyError, TypeError) as exc:
        parser.exit(2, f"excession slice refused: {exc}\n")


if __name__ == "__main__":
    cli()
