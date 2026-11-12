"""Source-native positive anchors, independently checked against pinned Git.

The native SCIP oracle proves accurate preservation of the index. This second,
small oracle checks manually selected import declarations from *source syntax*,
independently of the indexer and Excession's projection. Only these anchors
are corroborated. It is not a general import resolver or a source-equivalence
proof, and it must never be extrapolated to all observed SCIP edges.
"""
from __future__ import annotations

import ast
import json
import posixpath
import re
from pathlib import Path
import subprocess

from concepts_scip_oracle import native_references


def git_source(repo: Path, commit: str, path: str) -> str:
    if (not path or path.startswith("/") or "\\" in path or
            any(part in {"", ".", ".."} for part in path.split("/"))):
        raise ValueError("unsafe pinned source-oracle path")
    result = subprocess.run(
        ["git", "-C", str(repo), "show", f"{commit}:{path}"],
        capture_output=True, check=False, timeout=20,
    )
    if result.returncode:
        raise ValueError(f"source-oracle source missing from Git: {path}")
    return result.stdout.decode("utf-8")


def source_declaration(case: dict, source: str) -> bool:
    kind = case["kind"]
    target = case["target"]
    path = case["source"]
    if kind == "RELATIVE_PYTHON_IMPORT":
        module = case["module"]
        if posixpath.normpath(posixpath.join(posixpath.dirname(path), module + ".py")) != target:
            raise ValueError("Python oracle target is inconsistent with relative module name")
        tree = ast.parse(source, filename=path)
        return any(
            isinstance(node, ast.ImportFrom) and node.level == 1 and
            node.module == module for node in ast.walk(tree)
        )
    if kind == "ESM_IMPORT":
        specifier = case["specifier"]
        if posixpath.normpath(posixpath.join(posixpath.dirname(path), specifier)) != target:
            raise ValueError("ECMAScript oracle target is inconsistent with import specifier")
        for match in re.finditer(
            r"^\s*import\s+[A-Za-z_$][\w$]*\s+from\s+['\"]([^'\"]+)['\"]\s*;",
            source, flags=re.MULTILINE,
        ):
            if match.group(1) == specifier:
                return True
        return False
    if kind == "RUST_CRATE_USE":
        module = case["module"]
        if posixpath.normpath(posixpath.join(posixpath.dirname(path), module + ".rs")) != target:
            raise ValueError("Rust oracle target is inconsistent with crate module name")
        return any(
            match.group(1) == module
            for match in re.finditer(
                r"^\s*use\s+crate::([A-Za-z_][A-Za-z_0-9]*)\s*;",
                source, flags=re.MULTILINE,
            )
        )
    raise ValueError(f"unsupported source-oracle declaration kind: {kind}")


def verify_anchors(
    repo: Path,
    commit: str,
    decoded: dict,
    manifest: dict,
    fixture: str,
    language: str,
    source_prefix: str | None = None,
) -> dict:
    if manifest.get("schema") != "excession.concepts-source-anchors.v1":
        raise ValueError("unrecognized source oracle manifest")
    edges, _, _ = native_references(decoded, source_prefix)
    pairs = {(a, b) for a, b, _, _ in edges}
    cases = [c for c in manifest.get("cases", [])
             if c.get("fixture") == fixture and c.get("language") == language]
    seen = set()
    for case in cases:
        key = (case["source"], case["target"])
        if key in seen:
            raise ValueError("duplicate manually frozen source anchor")
        seen.add(key)
        text = git_source(repo, commit, case["source"])
        git_source(repo, commit, case["target"])
        if not source_declaration(case, text):
            raise ValueError(f"independent source import is absent: {key!r}")
        if key not in pairs:
            raise ValueError(f"native SCIP missed independently source-verified import: {key!r}")
    return {
        "schema": "excession.concepts-source-anchors-result.v1",
        "fixture": fixture,
        "language": language,
        "source_commit": commit,
        "verified_source_imports": len(cases),
        "status": "FROZEN_POSITIVE_SOURCE_ANCHORS_MATCH" if cases else "NO_FROZEN_SOURCE_ANCHORS",
        "general_semantic_accuracy_proven": False,
    }


def load_manifest(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))
