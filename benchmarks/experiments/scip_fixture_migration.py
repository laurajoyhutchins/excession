#!/usr/bin/env python3
"""Evaluate real SCIP indexers against source-bound evidence, not legacy parity.

SCIP owns its symbols. Git owns source identity. This runner measures coverage;
it does not invent import edges or claim semantic correctness from an indexer.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from real_fixtures import checkout

ROOT = Path(__file__).resolve().parents[2]
BUNDLES = ROOT / "core/fixtures"
HISTORY = BUNDLES / "crossrepo_revisions.json"
EXTENSIONS = {
    "python": (".py",),
    "typescript": (".js", ".jsx", ".ts", ".tsx", ".cjs", ".mjs"),
}
HISTORICAL_FAMILIES = frozenset({"h11", "click", "requests", "postcss"})


def command(args: list[object], *, cwd: Path | None = None,
            timeout: int = 360) -> bytes:
    proc = subprocess.run([str(x) for x in args], cwd=cwd,
                          capture_output=True, timeout=timeout, check=False)
    if proc.returncode:
        detail = proc.stderr.decode(errors="replace")[:1200]
        raise RuntimeError(f"{args[0]} exited {proc.returncode}: {detail}")
    return proc.stdout


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cases(manifest: Path, historical: bool, names: set[str]) -> list[dict]:
    declaration = json.loads(manifest.read_text())
    if declaration["schema"] != "excession.real-projection-fixtures.v1":
        raise ValueError("unknown immutable fixture manifest schema")
    result = [dict(x) for x in declaration["fixtures"]]
    if historical:
        for family, commit in json.loads(HISTORY.read_text()):
            if family not in HISTORICAL_FAMILIES or len(commit) != 40:
                raise ValueError("unexpected historical fixture identity")
            result.append({
                "name": f"{family}-{commit[:9]}", "family": family,
                "source": "historical", "commit": commit,
                "roots": f"{family},tests",
            })
    if names:
        result = [x for x in result if x["name"] in names
                  or x.get("family", x["name"]) in names]
        matched = {x["name"] for x in result} | {
            x.get("family", x["name"]) for x in result
        }
        if names - matched:
            raise ValueError(f"unknown fixture selections: {sorted(names - matched)}")
    return result


def checkout_fixture(case: dict, dest: Path) -> tuple[str, str]:
    if case["source"] != "historical":
        return checkout(case, dest)
    family = case["family"]
    if family not in HISTORICAL_FAMILIES:
        raise ValueError("unknown historical family")
    command(["git", "clone", "-q", BUNDLES / f"{family}-crossrepo.bundle", dest])
    command(["git", "-C", dest, "checkout", "-q", "--detach", case["commit"]])
    commit = command(["git", "-C", dest, "rev-parse", "HEAD"]).decode().strip()
    tree = command(["git", "-C", dest, "rev-parse", "HEAD^{tree}"]).decode().strip()
    if commit != case["commit"]:
        raise ValueError("bundle revision differs from fixture declaration")
    return commit, tree


def tracked_sources(repo: Path, commit: str, roots: list[str],
                    language: str) -> set[str]:
    """Scope is explicit Git-tracked regular source, never the old SQL catalog."""
    raw = command(["git", "-C", repo, "ls-tree", "-rz", commit])
    eligible: set[str] = set()
    for item in raw.split(b"\0"):
        if not item:
            continue
        metadata, path_bytes = item.split(b"\t", 1)
        mode, kind, _ = metadata.decode("ascii").split()
        path = path_bytes.decode("utf-8")
        if (kind == "blob" and mode in ("100644", "100755")
                and path.endswith(EXTENSIONS[language])
                and any(path.startswith(root.rstrip("/") + "/") for root in roots)
                and not any(part in {"node_modules", "vendor", ".venv"}
                            for part in path.split("/"))):
            eligible.add(path)
    return eligible


def generate_index(stage: Path, output: Path, binary: Path, scip: Path,
                   language: str, roots: list[str], project: str) -> bytes:
    """External indexers only touch a disposable copy; never the pinned tree."""
    encoded = output.with_suffix(".scip")
    if language == "python":
        config = stage / "pyrightconfig.json"
        if not config.exists():
            config.write_text(json.dumps({
                "include": [r for r in roots if (stage / r).exists()],
                "extraPaths": ["src"] if (stage / "src").is_dir() else [],
                "typeCheckingMode": "basic",
            }) + "\n")
        argv = [binary, "index", ".", "--project-name", project,
                "--project-version", "0.0.0", "--output", encoded]
    elif language == "typescript":
        if not (stage / "package.json").exists():
            (stage / "package.json").write_text(
                '{"name":"scip-fixture","private":true,"version":"0.0.0"}\n'
            )
        argv = [binary, "index", "--output", encoded]
        if not (stage / "tsconfig.json").exists():
            argv.append("--infer-tsconfig")
    else:
        raise ValueError("unsupported source language")
    command(argv, cwd=stage)
    if not encoded.is_file():
        raise RuntimeError("upstream indexer did not produce a SCIP file")
    decoded = command([scip, "print", "--json", encoded])
    # This input remains verbatim. Do not normalize symbols, invent language
    # fields or derive synthetic imports from names and paths.
    json.loads(decoded)
    output.write_bytes(decoded)
    return decoded


def coverage(expected: set[str], documents: set[str],
             symbols: dict) -> dict:
    missing = sorted(expected - documents)
    additional = sorted(documents - expected)
    count = symbols["indexed_occurrences"]
    if not expected:
        status = "UNSUPPORTED"
    elif not documents:
        status = "UNSUPPORTED"
    elif missing or count == 0:
        status = "PARTIAL"
    else:
        status = "SOURCE_BOUND"
    return {
        "status": status,
        "eligible_tracked_source_files": len(expected),
        "indexed_source_documents": len(documents),
        "missing_source_files": missing[:100],
        "missing_source_file_count": len(missing),
        "additional_source_files": additional[:100],
        "additional_source_file_count": len(additional),
        "indexed_occurrences": count,
        "defined_symbols": symbols["unique_defined_symbols"],
        "matched_cross_file_symbol_references": (
            symbols["matched_cross_file_references"]
        ),
        "unresolved_symbol_references": symbols["unresolved_references"],
        "line_only_position_documents": symbols["line_only_position_documents"],
        "semantic_correctness_independently_verified": False,
        "indexer_run_attested": False,
    }


def verify_one(args: argparse.Namespace, case: dict, work: Path,
               evidence: Path) -> dict:
    name = case["name"]
    family = case.get("family", name)
    repo = work / f"{name}-source"
    commit, tree = checkout_fixture(case, repo)
    roots = case["roots"].split(",")
    original = command(["git", "-C", repo, "status", "--porcelain",
                        "--untracked-files=all"]).strip()
    if original:
        raise ValueError("pinned source checkout is dirty")
    languages = [
        language for language in EXTENSIONS
        if tracked_sources(repo, commit, roots, language)
    ]
    result = {
        "name": name, "family": family,
        "source_commit": commit, "source_tree": tree, "roots": roots,
        "languages": [],
    }
    for language in languages:
        included = tracked_sources(repo, commit, roots, language)
        stage = work / f"{name}-{language}-indexer"
        output = evidence / f"{name}-{language}.json"
        if args.decoded_dir:
            supplied = args.decoded_dir / output.name
            if not supplied.is_file():
                raise ValueError(f"missing externally decoded SCIP input: {supplied}")
            shutil.copyfile(supplied, output)
            producer = "externally supplied SCIP JSON; execution not observed"
            elapsed_ms = None
        else:
            shutil.copytree(repo, stage)
            start = time.perf_counter_ns()
            generate_index(
                stage, output,
                args.python_indexer if language == "python"
                else args.typescript_indexer,
                args.scip, language, roots, family,
            )
            elapsed_ms = round((time.perf_counter_ns() - start) / 1_000_000, 3)
            producer = "upstream indexer executed in disposable checkout"
        decoded = output.read_bytes()
        parsed = json.loads(decoded)
        documents = parsed.get("documents")
        if not isinstance(documents, list):
            raise ValueError("SCIP lacks document array")
        relative = [doc["relative_path"] for doc in documents]
        if any(not isinstance(p, str) for p in relative):
            raise ValueError("SCIP document path is not text")
        report_file = evidence / f"{name}-{language}.report.json"
        audit_file = evidence / f"{name}-{language}.audit.json"
        report_bytes = command([
            args.bin, "refactor", "scip-suggest",
            "--repo", repo, "--rev", commit, "--index", output,
        ])
        report_file.write_bytes(report_bytes)
        report = json.loads(report_bytes)
        receipt_bytes = command([
            args.bin, "refactor", "scip-audit", "--repo", repo,
            "--rev", commit, "--index", output, "--report", report_file,
        ])
        audit_file.write_bytes(receipt_bytes)
        receipt = json.loads(receipt_bytes)
        if (report["source_commit"] != commit
                or report["source_tree"] != tree
                or report["index_sha256"] != digest(decoded)
                or receipt["status"] != "REPLAYED_SCIP_INDEX"
                or receipt["report_id"] != report["report_id"]):
            raise ValueError("SCIP Git binding, digest or replay receipt mismatch")
        finding = coverage(included, set(relative), report["coverage"])
        result["languages"].append({
            "language_family": language,
            "status": finding["status"],
            "indexer": report["indexer"],
            "index_sha256": digest(decoded),
            "report_id": report["report_id"],
            "generation_wall_ms": elapsed_ms,
            "evidence_origin": producer,
            "coverage": finding,
        })
    if not result["languages"]:
        result["status"] = "UNSUPPORTED"
    elif any(x["status"] == "PARTIAL" for x in result["languages"]):
        result["status"] = "PARTIAL"
    elif all(x["status"] == "SOURCE_BOUND" for x in result["languages"]):
        result["status"] = "SOURCE_BOUND"
    else:
        result["status"] = "UNSUPPORTED"
    if command(["git", "-C", repo, "status", "--porcelain",
                "--untracked-files=all"]).strip():
        raise ValueError("source checkout mutated during external indexing")
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--manifest", type=Path,
                        default=ROOT / "benchmarks/fixtures/real-repositories.json")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--fixture", action="append", default=[])
    parser.add_argument("--historical", action="store_true")
    parser.add_argument("--trusted-local", action="store_true")
    parser.add_argument("--decoded-dir", type=Path)
    parser.add_argument("--scip", type=Path, default=Path("scip"))
    parser.add_argument("--python-indexer", type=Path, default=Path("scip-python"))
    parser.add_argument("--typescript-indexer", type=Path,
                        default=Path("scip-typescript"))
    args = parser.parse_args()
    if not args.decoded_dir and not args.trusted_local:
        parser.error("explicit --trusted-local or --decoded-dir is required")
    selected = cases(args.manifest, args.historical, set(args.fixture))
    if not selected:
        parser.error("no matching pinned fixtures")
    evidence = args.evidence_dir or args.out.parent / "scip-fixture-evidence"
    evidence.mkdir(parents=True, exist_ok=True)
    results = {
        "schema": "excession.scip-fixture-evidence.v2",
        "manifest_sha256": digest(args.manifest.read_bytes()),
        "historical_sha256": digest(HISTORY.read_bytes()) if args.historical else None,
        "contract": (
            "SOURCE_BOUND verifies Git content binding, input digest and exact "
            "report replay, not the indexer's semantic correctness. PARTIAL "
            "exposes unindexed source scope or empty observations. "
            "UNSUPPORTED denotes missing applicable source or no documents. "
            "INVALID is a failed source-bound observation or execution."
        ),
        "fixtures": [],
    }
    invalid = False
    with tempfile.TemporaryDirectory(prefix="excession-scip-evidence-") as temp:
        for item in selected:
            try:
                finding = verify_one(args, item, Path(temp), evidence)
            except (OSError, ValueError, RuntimeError, KeyError, TypeError,
                    subprocess.TimeoutExpired) as err:
                invalid = True
                finding = {"name": item["name"], "status": "INVALID",
                           "diagnostic": str(err)[:1600]}
            results["fixtures"].append(finding)
            print(f"{finding['name']}: {finding['status']}", flush=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
    if invalid:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
