#!/usr/bin/env python3
"""Independent source-import inventory for pinned Excession Git fixtures.

The *expected* edges come from Git object contents and a separate resolver,
not from the adapter output or dependency table. The inventory defines only
literal, in-repository import edges, not runtime call-graph completeness.
"""
from __future__ import annotations

import ast
import json
import posixpath
import sqlite3
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _git_blob(repo: Path, commit: str, path: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), "show", f"{commit}:{path}"],
        capture_output=True, check=True, timeout=30,
    )
    return result.stdout.decode("utf-8")


def module_name(path: str) -> str:
    """Derive the indexer's path-oriented identifier without consulting SQL."""
    if path.endswith(".py"):
        stem = path[:-3]
    elif path.endswith((".mjs", ".cjs", ".ts")):
        stem = path
    elif path.endswith(".js"):
        stem = path[:-3]
    else:
        raise ValueError(f"not a supported source path: {path}")
    name = stem.replace("/", ".")
    for suffix in (".__init__", ".index"):
        if name.endswith(suffix):
            return name[: -len(suffix)]
    return name


def python_names(path_by_module: dict[str, str]) -> dict[str, str]:
    """Map importable names to indexed names, including the conventional src layout."""
    names = {name: name for name, path in path_by_module.items() if path.endswith(".py")}
    for name, path in path_by_module.items():
        if path.startswith("src/") and path.endswith(".py") and name.startswith("src."):
            alias = name[len("src."):]
            prior = names.setdefault(alias, name)
            if prior != name:
                raise ValueError(f"ambiguous importable Python module {alias}")
    return names


def _prefix_target(name: str, available: dict[str, str]) -> str | None:
    parts = name.split(".")
    matches = [available[".".join(parts[:n])]
               for n in range(1, len(parts) + 1)
               if ".".join(parts[:n]) in available]
    return matches[-1] if matches else None


def python_edges(
    source: dict[str, str], path_by_module: dict[str, str],
) -> tuple[set[tuple[str, str, int, str]], list[dict]]:
    """Inventory Python's declarative, in-tree import statements using CPython AST.

    Only an unambiguous indexed source module is a counted expected edge.
    Name-based dynamic imports and third-party/stdlib targets remain outside
    the completeness denominator.
    """
    available = python_names(path_by_module)
    edges: set[tuple[str, str, int, str]] = set()
    unresolved: list[dict] = []
    for module, path in sorted(path_by_module.items()):
        if not path.endswith(".py"):
            continue
        tree = ast.parse(source[path], filename=path)
        for stmt in ast.walk(tree):
            if isinstance(stmt, ast.Import):
                candidates = [alias.name for alias in stmt.names]
                kind = "import"
            elif isinstance(stmt, ast.ImportFrom):
                kind = "from"
                if stmt.level:
                    parent = module if path.endswith("/__init__.py") else module.rpartition(".")[0]
                    chunks = parent.split(".")
                    pop = stmt.level - 1
                    if pop >= len(chunks):
                        unresolved.append({"path": path, "line": stmt.lineno,
                                           "kind": "invalid-relative-level"})
                        continue
                    base = ".".join(chunks[:len(chunks) - pop])
                    if stmt.module:
                        base = f"{base}.{stmt.module}"
                else:
                    base = stmt.module or ""
                candidates = [base] + [
                    f"{base}.{alias.name}" if base else alias.name
                    for alias in stmt.names if alias.name != "*"
                ]
                if any(alias.name == "*" for alias in stmt.names):
                    unresolved.append({"path": path, "line": stmt.lineno,
                                       "kind": "wildcard-import"})
            else:
                if isinstance(stmt, ast.Call):
                    fn = stmt.func
                    if ((isinstance(fn, ast.Name) and fn.id in ("__import__", "import_module"))
                            or (isinstance(fn, ast.Attribute) and fn.attr == "import_module")):
                        unresolved.append({"path": path, "line": stmt.lineno,
                                           "kind": "dynamic-python-import"})
                continue
            for candidate in candidates:
                if not candidate:
                    continue
                matched = _prefix_target(candidate, available)
                if matched is not None:
                    edges.add((module, matched, stmt.lineno, kind))
                elif stmt.level if isinstance(stmt, ast.ImportFrom) else False:
                    unresolved.append({"path": path, "line": stmt.lineno,
                                       "kind": "unresolved-relative", "specifier": candidate})
    return edges, unresolved


def javascript_edges(
    source: dict[str, str], path_by_module: dict[str, str],
) -> tuple[set[tuple[str, str, int, str]], list[dict]]:
    """TypeScript syntax observations with an independent filesystem resolver.

    This is independent of the Rust resolution function; grammar coverage
    is not independent, because both parsers are built on TypeScript.
    """
    entries = [{"path": path, "content": source[path]} for path in sorted(path_by_module.values())
               if not path.endswith(".py")]
    if not entries:
        return set(), []
    proc = subprocess.run(
        ["node", str(HERE / "js_import_oracle.cjs")],
        input=json.dumps(entries), text=True, capture_output=True, check=False, timeout=90,
    )
    if proc.returncode:
        raise RuntimeError(f"JS independent import inventory failed: {proc.stderr[:1200]}")
    result = json.loads(proc.stdout)
    by_path = {path: name for name, path in path_by_module.items()}
    edges = {(by_path[row["source"]], by_path[row["target"]],
              row["line"], row["syntax"]) for row in result["edges"]}
    return edges, result["unresolved"]


def verify_dependencies(
    db: Path, repo: Path, commit: str, git_blobs: dict[str, tuple[str, str]],
) -> dict:
    with sqlite3.connect(db) as con:
        db_modules = con.execute("SELECT name,path FROM module ORDER BY name").fetchall()
        recorded = {(source, target, line, syntax) for source, target, line, syntax
                    in con.execute(
                        "SELECT source_name,target_name,line,syntax FROM dependency"
                    )}
    path_by_module = dict(db_modules)
    if len(path_by_module) != len(db_modules):
        raise ValueError("duplicate source modules in SQLite")
    if len(set(path_by_module.values())) != len(path_by_module):
        raise ValueError("one source path has multiple module names")
    for module, path in db_modules:
        if path not in git_blobs or module_name(path) != module:
            raise ValueError(f"module catalog disagrees with independent Git identity: {module}")
    source = {path: _git_blob(repo, commit, path) for path in path_by_module.values()}
    python, python_unresolved = python_edges(source, path_by_module)
    javascript, js_unresolved = javascript_edges(source, path_by_module)
    expected = python | javascript
    missing = sorted(expected - recorded)
    unexpected = sorted(recorded - expected)
    unresolved = python_unresolved + js_unresolved
    counts = {
        "module_count": len(db_modules),
        "expected_in_scope_edges": len(expected),
        "indexed_edges": len(recorded),
        "matched_edges": len(expected & recorded),
        "missing_edges": len(missing),
        "unexpected_edges": len(unexpected),
        "not_proven_import_sites": len(unresolved),
    }
    return {
        "status": "matched" if not missing and not unexpected else "mismatch",
        "counts": counts,
        "missing": [dict(zip(("source", "target", "line", "syntax"), row)) for row in missing[:100]],
        "unexpected": [dict(zip(("source", "target", "line", "syntax"), row)) for row in unexpected[:100]],
        "unresolved_examples": unresolved[:40],
        "claim": ("Exact agreement for statically resolved in-index import edges only; "
                  "no claim about dynamic import results, external packages, conditional "
                  "runtime execution, package initialization, or parser completeness."),
    }
