#!/usr/bin/env python3
"""Independent *static import* recall against real Git-bound SCIP relations.

Two oracles read pinned, clean source, not Excession output: CPython ast and
TypeScript's compiler AST/module resolver. The original indexer is a separate
producer. Direct syntactic import pairs are the ground-truth *subset*; native
SCIP references outside that subset are not deemed erroneous dependencies.
"""
from __future__ import annotations

import argparse
import ast
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from real_fixtures import checkout
from scip_fixture_migration import (
    EXTENSIONS, generate_index, tracked_sources,
)
from semantic_symbol_oracle import independent_python_sites, score_sites

ROOT = Path(__file__).resolve().parents[2]
# Fixed before observing any run from this new benchmark. These all belong to
# independently maintained repositories, not Excession or an agent-written repo.
FOCI = {
    "h11": ("h11/_connection.py", "h11/_events.py", "h11/_headers.py"),
    "click": ("src/click/core.py", "src/click/parser.py", "src/click/decorators.py"),
    "requests": ("src/requests/sessions.py", "src/requests/models.py",
                 "src/requests/adapters.py"),
    "axios": ("lib/core/Axios.js", "lib/adapters/http.js", "lib/utils.js"),
}
LIMIT = 16
LANGUAGES = {"h11": "python", "click": "python",
             "requests": "python", "axios": "typescript"}


def command(argv: list[object], *, cwd: Path | None = None,
            timeout: int = 360) -> bytes:
    proc = subprocess.run([str(x) for x in argv], cwd=cwd,
                          capture_output=True, timeout=timeout, check=False)
    if proc.returncode:
        raise RuntimeError(f"{argv[0]} exited {proc.returncode}: "
                           f"{proc.stderr.decode(errors='replace')[:1800]}")
    return proc.stdout


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def inventory(repo: Path, commit: str) -> dict[str, tuple[str, str]]:
    source = command(["git", "-C", repo, "ls-tree", "-rz", commit])
    items = {}
    for row in source.split(b"\0"):
        if not row:
            continue
        header, path = row.split(b"\t", 1)
        mode, kind, blob = header.decode("ascii").split()
        if kind == "blob" and mode in {"100644", "100755"}:
            name = path.decode("utf-8")
            if name in items:
                raise ValueError("duplicate Git source identity")
            items[name] = (mode, blob)
    return items


def checked_source(repo: Path, name: str, items: dict) -> bytes:
    if name not in items:
        raise ValueError(f"source path not in exact Git tree: {name}")
    path = repo / name
    blob = command(["git", "-C", repo, "hash-object", "--", path]).decode().strip()
    if blob != items[name][1]:
        raise ValueError(f"dirty or substituted checked-out source: {name}")
    return path.read_bytes()


def python_imports(repo: Path, files: set[str], items: dict) -> dict:
    """Conservative direct module resolution, no imports executed.

    'src/' is the canonical Python package layout for these pinned fixtures.
    Unresolved and dynamic imports remain excluded. Attribute-only FromImport
    names are not treated as files without a literal module resolution.
    """
    modules = {}
    for name in sorted(files):
        relative = name[4:] if name.startswith("src/") else name
        module = relative.removesuffix(".py").replace("/", ".")
        if module.endswith(".__init__"):
            module = module[:-len(".__init__")]
        if module in modules:
            raise ValueError(f"ambiguous Python module identity: {module}")
        modules[module] = name
    pairs: set[tuple[str, str]] = set()
    unresolved = 0
    import_sites = 0
    dynamic = 0
    for name in sorted(files):
        body = checked_source(repo, name, items)
        syntax = ast.parse(body.decode("utf-8-sig"), filename=name)
        relative = name[4:] if name.startswith("src/") else name
        own_module = relative.removesuffix(".py").replace("/", ".")
        package = (own_module[:-len(".__init__")] if
                   own_module.endswith(".__init__") else
                   own_module.rpartition(".")[0])
        for node in ast.walk(syntax):
            if isinstance(node, (ast.Import, ast.ImportFrom)):
                import_sites += 1
                targets = []
                if isinstance(node, ast.Import):
                    targets = [item.name for item in node.names]
                elif node.level:
                    parts = package.split(".") if package else []
                    if node.level - 1 >= len(parts) and node.level > 1:
                        unresolved += 1
                        continue
                    anchor = ".".join(parts[:len(parts) - node.level + 1])
                    module = ".".join(x for x in (anchor, node.module or "") if x)
                    if node.module:
                        targets = [
                            module + "." + item.name
                            if module + "." + item.name in modules else module
                            for item in node.names
                        ]
                    else:
                        targets = [module + "." + item.name for item in node.names]
                else:
                    module = node.module or ""
                    targets = [
                        module + "." + item.name
                        if module + "." + item.name in modules else module
                        for item in node.names
                    ]
                for target in targets:
                    resolved = modules.get(target)
                    if resolved and resolved != name:
                        pairs.add((name, resolved))
                    elif not resolved:
                        unresolved += 1
            elif isinstance(node, ast.Call) and isinstance(node.func, ast.Name) \
                    and node.func.id == "__import__":
                dynamic += 1
    return {"schema": "excession.independent-python-import-oracle.v1",
            "parser": "CPython-ast-static-module-imports",
            "tracked_files": len(files), "pairs": sorted(pairs),
            "import_sites": import_sites, "unresolved_module_targets": unresolved,
            "dynamic_or_nonliteral_excluded": dynamic,
            "semantic_runtime_dependencies_proven": False}


