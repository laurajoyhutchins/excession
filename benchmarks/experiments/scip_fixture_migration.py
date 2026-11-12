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
HISTORICAL_ROOTS = {
    "h11": "h11,tests",
    "click": "click,src,tests",
    "requests": "requests,src,tests",
    "postcss": "lib,src,tests",
}
HISTORICAL_FAMILIES = frozenset(HISTORICAL_ROOTS)


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
                "roots": HISTORICAL_ROOTS[family],
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


def typescript_fixture_config(stage: Path, sources: set[str]) -> Path:
    """Explicit sidecar project, preserving the pinned repository's own config.

    Git-tracked fixture roots are the source inventory. Unlike an inferred
    TypeScript project, this includes JavaScript even when an upstream tsconfig
    selects only .ts declarations or test sources. This is a *separate* project
    scope, not a claim about the original build's selected source files.
    """
    if not sources:
        raise ValueError("cannot configure an empty TypeScript fixture scope")
    filename = stage / "tsconfig.excession-scip-fixture.json"
    if filename.exists():
        raise ValueError("fixture overlay would overwrite a source file")
    config = {
        "compilerOptions": {
            "allowJs": True,
            "checkJs": False,
            "noEmit": True,
            "skipLibCheck": True,
        },
        "files": sorted(sources),
        "include": [],
    }
    if (stage / "tsconfig.json").is_file():
        config["extends"] = "./tsconfig.json"
    filename.write_text(json.dumps(config, indent=2, sort_keys=True) + "\n")
    return filename


def generate_index(stage: Path, output: Path, binary: Path, scip: Path,
                   language: str, roots: list[str], project: str,
                   source_files: set[str]) -> bytes:
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
        configured = typescript_fixture_config(stage, source_files)
        argv = [binary, "index", configured.name, "--output", encoded]
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


