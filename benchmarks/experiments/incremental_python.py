#!/usr/bin/env python3
"""A falsifiable incremental-invalidation experiment, not a second product indexer.

Reuse the *existing CPython AST adapter's* immutable observations only when its
version, the importable module catalog, the source blob and mode all match.
Compare every output to a full adapter pass and a separate Git/AST import oracle.
This measures the potential optimization before implementing it in Rust/SQLite.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import statistics
import subprocess
import sys
import time
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ADAPTER_FILE = ROOT / "core/adapters/python_ast.py"
sys.path.insert(0, str(HERE))
from dependency_oracle import module_name, python_edges  # noqa: E402


def read_adapter():
    spec = importlib.util.spec_from_file_location("excession_python_ast", ADAPTER_FILE)
    if spec is None or spec.loader is None:
        raise RuntimeError("Python AST adapter unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(*args: object, input_data: bytes | None = None) -> bytes:
    argv = [str(arg) for arg in args]
    p = subprocess.run(argv, input=input_data, capture_output=True, timeout=120)
    if p.returncode:
        raise RuntimeError(f"{argv[:6]!r}: {p.stderr.decode(errors='replace')[:1200]}")
    return p.stdout


def git(repo: Path, *args: str) -> str:
    return run("git", "-C", repo, *args).decode().strip()


def inventory(repo: Path, revision: str, roots: tuple[str, ...]) -> dict[str, tuple[str, str]]:
    raw = run("git", "-C", repo, "ls-tree", "-r", "-z", revision)
    result: dict[str, tuple[str, str]] = {}
    for record in raw.split(b"\0"):
        if not record:
            continue
        header, encoded_path = record.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split()
        path = encoded_path.decode("utf-8")
        if (kind == "blob" and mode in ("100644", "100755")
                and path.endswith(".py")
                and any(path.startswith(f"{root}/") for root in roots)):
            result[path] = (oid, mode)
    return dict(sorted(result.items()))


def blobs(repo: Path, paths: dict[str, tuple[str, str]]) -> dict[str, str]:
    """One Git cat-file batch, matching the production BlobReader approach."""
    if not paths:
        return {}
    oids = [row[0] for row in paths.values()]
    raw = run("git", "-C", repo, "cat-file", "--batch",
              input_data=("".join(f"{oid}\n" for oid in oids)).encode())
    offset = 0
    contents = {}
    for path, (oid, _) in paths.items():
        end = raw.find(b"\n", offset)
        if end < 0:
            raise ValueError("truncated Git blob header")
        header = raw[offset:end].decode("ascii").split()
        if len(header) != 3 or header[0] != oid or header[1] != "blob":
            raise ValueError("Git batch returned a different object")
        length = int(header[2])
        data = raw[end + 1:end + 1 + length]
        if len(data) != length or raw[end + 1 + length:end + 2 + length] != b"\n":
            raise ValueError("truncated Git batch blob")
        contents[path] = data.decode("utf-8")
        offset = end + 2 + length
    if offset != len(raw):
        raise ValueError("unexpected trailing Git batch data")
    return contents


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
    ).encode("utf-8")).hexdigest()


def catalog(paths: dict[str, tuple[str, str]]) -> list[dict[str, str]]:
    rows = [{"name": module_name(path), "path": path} for path in paths]
    if len({r["name"] for r in rows}) != len(rows):
        raise ValueError("duplicate module identity")
    return rows


def producer(roots: tuple[str, ...]) -> str:
    return digest({
        "adapter_sha256": hashlib.sha256(ADAPTER_FILE.read_bytes()).hexdigest(),
        "python": list(sys.version_info[:3]),
        "roots": roots,
        "experiment": "ast-module-catalog-v1",
    })


def full_scan(
    repo: Path, paths: dict[str, tuple[str, str]], adapter: Any,
) -> tuple[dict[str, dict], int]:
    records = catalog(paths)
    available, names = adapter.importable_names(records)
    contents = blobs(repo, paths)
    parsed = {
        item["path"]: adapter.parse_item(
            {**item, "content": contents[item["path"]]},
            available, names[item["name"]],
        )
        for item in records
    }
    return parsed, sum(len(v.encode("utf-8")) for v in contents.values())


def cache_snapshot(
    base: str, tree: str, paths: dict[str, tuple[str, str]],
    observations: dict[str, dict], version: str,
) -> dict:
    return {
        "base_commit": base, "base_tree": tree, "producer": version,
        "catalog": catalog(paths), "paths": paths,
        "observations": observations,
        "receipts": {
            path: digest({"blob": paths[path][0], "mode": paths[path][1],
                          "observation": value, "producer": version})
            for path, value in observations.items()
        },
    }


def incremental(
    repo: Path, paths: dict[str, tuple[str, str]], cache: dict,
    adapter: Any, version: str,
) -> tuple[dict[str, dict], dict]:
    current_catalog = catalog(paths)
    if cache["producer"] != version:
        reason = "PRODUCER_CHANGED"
    elif cache["catalog"] != current_catalog:
        reason = "MODULE_CATALOG_CHANGED"
    else:
        reason = None
    if reason:
        results, read_bytes = full_scan(repo, paths, adapter)
        return results, {
            "reparsed": len(paths), "reused": 0, "fallback": reason,
            "source_bytes_read": read_bytes,
        }
    previous = cache["paths"]
    old_results = cache["observations"]
    changed = {p: current for p, current in paths.items()
               if previous[p] != current}
    preserved = {p: current for p, current in paths.items()
                 if previous[p] == current}
    reused: dict[str, dict] = {}
    for path in preserved:
        record = old_results[path]
        expected = digest({
            "blob": previous[path][0], "mode": previous[path][1],
            "observation": record, "producer": version,
        })
        if cache["receipts"][path] != expected:
            raise ValueError(f"cached observation receipt changed: {path}")
        reused[path] = record
    available, names = adapter.importable_names(current_catalog)
    content = blobs(repo, changed)
    fresh = {}
    for path in changed:
        entry = next(item for item in current_catalog if item["path"] == path)
        fresh[path] = adapter.parse_item(
            {**entry, "content": content[path]},
            available, names[entry["name"]],
        )
    return {p: fresh[p] if p in fresh else reused[p] for p in paths}, {
        "reparsed": len(changed), "reused": len(preserved),
        "fallback": None, "source_bytes_read": sum(
            len(v.encode("utf-8")) for v in content.values()
        ),
    }


def import_edges(observations: dict[str, dict]) -> set[tuple[str, str, int, str]]:
    return {
        (row["name"], imp["target"], imp["line"], imp["syntax"])
        for row in observations.values() for imp in row["imports"]
    }


def independent_oracle(
    repo: Path, paths: dict[str, tuple[str, str]],
) -> tuple[set[tuple[str, str, int, str]], int]:
    source = blobs(repo, paths)
    mapping = {module_name(path): path for path in paths}
    edges, unresolved = python_edges(source, mapping)
    return edges, len(unresolved)


def evaluate(
    repo: Path, base: str, head: str, roots: tuple[str, ...],
    repetitions: int = 5,
) -> dict:
    adapter = read_adapter()
    version = producer(roots)
    base_tree = git(repo, "rev-parse", f"{base}^{{tree}}")
    head_tree = git(repo, "rev-parse", f"{head}^{{tree}}")
    base_paths = inventory(repo, base, roots)
    head_paths = inventory(repo, head, roots)
    if not base_paths or not head_paths:
        raise ValueError("no supported Python files in one or both commits")
    initial_start = time.perf_counter_ns()
    base_records, initial_bytes = full_scan(repo, base_paths, adapter)
    cold_base_ms = (time.perf_counter_ns() - initial_start) / 1e6
    cache = cache_snapshot(base, base_tree, base_paths, base_records, version)
    measured: dict[str, list[float]] = {"full": [], "incremental": []}
    result_stats = None
    for n in range(repetitions):
        # Alternate execution order to reduce systematic warm-cache bias.
        order = ("full", "incremental") if n % 2 == 0 else ("incremental", "full")
        comparison: dict[str, dict[str, dict]] = {}
        for name in order:
            start = time.perf_counter_ns()
            if name == "full":
                records, _ = full_scan(repo, head_paths, adapter)
            else:
                records, result_stats = incremental(repo, head_paths, cache, adapter, version)
            measured[name].append((time.perf_counter_ns() - start) / 1e6)
            comparison[name] = records
        if comparison["full"] != comparison["incremental"]:
            raise ValueError("incremental output differs from complete native adapter pass")
    assert result_stats is not None
    independent, unresolved = independent_oracle(repo, head_paths)
    found = import_edges(comparison["incremental"])
    missing = sorted(independent - found)
    unexpected = sorted(found - independent)
    return {
        "schema": "excession.incremental-python-experiment.v1",
        "status": "MATCH" if not missing and not unexpected else "MISMATCH",
        "source_authority": "Git objects",
        "producer": "existing core/adapters/python_ast.py",
        "base_commit": base, "base_tree": base_tree,
        "head_commit": head, "head_tree": head_tree,
        "roots": list(roots),
        "baseline_sources": len(base_paths),
        "head_sources": len(head_paths),
        "changed_modules": sorted(
            p for p in base_paths.keys() | head_paths.keys()
            if base_paths.get(p) != head_paths.get(p)
        ),
        "reparsed_modules": result_stats["reparsed"],
        "reused_modules": result_stats["reused"],
        "fallback_reason": result_stats["fallback"],
        "base_index_wall_ms": round(cold_base_ms, 3),
        "median_full_head_ms": round(statistics.median(measured["full"]), 3),
        "median_incremental_head_ms": round(statistics.median(measured["incremental"]), 3),
        "repetitions": repetitions,
        "full_head_timings_ms": [round(x, 3) for x in measured["full"]],
        "incremental_head_timings_ms": [round(x, 3) for x in measured["incremental"]],
        "head_static_edges": len(found),
        "oracle_static_edges": len(independent),
        "missing_edges": [list(x) for x in missing[:50]],
        "unexpected_edges": [list(x) for x in unexpected[:50]],
        "oracle_unresolved_import_sites": unresolved,
        "head_source_bytes_read_incrementally": result_stats["source_bytes_read"],
        "base_source_bytes_read": initial_bytes,
        "scope": "Python static imports, symbols, exports and hazards; stable module catalog and parser",
        "not_established": [
            "production Rust incremental indexing",
            "runtime/import environment completeness",
            "cross-language dependencies",
            "end-to-end Excession CLI speedup",
            "safety of skipping tests",
            "trustworthiness of a malicious cache",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--roots", default="h11,tests")
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if args.repetitions < 1 or args.repetitions > 50:
        parser.error("repetitions must be between 1 and 50")
    try:
        repo = args.repo.resolve()
        commits = [git(repo, "rev-parse", rev) for rev in (args.base, args.head)]
        if any(rev != commit for rev, commit in zip((args.base, args.head), commits)):
            raise ValueError("use literal pinned Git commit IDs")
        report = evaluate(repo, commits[0], commits[1],
                          tuple(sorted(set(args.roots.split(",")))), args.repetitions)
    except Exception as exc:
        report = {"schema": "excession.incremental-python-experiment.v1",
                  "status": "ERROR", "error": f"{type(exc).__name__}: {exc}"}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0 if report["status"] == "MATCH" else 1


if __name__ == "__main__":
    raise SystemExit(main())