def typescript_imports(repo: Path, files: set[str], items: dict,
                       node_oracle: Path) -> dict:
    for name in sorted(files):
        checked_source(repo, name, items)
    with tempfile.TemporaryDirectory(prefix="excession-ts-import-inventory-") as tmp:
        manifest = Path(tmp) / "sources.json"
        manifest.write_text(json.dumps(sorted(files)))
        report = json.loads(command(
            ["node", node_oracle.resolve(), repo.resolve(), manifest], timeout=180
        ))
    if report.get("schema") != "excession.independent-ts-import-oracle.v1":
        raise ValueError("unsupported independent TypeScript import oracle")
    return report


def verified_pairs(report: dict, repo: Path, commit: str,
                   tree: str, items: dict) -> list[dict]:
    if (report.get("source_commit"), report.get("source_tree")) != (commit, tree):
        raise ValueError("native graph belongs to a different Git revision")
    results = report.get("relationships")
    if not isinstance(results, list):
        raise ValueError("native graph has no relationship ledger")
    for relation in results:
        for endpoint in ("source", "target"):
            src = relation[endpoint]
            path = src["path"]
            if (src.get("source_commit") != commit or src.get("source_tree") != tree
                    or (src.get("mode"), src.get("blob")) != items.get(path)):
                raise ValueError(f"stale or invented Git relationship endpoint: {path}")
        if relation.get("kind") == "SCIP_REFERENCE":
            evidence = relation.get("evidence")
            native = relation.get("native_observation", {})
            if (not isinstance(evidence, list) or not evidence
                    or any(e.get("assessment") != "OBSERVED" for e in evidence)
                    or not isinstance(native.get("snapshot_id"), str)
                    or not isinstance(native.get("observation_id"), str)):
                raise ValueError("native reference lacks original observed producer evidence")
    return results


def relation_pair(row: dict) -> tuple[str, str]:
    return row["source"]["path"], row["target"]["path"]


def native_pairs(rows: list[dict]) -> set[tuple[str, str]]:
    return {relation_pair(row) for row in rows if row["kind"] == "SCIP_REFERENCE"}


def independent_priority(rows: list[dict], limit: int) -> list[dict]:
    """Independent small oracle of the documented distinct-native-first policy.

    The evaluation checks the native executable returns this exact bounded
    sequence. It never supplies edge labels or predicted results to Excession.
    """
    seen = set()
    distinct, repeats, candidates = [], [], []
    for row in rows:
        if row["kind"] == "SCIP_REFERENCE":
            pair = relation_pair(row)
            if pair in seen:
                repeats.append(row)
            else:
                distinct.append(row)
                seen.add(pair)
        else:
            candidates.append(row)
    return (distinct + repeats + candidates)[:limit]


