#!/usr/bin/env python3
"""Controlled Java impact A/B with a source-written oracle.

Not a real-repository or complete-semantic benchmark. The independently
specified denominator is source-level Java references in five small files.
Two intentionally weak baselines make their limitations measurable rather
than proving superiority over existing industrial Java tools.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time

SOURCES = {
    "src/main/java/sample/Helper.java":
        "package sample; public final class Helper { public static int answer() { return 42; } }\n",
    "src/main/java/sample/Service.java":
        "package sample; public final class Service { public static int run() { return Helper.answer(); } }\n",
    "src/main/java/sample/Client.java":
        "package sample; public final class Client { public static int execute() { return Service.run(); } }\n",
    "src/main/java/sample/Shadowed.java":
        "package sample; public final class Shadowed { public int run() { int Helper = 7; return Helper; } }\n",
    "src/main/java/sample/Unrelated.java":
        "package sample; public final class Unrelated { public int run() { return 0; } }\n",
}
DIRECT_REFERENCE_ORACLE = {
    ("src/main/java/sample/Service.java", "src/main/java/sample/Helper.java"),
    ("src/main/java/sample/Client.java", "src/main/java/sample/Service.java"),
}
CHANGED = "src/main/java/sample/Helper.java"
EXPECTED_AFFECTED = {
    "src/main/java/sample/Service.java",
    "src/main/java/sample/Client.java",
}


def command(args: list[str], *, cwd: Path, timeout: int = 180, env=None) -> bytes:
    process = subprocess.run(args, cwd=cwd, env=env, capture_output=True, timeout=timeout)
    if process.returncode:
        raise RuntimeError(
            f"{args[:5]} exited {process.returncode}: "
            f"{process.stderr.decode(errors='replace')[-1200:]}"
        )
    return process.stdout


def git(repo: Path, *args: str, env=None) -> str:
    return command(["git", "-C", str(repo), *args], cwd=repo, env=env).decode().strip()


def build_fixture(repo: Path) -> tuple[str, str]:
    repo.mkdir(parents=True)
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "core.autocrlf", "false")
    for path, content in sorted(SOURCES.items()):
        file = repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content, encoding="utf-8")
    git(repo, "add", ".")
    import os
    env = os.environ.copy()
    env.update({
        "GIT_AUTHOR_DATE": "2020-01-01T00:00:00Z",
        "GIT_COMMITTER_DATE": "2020-01-01T00:00:00Z",
    })
    args = ["-c", "user.name=Excession Bench",
            "-c", "user.email=benchmark@example.invalid", "commit", "-qm"]
    git(repo, *args, "fixed source", env=env)
    base = git(repo, "rev-parse", "HEAD")
    (repo / CHANGED).write_text(SOURCES[CHANGED].replace("return 42;", "return 43;"))
    git(repo, "add", CHANGED)
    git(repo, *args, "change Helper.answer", env=env)
    head = git(repo, "rev-parse", "HEAD")
    return base, head


def reachable(edges: set[tuple[str, str]], seed: str) -> set[str]:
    seen = {seed}
    pending = [seed]
    result = set()
    while pending:
        changed = pending.pop()
        for source, target in edges:
            if target == changed and source not in seen:
                seen.add(source)
                result.add(source)
                pending.append(source)
    return result


def imports_only(source: dict[str, str]) -> set[tuple[str, str]]:
    edges = set()
    for path, contents in source.items():
        for target in source:
            if path != target:
                class_name = Path(target).stem
                if re.search(r"(?m)^\s*import\s+(?:static\s+)?sample\." +
                             re.escape(class_name) + r"\s*;", contents):
                    edges.add((path, target))
    return edges


def name_tokens(source: dict[str, str]) -> set[tuple[str, str]]:
    edges = set()
    for path, contents in source.items():
        for target in source:
            if path != target and re.search(
                r"\b" + re.escape(Path(target).stem) + r"\b", contents
            ):
                edges.add((path, target))
    return edges


def score(candidates: set[str], expected: set[str]) -> dict:
    tp = len(candidates & expected)
    return {
        "candidates": sorted(candidates),
        "true_positives": tp,
        "false_positives": sorted(candidates - expected),
        "false_negatives": sorted(expected - candidates),
        "precision": tp / len(candidates) if candidates else None,
        "recall": tp / len(expected) if expected else None,
    }


def timed(fn):
    start = time.perf_counter()
    result = fn()
    return result, (time.perf_counter() - start) * 1000.0


def run(binary: Path, destination: Path) -> dict:
    from os import chmod
    with tempfile.TemporaryDirectory(prefix="excession-native-oracle-") as directory:
        root = Path(directory)
        repo = root / "source"
        base, head = build_fixture(repo)
        git(repo, "checkout", "-q", "--detach", base)
        # Explicit, reviewed source oracle. Never derive expected edges from
        # Excession, javac reports, or the baseline scanners.
        assert reachable(DIRECT_REFERENCE_ORACLE, CHANGED) == EXPECTED_AFFECTED
        sources = {p: (repo / p).read_text() for p in SOURCES}
        (import_edges, import_ms) = timed(lambda: imports_only(sources))
        (token_edges, token_ms) = timed(lambda: name_tokens(sources))
        db = root / "evidence.sqlite"
        report_path = root / "java.json"
        report, producer_ms = timed(lambda: json.loads(command([
            str(binary), "analyze", "native", "--repo", str(repo),
            "--rev", base, "--language", "java",
            "--source", "src/main/java/sample/Service.java",
            "--source-prefix", "src/main/java", "--trusted-local", "--json",
        ], cwd=repo)))
        if report["producer"] != "javac-symbols":
            raise ValueError("unexpected Java producer")
        direct = {
            (edge["from"]["path"], edge["to"]["path"])
            for edge in report["observation"]["references"]
        }
        if direct != DIRECT_REFERENCE_ORACLE:
            raise AssertionError(f"compiler observations differ from source oracle: {direct!r}")
        report_path.write_text(json.dumps(report, separators=(",", ":")))
        imported, import_receipt_ms = timed(lambda: json.loads(command([
            str(binary), "evidence", "native-import", "--repo", str(repo),
            "--rev", base, "--report", str(report_path), "--db", str(db),
        ], cwd=repo)))
        if not imported.get("snapshot_id"):
            raise AssertionError("native evidence import has no snapshot identity")
        git(repo, "checkout", "-q", "--detach", head)
        def query(with_native: bool) -> dict:
            return json.loads(command([
                str(binary), "impact", str(repo), "--base", base,
                "--head", head, "--db", str(db if with_native else root / "empty.sqlite"),
                "--json",
            ], cwd=repo))
        base_impact, base_impact_ms = timed(lambda: query(False))
        native_impact, native_impact_ms = timed(lambda: query(True))
        second_impact, replay_ms = timed(lambda: query(True))
        if native_impact["report_id"] != second_impact["report_id"]:
            raise AssertionError("same-revision evidence replay is not deterministic")
        observed = native_impact["native_reference_impacts"]
        actual = {
            row["entity"]["path"]
            for row in observed
        }
        if actual != EXPECTED_AFFECTED or native_impact["native_reference_count"] != len(actual):
            raise AssertionError("native affected source set differs from independent oracle")
        if not native_impact["native_reference_enumeration_complete"]:
            raise AssertionError("native source inventory truncated")
        for item in observed:
            hop = item["next_hop"]
            if hop["relation"] != "JAVAC_RESOLVED_SOURCE_REFERENCE" or not hop["evidence"]:
                raise AssertionError("native candidate lacks its source-refined evidence witness")
            if hop["evidence"][0]["status"] != "PRODUCER_SUPPLIED_UNATTESTED":
                raise AssertionError("unattested compiler evidence was misrepresented")
        ordinary = {
            item["entity"]["path"] for item in base_impact["candidate_source_impacts"]
        }
        if EXPECTED_AFFECTED & ordinary:
            raise AssertionError("Java references unexpectedly present in static/SCIP baseline")
        verify = json.loads(command([
            str(binary), "agent", "verify", "--repo", str(repo), "--base", base,
            "--candidate", head, "--db", str(db),
        ], cwd=repo))
        if verify["ready_to_merge"] or verify["native_reference_count"] != len(EXPECTED_AFFECTED):
            raise AssertionError("agent must report candidates without asserting readiness")
        if "NATIVE_PRODUCER_ATTESTATION_MISSING" not in verify["obligations"]:
            raise AssertionError("agent did not preserve native attestation obligation")
        bundle = destination / "controlled-java.git.bundle"
        git(repo, "bundle", "create", str(bundle), "--all")
        return {
            "schema": "excession.native-reference-ab.v1",
            "status": "PASS",
            "fixture": {
                "type": "CONTROLLED_SYNTHETIC_JAVA",
                "base": base,
                "head": head,
                "source_count": len(SOURCES),
                "changed_path": CHANGED,
                "oracle_direct_references": sorted([list(x) for x in DIRECT_REFERENCE_ORACLE]),
                "oracle_affected": sorted(EXPECTED_AFFECTED),
                "bundle_sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
            },
            "scores": {
                "import_only": score(reachable(import_edges, CHANGED), EXPECTED_AFFECTED),
                "name_token": score(reachable(token_edges, CHANGED), EXPECTED_AFFECTED),
                "static_scip_candidate": score(ordinary, EXPECTED_AFFECTED),
                "native_java_reference": score(actual, EXPECTED_AFFECTED),
            },
            "cost": {
                "import_only_parse_ms": import_ms,
                "name_token_scan_ms": token_ms,
                "java_compiler_ms": producer_ms,
                "evidence_import_ms": import_receipt_ms,
                "static_scip_impact_ms": base_impact_ms,
                "native_impact_ms": native_impact_ms,
                "same_revision_replay_ms": replay_ms,
                "evidence_db_bytes": db.stat().st_size,
            },
            "coverage": {
                "scope": "JAVAC_SOURCE_REFERENCES_IN_TRACKED_SELF_CONTAINED_SOURCE_SET",
                "producer_attested": False,
                "runtime_semantics_proven": False,
                "build_graph_complete": False,
                "incremental_computation_demonstrated": False,
            },
            "limitations": [
                "Five controlled Java sources, not independent upstream repositories",
                "Lexical baselines intentionally lack compiler symbol resolution",
                "Timing includes subprocess startup and differs by work performed",
                "Replaying an immutable receipt is not incremental compilation",
            ],
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    out = args.out_dir.resolve()
    out.mkdir(parents=True, exist_ok=True)
    report = run(args.bin.resolve(), out)
    (out / "summary.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
