#!/usr/bin/env python3
"""Source-bound local behavioral controls over exact upstream method bodies.

Only the named method AST is compiled in an isolated namespace. No vendor package,
third-party dependencies, HTTP server, or shell command from source is executed.
The test is deliberately limited to parent traversal and benign file access.
"""
from __future__ import annotations

import argparse
import ast
import asyncio
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile
import types
import typing


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True,
        capture_output=True, text=True,
    ).stdout.strip()


def method_from_source(path: Path, name: str, namespace: dict) -> object:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    candidates = [
        method for owner in tree.body if isinstance(owner, ast.ClassDef)
        for method in owner.body
        if isinstance(method, (ast.FunctionDef, ast.AsyncFunctionDef))
        and method.name == name
    ]
    if len(candidates) != 1:
        raise ValueError(f"expected one upstream method {name}; found {len(candidates)}")
    method = copy.deepcopy(candidates[0])
    method.decorator_list = []
    isolated = ast.Module(body=[
        ast.ImportFrom(module="__future__", names=[ast.alias(name="annotations")], level=0),
        method,
    ], type_ignores=[])
    ast.fix_missing_locations(isolated)
    exec(compile(isolated, str(path), "exec"), namespace)
    return namespace[name]


def starlette_oracle(source: Path, root: Path) -> dict:
    outside = root.parent / "static-private.txt"
    outside.write_text("private-starlette", encoding="utf-8")
    public = root / "public.txt"
    public.write_text("public", encoding="utf-8")
    lookup = method_from_source(source, "lookup_path", {"os": os, "typing": typing})
    instance = types.SimpleNamespace(all_directories=[str(root)], follow_symlink=False)
    escaped, escaped_stat = lookup(instance, "../static-private.txt")
    normal, normal_stat = lookup(instance, "public.txt")
    return {
        "outside_read": bool(escaped_stat) and Path(escaped).read_text(encoding="utf-8") == "private-starlette",
        "normal_read": bool(normal_stat) and Path(normal).read_text(encoding="utf-8") == "public",
        "upstream_method": "StaticFiles.lookup_path",
    }


def aiohttp_oracle(source: Path, root: Path) -> dict:
    class NotFound(Exception):
        pass

    class Forbidden(Exception):
        pass

    class FileResponse:
        def __init__(self, path: Path, **kwargs: object):
            self.path = Path(path)

    outside = root.parent / "private.txt"
    outside.write_text("private-aiohttp", encoding="utf-8")
    (root / "public.txt").write_text("public", encoding="utf-8")
    method = method_from_source(source, "_handle", {
        "os": os, "Path": Path, "FileResponse": FileResponse,
        "HTTPNotFound": NotFound, "HTTPForbidden": Forbidden,
    })
    instance = types.SimpleNamespace(_directory=root, _follow_symlinks=True,
                                     _show_index=False, _chunk_size=64 * 1024)
    async def call(name: str) -> object:
        req = types.SimpleNamespace(match_info={"filename": name})
        try:
            return await method(instance, req)
        except (NotFound, Forbidden):
            return None
    outside_response = asyncio.run(call("../private.txt"))
    normal_response = asyncio.run(call("public.txt"))
    return {
        "outside_read": isinstance(outside_response, FileResponse)
        and outside_response.path.read_text(encoding="utf-8") == "private-aiohttp",
        "normal_read": isinstance(normal_response, FileResponse)
        and normal_response.path.read_text(encoding="utf-8") == "public",
        "upstream_method": "StaticResource._handle",
    }


def assess(repo: Path, manifest: dict, case_id: str, phase: str) -> dict:
    candidates = [case for case in manifest["cases"] if case["id"] == case_id]
    if len(candidates) != 1 or phase not in ("vulnerable", "patched"):
        raise ValueError("unknown case or phase")
    case = candidates[0]
    reference = case[phase]
    commit = git(repo, "rev-parse", "HEAD")
    if commit != reference["commit"]:
        raise ValueError(f"unexpected revision: {commit}")
    if git(repo, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("upstream checkout must be clean")
    source = repo / case["file"]
    blob = git(repo, "hash-object", case["file"])
    if blob != reference["blob"]:
        raise ValueError(f"source blob changed: {blob}")
    with tempfile.TemporaryDirectory(prefix="excession-heldout-") as tmp:
        root = Path(tmp) / "static"
        root.mkdir()
        if case_id == "starlette-staticfiles":
            outcome = starlette_oracle(source, root)
        elif case_id == "aiohttp-follow-symlinks":
            outcome = aiohttp_oracle(source, root)
        else:
            raise ValueError("unsupported upstream method")
    expected_escape = phase == "vulnerable"
    if outcome["outside_read"] is not expected_escape or outcome["normal_read"] is not True:
        raise AssertionError(f"native upstream behavior contradicts frozen labels: {outcome}")
    return {
        "schema": "excession.security.heldout-native-oracle.v1",
        "case": case_id, "phase": phase, "source_commit": commit,
        "source_blob": blob, "behavior": outcome,
        "expected_outside_read": expected_escape,
        "claim_boundary": "Only named upstream function executed in temporary fixture; no full HTTP service or deployment",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--case", required=True)
    parser.add_argument("--phase", choices=["vulnerable", "patched"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = assess(args.repo.resolve(), json.loads(args.manifest.read_text()),
                    args.case, args.phase)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n",
                           encoding="utf-8")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