def score(full: list[dict], returned: list[dict], oracle_pairs: set,
          focus: str, limit: int) -> dict:
    scoped = [row for row in full if focus in relation_pair(row)]
    if len(returned) != min(limit, len(scoped)):
        raise ValueError(f"bounded native context differs from total for {focus}")
    if independent_priority(scoped, limit) != returned:
        raise ValueError(f"native source pair ordering disagrees with independent policy for {focus}")
    gold = {pair for pair in oracle_pairs if focus in pair}
    baseline = scoped[:limit]
    old_native, new_native = native_pairs(baseline), native_pairs(returned)
    old_confirmed, new_confirmed = gold & old_native, gold & new_native
    return {
        "focus": focus,
        "oracle_import_pairs": len(gold),
        "full_native_observed_pairs": len(native_pairs(scoped)),
        "old_observed_pairs_retained": len(old_native),
        "new_observed_pairs_retained": len(new_native),
        "old_import_confirmed": len(old_confirmed),
        "new_import_confirmed": len(new_confirmed),
        "old_import_recall": len(old_confirmed) / len(gold) if gold else None,
        "new_import_recall": len(new_confirmed) / len(gold) if gold else None,
        "old_unclassified_native_pairs": len(old_native - gold),
        "new_unclassified_native_pairs": len(new_native - gold),
        "returned_relationships": len(returned),
        "total_touching_relationships": len(scoped),
        # A native SCIP edge that is not a static import is not established
        # false: it can denote another legitimate cross-file symbol reference.
        "unclassified_native_edges_are_false_positives": False,
    }


