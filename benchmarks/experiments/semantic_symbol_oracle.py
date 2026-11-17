"""Conservative source-only Python call and type-reference oracle.

Works on pinned Git blobs, without importing or executing tested modules.
Only lexically unshadowed imported top-level functions/classes are labeled.
Methods, instance dispatch, monkey-patching, re-exports, reflection and
runtime resolution are explicitly UNKNOWN, not inferred dependencies.
"""
from __future__ import annotations

import ast
from collections import Counter
from pathlib import Path


def module_name(path: str) -> str:
    p = path.removeprefix("src/").removesuffix(".py").replace("/", ".")
    return p[:-len(".__init__")] if p.endswith(".__init__") else p


def independent_python_sites(repo: Path, sources: set[str], blobs: dict,
                             read_blob) -> dict:
    modules = {module_name(p): p for p in sources}
    if len(modules) != len(sources):
        raise ValueError("ambiguous Python source-module identities")
    trees = {p: ast.parse(read_blob(repo, p, blobs).decode("utf-8-sig"), filename=p)
             for p in sorted(sources)}
    definitions = {
        p: {n.name: ("class" if isinstance(n, ast.ClassDef) else "function")
            for n in tree.body if isinstance(n, (ast.FunctionDef,
                                                  ast.AsyncFunctionDef, ast.ClassDef))}
        for p, tree in trees.items()
    }
    sites: set[tuple[str, str, int, str, str]] = set()
    unsupported = Counter()
    for p, tree in trees.items():
        own = module_name(p)
        package = own if p.endswith("/__init__.py") else own.rpartition(".")[0]
        imports = {}
        for stmt in tree.body:
            if isinstance(stmt, ast.ImportFrom):
                parts = package.split(".") if package else []
                if stmt.level > len(parts) + 1:
                    unsupported["unresolved_relative_import"] += 1
                    continue
                base = ".".join(parts[:len(parts) - stmt.level + 1]) \
                    if stmt.level else ""
                module = ".".join(x for x in (base, stmt.module or "") if x)
                for alias in stmt.names:
                    local = alias.asname or alias.name
                    target = modules.get(module)
                    if target and alias.name in definitions[target]:
                        imports[local] = (target, alias.name)
                    elif not stmt.module:
                        child = modules.get(module + "." + alias.name)
                        if child:
                            imports[local] = (child, None)
            elif isinstance(stmt, ast.Import):
                for alias in stmt.names:
                    target = modules.get(alias.name)
                    if target and (alias.asname or "." not in alias.name):
                        imports[alias.asname or alias.name] = (target, None)
        # A Python name rebound anywhere within the source file cannot be
        # proved to denote the imported symbol without flow analysis.
        unsafe = set()
        for n in ast.walk(tree):
            if isinstance(n, ast.Name) and isinstance(n.ctx, (ast.Store, ast.Del)):
                unsafe.add(n.id)
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                unsafe.update(a.arg for a in (n.args.posonlyargs + n.args.args +
                                                  n.args.kwonlyargs))
                if n.args.vararg:
                    unsafe.add(n.args.vararg.arg)
                if n.args.kwarg:
                    unsafe.add(n.args.kwarg.arg)
            if isinstance(n, (ast.Global, ast.Nonlocal)):
                unsafe.update(n.names)
        safe_imports = {name: pair for name, pair in imports.items()
                        if name not in unsafe}
        unsupported["shadowed_import_aliases"] += len(imports) - len(safe_imports)

        def lookup(expr):
            if isinstance(expr, ast.Name):
                resolved = safe_imports.get(expr.id)
                if resolved and resolved[1] is not None:
                    dest, symbol = resolved
                    return dest, symbol, definitions[dest][symbol]
            if isinstance(expr, ast.Attribute) and isinstance(expr.value, ast.Name):
                resolved = safe_imports.get(expr.value.id)
                if resolved and resolved[1] is None:
                    dest = resolved[0]
                    if expr.attr in definitions[dest]:
                        return dest, expr.attr, definitions[dest][expr.attr]
            return None

        def emit(expr, kind):
            resolved = lookup(expr)
            if resolved:
                dest, symbol, defined_kind = resolved
                if dest != p and (kind != "type" or defined_kind == "class"):
                    sites.add((p, dest, expr.lineno, kind, symbol))
                    return True
            unsupported["unresolved_or_dynamic_" + kind] += 1
            return False

        def annotations(expr):
            if expr is None:
                return
            for n in ast.walk(expr):
                if isinstance(n, ast.Name) or (
                    isinstance(n, ast.Attribute) and isinstance(n.value, ast.Name)
                ):
                    emit(n, "type")

        for n in ast.walk(tree):
            if isinstance(n, ast.Call):
                if not isinstance(n.func, (ast.Name, ast.Attribute)) \
                        or (isinstance(n.func, ast.Attribute) and
                            not isinstance(n.func.value, ast.Name)):
                    unsupported["dynamic_call"] += 1
                else:
                    emit(n.func, "call")
            elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                annotations(n.returns)
                for arg in n.args.posonlyargs + n.args.args + n.args.kwonlyargs:
                    annotations(arg.annotation)
                if n.args.vararg:
                    annotations(n.args.vararg.annotation)
                if n.args.kwarg:
                    annotations(n.args.kwarg.annotation)
            elif isinstance(n, ast.AnnAssign):
                annotations(n.annotation)
            elif isinstance(n, ast.ClassDef):
                for base in n.bases:
                    annotations(base)

    ordered = sorted(sites)
    return {
        "schema": "excession.independent-python-symbol-sites.v1",
        "oracle": "CPython_ast_lexically_unshadowed_cross_file_function_class",
        "sites": [{"source": a, "target": b, "line": line, "kind": kind,
                   "symbol": sym} for a, b, line, kind, sym in ordered],
        "counts": dict(Counter(row[3] for row in ordered)),
        "unsupported": dict(unsupported),
        "runtime_dispatch_proven": False,
        "cross_language_links_proven": False,
    }


def score_sites(oracle: dict, full: list, before: list, after: list,
                focus: str) -> dict:
    """Count exact source+target+1-based line observations for labeled sites.

    A matching symbol line supports source-level correspondence only.
    Matching the line does not prove which same-line expression was indexed,
    runtime dispatch or the correctness of its callable type.
    """
    sites = [s for s in oracle["sites"] if focus == s["source"]]
    def ids(rows):
        return {
            (r["source"]["path"], r["target"]["path"],
             r["native_observation"]["line"])
            for r in rows
            if r["kind"] == "SCIP_REFERENCE"
            and isinstance(r.get("native_observation", {}).get("line"), int)
        }
    native_all, old, new = ids(full), ids(before), ids(after)
    result = {"focus": focus, "labeled_sites": len(sites), "kinds": {}}
    for kind in ("call", "type"):
        group = [s for s in sites if s["kind"] == kind]
        def hits(refs):
            return sum((s["source"], s["target"], s["line"]) in refs for s in group)
        result["kinds"][kind] = {
            "oracle_sites": len(group),
            "full_native_hits": hits(native_all),
            "old_bounded_hits": hits(old),
            "new_bounded_hits": hits(new),
        }
    if sum(row["oracle_sites"] for row in result["kinds"].values()) != len(sites):
        raise ValueError("unsupported source-site kind in independent oracle")
    for values in result["kinds"].values():
        if not (0 <= values["old_bounded_hits"] <= values["full_native_hits"] <= values["oracle_sites"] and
                0 <= values["new_bounded_hits"] <= values["full_native_hits"]):
            raise ValueError("bounded product claims source evidence absent from full graph")
    return result