def replay_stored_scip_evidence(bin_path: Path, source: Path, commit: str,
                                decoded: Path, database: Path,
                                expected_cross_file: int) -> dict:
    """Import a real upstream index without rerunning it, then replay Git from
    the *stored original bytes* and independently audit stored relationships.
    A successful replay does not establish indexer semantic correctness.
    """
    imported = json.loads(command([
        bin_path, "refactor", "scip-ingest", "--repo", source,
        "--rev", commit, "--index", decoded, "--db", database,
    ]))
    snapshot_id = imported["snapshot_id"]
    if (len(snapshot_id) != 64
            or imported["source_commit"] != commit
            or imported["matched_cross_file_references"] != expected_cross_file):
        raise ValueError("SCIP persisted evidence identity or relation count mismatch")
    inspected = json.loads(command([
        bin_path, "inspect", "scip-evidence", "--db", database,
        "--snapshot", snapshot_id,
    ]))
    replayed = json.loads(command([
        bin_path, "refactor", "scip-store-audit", "--repo", source,
        "--rev", commit, "--db", database, "--snapshot", snapshot_id,
    ]))
    if (inspected["source_commit"] != commit
            or inspected["matched_observation_count"] != expected_cross_file
            or inspected["absence_proven"]
            or inspected["safe_to_skip_tests"]
            or replayed["status"] != "REPLAYED_GIT_BOUND_SCIP_OBSERVATIONS"
            or replayed["indexer_independently_executed"]
            or replayed["symbol_semantics_independently_verified"]
            or replayed["relationship_absence_proven"]):
        raise ValueError("SCIP store query/replay contradicts its coverage or authority limits")
    return {
        "snapshot_id": snapshot_id,
        "source_replay": replayed["status"],
        "retained_cross_file_references": inspected["matched_observation_count"],
        "indexer_semantics_verified": False,
        "relationship_absence_proven": False,
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
                args.scip, language, roots, family, included,
            )
            elapsed_ms = round((time.perf_counter_ns() - start) / 1_000_000, 3)
            producer = "upstream indexer executed in disposable checkout"
        config_file = stage / "tsconfig.excession-scip-fixture.json"
        if language == "typescript" and not args.decoded_dir:
            # Save the exact generated analysis project alongside the index.
            # It never replaces the pinned repository's original tsconfig.
            provenance = evidence / f"{name}-{language}.tsconfig.json"
            shutil.copyfile(config_file, provenance)
            configuration = {
                "scope": "explicit_git_fixture_files",
                "analysis_tsconfig_sha256": digest(provenance.read_bytes()),
                "analysis_tsconfig_path": provenance.name,
                "original_tsconfig_sha256": (
                    digest((repo / "tsconfig.json").read_bytes())
                    if (repo / "tsconfig.json").is_file() else None
                ),
            }
        elif language == "typescript":
            configuration = {
                "scope": "externally_supplied_unknown",
                "analysis_tsconfig_sha256": None,
            }
        else:
            configuration = {"scope": "python_fixture_roots"}
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
        # A large upstream index is still valid SCIP; the present in-DB
        # receipt implementation has a separate 4 MiB capacity bound.
        # Report unsupported capacity instead of treating it as a clean index.
        if len(decoded) <= 4 * 1024 * 1024:
            stored = replay_stored_scip_evidence(
                args.bin, repo, commit, output,
                evidence / f"{name}-{language}.evidence.sqlite",
                report["coverage"]["matched_cross_file_references"],
            )
        else:
            stored = {
                "status": "UNSUPPORTED_RECEIPT_CAPACITY",
                "index_bytes": len(decoded),
                "limit_bytes": 4 * 1024 * 1024,
            }
        result["languages"].append({
            "language_family": language,
            "status": finding["status"],
            "indexer": report["indexer"],
            "index_sha256": digest(decoded),
            "report_id": report["report_id"],
            "generation_wall_ms": elapsed_ms,
            "evidence_origin": producer,
            "analysis_configuration": configuration,
            "coverage": finding,
            "stored_evidence": stored,
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


def independently_replayed_occurrences(fixtures: list[dict]) -> int:
    """A green smoke requires real source-bound observations, not empty reports."""
    return sum(
        entry["coverage"]["indexed_occurrences"]
        for fixture in fixtures if fixture.get("status") != "INVALID"
        for entry in fixture.get("languages", [])
        if entry["status"] in {"SOURCE_BOUND", "PARTIAL"}
    )


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
    evidence = evidence.resolve()
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
    observed = independently_replayed_occurrences(results["fixtures"])
    results["source_bound_occurrences"] = observed
    stored_real = sum(
        entry["stored_evidence"]["retained_cross_file_references"]
        for finding in results["fixtures"]
        for entry in finding.get("languages", [])
        if "retained_cross_file_references" in entry.get("stored_evidence", {})
    )
    results["persisted_source_bound_cross_file_references"] = stored_real
    persisted = sum(
        1 for finding in results["fixtures"]
        for entry in finding.get("languages", [])
        if "snapshot_id" in entry.get("stored_evidence", {})
    )
    results["persisted_source_bound_scip_snapshots"] = persisted
    # A legitimate upstream index can have zero cross-file matches.
    # Requiring a replayed original receipt tests the integration without
    # incorrectly claiming a negative relationship conclusion.
    if persisted == 0:
        invalid = True
        results["status"] = "NO_REPLAYED_SCIP_EVIDENCE_SNAPSHOTS"
    if observed == 0:
        invalid = True
        results["status"] = "NO_VALID_SCIP_OBSERVATIONS"
    elif persisted == 0:
        results["status"] = "NO_REPLAYED_SCIP_EVIDENCE_SNAPSHOTS"
    elif stored_real == 0:
        results["status"] = "SOURCE_BOUND_SCIP_OBSERVATIONS_WITH_UNRESOLVED_RELATIONS"
    else:
        results["status"] = "SOURCE_BOUND_SCIP_OBSERVATIONS"
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
    if invalid:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