def evaluate(case: dict, args, work: Path, evidence: Path) -> dict:
    name = case["name"]
    language = LANGUAGES[name]
    repo = work / f"{name}-git"
    commit, tree = checkout(case, repo)
    tracked = inventory(repo, commit)
    roots = case["roots"].split(",")
    included = tracked_sources(repo, commit, roots, language)
    if not included or any(f not in included for f in FOCI[name]):
        raise ValueError(f"missing preregistered {name} focus source")
    if command(["git", "-C", repo, "status", "--porcelain", "--untracked-files=all"]).strip():
        raise ValueError("unexpected dirty upstream fixture")
    oracle = (python_imports(repo, included, tracked) if language == "python"
              else typescript_imports(repo, included, tracked, args.node_oracle))
    gold = {tuple(pair) for pair in oracle["pairs"]}
    semantic = None
    if args.semantic:
        if language == "python":
            semantic = independent_python_sites(repo, included, tracked, checked_source)
        else:
            with tempfile.TemporaryDirectory(prefix="excession-independent-symbols-") as temp:
                manifest = Path(temp) / "sources.json"
                manifest.write_text(json.dumps(sorted(included)))
                semantic = json.loads(command([
                    "node", args.ts_symbol_oracle, repo.resolve(), manifest
                ]))
            if semantic.get("schema") != "excession.independent-typescript-symbol-sites.v1":
                raise ValueError("unrecognized TypeScript independent symbol oracle")
        for site in semantic["sites"]:
            if (site["source"] not in included or site["target"] not in included
                    or site["kind"] not in ("call", "type") or
                    not isinstance(site["line"], int) or site["line"] < 1):
                raise ValueError("source semantic oracle produced invalid site identity")
        semantic["source_commit"] = commit
        semantic["source_tree"] = tree
        (evidence / f"{name}-{language}.symbol-oracle.json").write_text(
            json.dumps(semantic, indent=2, sort_keys=True) + "\n"
        )
    if not gold:
        raise ValueError(f"no independently identified static import pairs: {name}")

    stage = work / f"{name}-indexer"
    shutil.copytree(repo, stage)
    # Native SCIP's project-root URI belongs to this disposable checkout.
    # Rebind Excession to the *same* exact Git tree and source root rather than
    # silently changing the SCIP URI or weakening source provenance checks.
    if (command(["git", "-C", stage, "rev-parse", "HEAD"]).decode().strip() != commit
            or command(["git", "-C", stage, "rev-parse", "HEAD^{tree}"]).decode().strip() != tree):
        raise ValueError("SCIP staging checkout differs from independently parsed Git source")
    output = evidence / f"{name}-{language}.decoded.json"
    started = time.perf_counter()
    decoded = generate_index(
        stage, output,
        args.python_indexer if language == "python" else args.typescript_indexer,
        args.scip, language, roots, name, included,
    )
    generation_s = round(time.perf_counter() - started, 3)
    if len(decoded) > 4 * 1024 * 1024:
        raise ValueError(f"native evidence exceeds existing 4MiB capacity: {name}")
    db = work / f"{name}.sqlite"
    imported = json.loads(command([
        args.bin, "refactor", "scip-ingest", "--repo", stage,
        "--rev", commit, "--index", output, "--db", db
    ]))
    if imported.get("source_commit") != commit or imported.get("source_tree") != tree:
        raise ValueError("SCIP producer import does not bind original Git source")
    if imported.get("matched_cross_file_references", 0) <= 0:
        raise ValueError("SCIP index has no independently source-bound observations")

    full_report = json.loads(command([
        args.bin, "analyze", stage, "--rev", commit, "--roots",
        ",".join(roots), "--db", db, "--json"
    ]))
    full = verified_pairs(full_report, repo, commit, tree, tracked)
    cases = []
    for focus in FOCI[name]:
        context = json.loads(command([
            args.bin, "agent", "context", "--repo", stage, "--rev", commit,
            "--focus", focus, "--roots", ",".join(roots),
            "--db", db, "--limit", LIMIT
        ], timeout=240))
        if context.get("schema") != "excession.agent-context.v1":
            raise ValueError("unexpected native bounded context format")
        rows = verified_pairs(context, repo, commit, tree, tracked)
        finding = score(full, rows, gold, focus, LIMIT)
        if semantic is not None:
            scoped = [r for r in full if focus in relation_pair(r)]
            finding["semantic_site_recall"] = score_sites(
                semantic, full, scoped[:LIMIT], rows, focus
            )
        if (context.get("relationship_count") != finding["total_touching_relationships"]
                or context.get("returned_observed_reference_count") !=
                sum(row["kind"] == "SCIP_REFERENCE" for row in rows)
                or context.get("observed_reference_count") !=
                sum(row["kind"] == "SCIP_REFERENCE" for row in full
                    if focus in relation_pair(row))):
            raise ValueError("native bounded output counters disagree with independent replay")
        cases.append(finding)

    if command(["git", "-C", repo, "status", "--porcelain", "--untracked-files=all"]).strip():
        raise ValueError("oracle or native code mutated the pinned original checkout")
    receipt = {
        "fixture": name, "language": language,
        "repository": case.get("repository", "python-hyper/h11"),
        "source_commit": commit, "source_tree": tree,
        "scip_index_sha256": digest(decoded),
        "scip_index_bytes": len(decoded),
        "scip_native_observed_references": imported["matched_cross_file_references"],
        "scip_unresolved_references": imported["unresolved_references"],
        "scip_snapshot_id": imported["snapshot_id"],
        "independent_oracle": {
            "parser": oracle["parser"],
            "source_count": oracle["tracked_files"],
            "import_site_count": oracle["import_sites"],
            "static_import_pairs": len(gold),
            "unknown_or_external_imports": oracle.get("unresolved_module_targets",
                                                     oracle.get("unresolved_relative", 0)),
            "dynamic_imports_excluded": oracle["dynamic_or_nonliteral_excluded"],
        },
        "index_generation_seconds": generation_s,
        "semantic_site_oracle": (
            {"schema": semantic["schema"], "oracle": semantic["oracle"],
             "labeled_site_count": len(semantic["sites"]),
             "counts": semantic["counts"],
             "unsupported": semantic["unsupported"],
             "cross_language_links_proven": False,
             "runtime_dispatch_proven": False,
             "shared_compiler_family_with_indexer": (
                 language == "typescript"
             )}
            if semantic is not None else None
        ),
        "cases": cases,
    }
    (evidence / f"{name}-oracle-and-product.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    )
    return receipt


def summarize(fixtures: list[dict]) -> dict:
    cases = [case for item in fixtures for case in item["cases"]]
    total_gold = sum(row["oracle_import_pairs"] for row in cases)
    old = sum(row["old_import_confirmed"] for row in cases)
    new = sum(row["new_import_confirmed"] for row in cases)
    if not total_gold:
        raise ValueError("all independent source-oracle scopes were empty")
    report = {
        "schema": "excession.independent-import-discovery.v1",
        "meaning": "INDEPENDENT_STATIC_IMPORT_PAIR_RECALL_ONLY",
        "sources_are_independently_git_pinned": True,
        "languages": sorted(set(item["language"] for item in fixtures)),
        "repositories": len(fixtures), "evaluated_focus_files": len(cases),
        "per_focus_import_opportunities": total_gold,
        "old_import_confirmed": old,
        "new_import_confirmed": new,
        "old_static_import_pair_recall": old / total_gold,
        "new_static_import_pair_recall": new / total_gold,
        "native_references_outside_static_import_oracle_are_not_false_positives": True,
        "runtime_dependency_completeness_proven": False,
        "fixtures": fixtures,
    }
    if all(item.get("semantic_site_oracle") is not None for item in fixtures):
        per_kind = {}
        for kind in ("call", "type"):
            totals = {"oracle_sites": 0, "full_native_hits": 0,
                      "old_bounded_hits": 0, "new_bounded_hits": 0}
            for row in cases:
                current = row["semantic_site_recall"]["kinds"][kind]
                for metric in totals:
                    totals[metric] += current[metric]
            totals["full_native_recall"] = (
                totals["full_native_hits"] / totals["oracle_sites"]
                if totals["oracle_sites"] else None
            )
            totals["old_bounded_recall"] = (
                totals["old_bounded_hits"] / totals["oracle_sites"]
                if totals["oracle_sites"] else None
            )
            totals["new_bounded_recall"] = (
                totals["new_bounded_hits"] / totals["oracle_sites"]
                if totals["oracle_sites"] else None
            )
            per_kind[kind] = totals
        report["semantic_sites"] = {
            "schema": "excession.conservative-symbol-site-recall.v1",
            "unit": "source_target_1_based_line",
            "kinds": per_kind,
            "independent_semantics_complete": False,
            "dynamic_dispatch_proven": False,
            "cross_language_links_proven": False,
            "typescript_compiler_family_shared_with_scip_indexer": True,
        }
    return report


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--evidence-dir", required=True, type=Path)
    parser.add_argument("--scip", required=True, type=Path)
    parser.add_argument("--python-indexer", required=True, type=Path)
    parser.add_argument("--typescript-indexer", required=True, type=Path)
    parser.add_argument("--node-oracle", type=Path,
                        default=ROOT / "benchmarks/experiments/ts_import_oracle.cjs")
    parser.add_argument("--ts-symbol-oracle", type=Path,
                        default=ROOT / "benchmarks/experiments/ts_symbol_oracle.cjs")
    parser.add_argument("--semantic", action="store_true")
    parser.add_argument("--fixture", action="append", choices=tuple(FOCI))
    args = parser.parse_args()
    args.bin = args.bin.resolve(strict=True)
    args.scip = args.scip.resolve(strict=True)
    args.python_indexer = args.python_indexer.resolve(strict=True)
    args.typescript_indexer = args.typescript_indexer.resolve(strict=True)
    args.node_oracle = args.node_oracle.resolve(strict=True)
    if args.semantic:
        args.ts_symbol_oracle = args.ts_symbol_oracle.resolve(strict=True)
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    args.evidence_dir = args.evidence_dir.resolve()
    selected = args.fixture or list(FOCI)
    declared = json.loads((ROOT / "benchmarks/fixtures/real-repositories.json").read_text())
    fixture_map = {item["name"]: item for item in declared["fixtures"]}
    if declared["schema"] != "excession.real-projection-fixtures.v1":
        raise ValueError("unexpected fixed source fixture manifest")
    if any(name not in fixture_map for name in selected) or len(set(selected)) != len(selected):
        raise ValueError("fixture set not unique or unknown")
    output = []
    with tempfile.TemporaryDirectory(prefix="excession-independent-import-recall-") as tmp:
        for name in selected:
            item = evaluate(fixture_map[name], args, Path(tmp), args.evidence_dir)
            output.append(item)
            print(f"{name}: {len(item['cases'])} native contexts, "
                  f"{item['independent_oracle']['static_import_pairs']} static import pairs",
                  flush=True)
    result = summarize(output)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "fixtures"},
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
