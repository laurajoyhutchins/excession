#!/usr/bin/env python3
"""Independent native-evidence impact baseline, on pinned upstream Git trees.

We do not equate Go compiler-selected files, C translation-unit headers,
and Java source-symbol references. Every language has its own independent
oracle and a deliberately primitive baseline. Tests are about exact scope,
measurement hygiene and whether native observations reach impact candidates.

No user-supplied build scripts are invoked. Excession's trusted-local native
compiler analysis is NOT sandboxed. Run only against reviewed upstream pins.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import time
from pathlib import Path

SCHEMA = "excession.native-impact-baselines.v1"
ALLOWED = {
    "stleary/JSON-java": ("java", "2cd0a8c2112a3bc17d33ba8546c54ecb613613f1"),
    "hashicorp/golang-lru": ("go", "2bb29091a6c341aba625d23fea354c4fbaef5d6f"),
    "madler/zlib": ("c", "99623ef7097352da7993ba614161fd6a311e046e"),
}
HEX40 = re.compile(r"^[0-9a-f]{40}$")
SOURCE_PATH = re.compile(r"^[A-Za-z0-9_./-]+$")
LANGUAGES = {"java", "go", "c"}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(*command: str, cwd: Path | None = None, timeout: int = 180,
        env: dict[str, str] | None = None) -> bytes:
    proc = subprocess.run(command, cwd=cwd, env=env, capture_output=True,
                          timeout=timeout, check=False)
    if proc.returncode:
        message = proc.stderr.decode("utf-8", "replace")[:2500]
        raise RuntimeError(f"{command[0]} failed ({proc.returncode}): {message}")
    if len(proc.stdout) > 8 * 1024 * 1024:
        raise ValueError("native benchmark tool exceeded output budget")
    return proc.stdout


def git(repo: Path, *args: str) -> str:
    return run("git", "-C", str(repo), *args).decode().strip()


def validate_cases(manifest: dict) -> list[dict]:
    if set(manifest) != {"schema", "cases"} or manifest["schema"] != SCHEMA:
        raise ValueError("wrong native benchmark schema")
    if len(manifest["cases"]) != 3:
        raise ValueError("benchmark requires all three independent language cases")
    seen = set()
    for case in manifest["cases"]:
        if set(case) - {"id", "repository", "commit", "language",
                         "source", "changed_path", "source_prefix"}:
            raise ValueError("unexpected case fields")
        repo = case["repository"]
        if repo not in ALLOWED or (case["language"], case["commit"]) != ALLOWED[repo]:
            raise ValueError("only reviewed upstream repository + commit pins are allowed")
        if not HEX40.fullmatch(case["commit"]):
            raise ValueError("invalid Git pin")
        for key in ("id", "source", "changed_path"):
            value = case[key]
            if (not isinstance(value, str) or not SOURCE_PATH.fullmatch(value)
                    or ".." in value.split("/") or value.startswith("/")):
                raise ValueError(f"unsafe {key}")
        prefix = case.get("source_prefix")
        if (prefix is not None and
                (case["language"] != "java" or not SOURCE_PATH.fullmatch(prefix))):
            raise ValueError("invalid native language source prefix")
        if case["id"] in seen:
            raise ValueError("duplicate benchmark case")
        seen.add(case["id"])
    if {case["language"] for case in manifest["cases"]} != LANGUAGES:
        raise ValueError("benchmark must include all supported languages")
    return list(manifest["cases"])


def fetch(case: dict, path: Path) -> tuple[str, str]:
    run("git", "init", "-q", str(path))
    git(path, "remote", "add", "origin",
        f"https://github.com/{case['repository']}.git")
    git(path, "fetch", "-q", "--depth=2", "origin", case["commit"])
    git(path, "checkout", "-q", "--detach", case["commit"])
    head = git(path, "rev-parse", "HEAD")
    tree = git(path, "rev-parse", "HEAD^{tree}")
    if head != case["commit"] or git(path, "status", "--porcelain"):
        raise ValueError("upstream checkout pin or cleanliness failed")
    for key in ("source", "changed_path"):
        if not (path / case[key]).is_file():
            raise ValueError(f"missing upstream source file: {case[key]}")
    return head, tree


def precision_recall(predicted: set[str], reference: set[str]) -> dict:
    true_positive = predicted & reference
    missing = reference - predicted
    extra = predicted - reference
    return {
        "reference_count": len(reference),
        "predicted_count": len(predicted),
        "matched_count": len(true_positive),
        "precision": round(len(true_positive) / len(predicted), 5) if predicted else None,
        "recall": round(len(true_positive) / len(reference), 5) if reference else None,
        "missing": sorted(missing),
        "extra": sorted(extra),
        "reference_paths": sorted(reference),
        "predicted_paths": sorted(predicted),
    }


def source_files(repo: Path, suffix: str) -> list[str]:
    return sorted(path for path in git(repo, "ls-files", "-z").split("\0")
                  if path.endswith(suffix))


def independent_go(repo: Path) -> tuple[set[str], set[str]]:
    # Independent compiler-selected production closure across all local
    # dependencies, including package directories reached by "go list -deps".
    # This intentionally excludes *_test.go and build-tag alternatives.
    env = dict(os.environ, GOPROXY="off", GOSUMDB="off", GOWORK="off",
               CGO_ENABLED="0", GOTOOLCHAIN="local")
    raw = run("go", "list", "-deps", "-json", ".", cwd=repo, env=env)
    text = raw.decode()
    decoder = json.JSONDecoder()
    offset = 0
    oracle: set[str] = set()
    root = repo.resolve()
    while offset < len(text):
        while offset < len(text) and text[offset].isspace():
            offset += 1
        if offset == len(text):
            break
        package, offset = decoder.raw_decode(text, offset)
        if package.get("Error") or package.get("DepsErrors"):
            raise ValueError("independent go list has unresolved package inputs")
        if package.get("Standard"):
            continue
        directory = Path(package.get("Dir", "")).resolve()
        if not directory.is_relative_to(root):
            raise ValueError("independent Go package dependency lies outside pinned checkout")
        for filename in package.get("GoFiles", []) + package.get("SFiles", []):
            candidate = (directory / filename).resolve()
            if not candidate.is_relative_to(root):
                raise ValueError("independent Go source escapes pinned checkout")
            oracle.add(candidate.relative_to(root).as_posix())
    if not oracle:
        raise ValueError("independent go list returned no selected module inputs")
    baseline = {path for path in source_files(repo, ".go")
                if not path.endswith("_test.go")}
    return oracle, baseline

def independent_clang(repo: Path, source: str) -> tuple[set[str], set[str]]:
    # Clang's independent -MM invocation evaluates transitive user headers.
    makefile = run("clang", "-MM", "-I.", source, cwd=repo).decode()
    flattened = makefile.replace("\\\n", " ")
    if ":" not in flattened:
        raise ValueError("Clang returned no dependency target")
    tokens = flattened.split(":", 1)[1].split()
    tracked = set(git(repo, "ls-files", "-z").split("\0"))
    oracle = {str(Path(p)) for p in tokens if p in tracked and p != source}
    source_text = (repo / source).read_text(encoding="utf-8")
    direct = re.findall(r'^\s*#\s*include\s*["<]([^">]+)[">]',
                        source_text, re.MULTILINE)
    baseline = set()
    for filename in direct:
        for candidate in (str(Path(source).parent / filename), filename):
            if candidate in tracked:
                baseline.add(candidate)
                break
    if not oracle:
        raise ValueError("Clang oracle produced no tracked transitive headers")
    return oracle, baseline


def independent_java(repo: Path, prefix: str, target: str,
                     scratch: Path) -> tuple[set[str], set[str], float]:
    # Ground truth for this DIFFERENT, narrower predicate is compiled
    # bytecode class references, not all source-level semantic references.
    sources = [s for s in source_files(repo, ".java")
               if s.startswith(prefix.rstrip("/") + "/")]
    if not 1 <= len(sources) <= 128:
        raise ValueError("Java bytecode oracle source-set budget exceeded")
    out = scratch / "classes"
    empty = scratch / "empty-classpath"
    out.mkdir()
    empty.mkdir()
    start = time.perf_counter()
    run("javac", "-proc:none", "-implicit:none", "-classpath", str(empty),
        "-sourcepath", str(empty), "-d", str(out),
        *[str(repo / path) for path in sources], timeout=180)
    target_class = Path(target).stem
    emitted = sorted(out.rglob("*.class"))
    if not emitted:
        raise ValueError("Java oracle compiler emitted no classes")
    related = set()
    scanned = 0
    for path in emitted:
        # javap works on bytecode emitted by a separate compiler invocation.
        # The native JDK parser uses AST symbol bindings instead.
        source_stem = path.name.split("$", 1)[0].removesuffix(".class")
        candidate = f"{prefix}/org/json/{source_stem}.java"
        if candidate not in sources or candidate == target:
            continue
        listing = run("javap", "-v", str(path), timeout=30).decode("utf-8", "replace")
        pool = listing.split("{", 1)[0]
        scanned += 1
        if re.search(r"//\s+org/json/" + re.escape(target_class) +
                     r"(?:\b|[$./])", pool):
            related.add(candidate)
    duration = time.perf_counter() - start
    baseline = {path for path in sources if path != target
                and re.search(r"\b" + re.escape(target_class) + r"\b",
                              (repo / path).read_text(encoding="utf-8"))}
    if scanned < 2 or not related:
        raise ValueError("independent classfile reference oracle is trivially empty")
    return related, baseline, duration


def measured_native(binary: Path, repo: Path, case: dict,
                    output: Path) -> tuple[dict, float, float]:
    args = [str(binary), "analyze", "native", "--repo", str(repo),
            "--rev", case["commit"], "--language", case["language"],
            "--source", case["source"], "--trusted-local", "--json"]
    if case["language"] == "java":
        args.extend(["--source-prefix", case["source_prefix"]])
    elif case["language"] == "c":
        compilation = output / "compile_commands.json"
        compilation.write_text(json.dumps([{
            "directory": str(repo),
            "file": case["source"],
            "arguments": ["clang", "-I.", "-c", case["source"]],
        }]))
        args.extend(["--compile-db", str(compilation)])
    times = []
    reports = []
    for repeat in range(2):
        start = time.perf_counter()
        raw = run(*args, cwd=repo)
        times.append(time.perf_counter() - start)
        report = json.loads(raw)
        reports.append(report)
        (output / f"native-{repeat}.json").write_bytes(raw)
    if reports[0] != reports[1] or reports[0].get("source_commit") != case["commit"]:
        raise ValueError("native compiler evidence was nondeterministic or not Git pinned")
    return reports[0], times[0], times[1]


def classify(case: dict, repo: Path, report: dict, work: Path) -> dict:
    kind = case["language"]
    if kind == "go":
        start = time.perf_counter()
        oracle, baseline = independent_go(repo)
        oracle_s = time.perf_counter() - start
        observed = {item["source"]["path"] for item in report["inputs"]
                    if item["role"] == "source"}
        return {"predicate": "GO_COMPILER_SELECTED_SOURCE_INPUT",
                "native_vs_oracle": precision_recall(observed, oracle),
                "naive_vs_oracle": precision_recall(baseline, oracle),
                "independent_oracle_wall_s": round(oracle_s, 4),
                "change_impact_ground_truth": "UNSUPPORTED_SOURCE_MEMBERSHIP_IS_NOT_A_DEPENDENCY"}
    if kind == "c":
        start = time.perf_counter()
        oracle, baseline = independent_clang(repo, case["source"])
        oracle_s = time.perf_counter() - start
        headers = report["observation"]["observation"]["dependencies"]
        observed = {item["repo_path"] for item in headers
                    if item.get("repo_path") is not None
                    and item["repo_path"] != case["source"]}
        return {"predicate": "CLANG_TRACKED_TRANSITIVE_HEADER",
                "native_vs_oracle": precision_recall(observed, oracle),
                "naive_vs_oracle": precision_recall(baseline, oracle),
                "independent_oracle_wall_s": round(oracle_s, 4),
                "change_impact_ground_truth": "COMPILER_HEADER_MEMBERSHIP_NOT_RUNTIME_IMPACT"}
    if kind == "java":
        oracle, baseline, oracle_s = independent_java(
            repo, case["source_prefix"], case["changed_path"], work)
        references = report["observation"]["references"]
        observed = {edge["from"]["path"] for edge in references
                    if edge["to"]["path"] == case["changed_path"]}
        return {"predicate": "BYTECODE_CLASS_REFERENCE_TO_CHANGED_JAVA_SOURCE",
                "native_vs_oracle": precision_recall(observed, oracle),
                "naive_vs_oracle": precision_recall(baseline, oracle),
                "independent_oracle_wall_s": round(oracle_s, 4),
                "change_impact_ground_truth": "CLASSFILE_REFERENCES_ONLY_NOT_ALL_SOURCE_DEPENDENCIES"}
    raise ValueError("unrecognized language")


def one_case(case: dict, binary: Path, root: Path) -> dict:
    out = root / case["id"]
    out.mkdir()
    repo = out / "checkout"
    repo.mkdir()
    base, tree = fetch(case, repo)
    producer, cold_s, repeat_s = measured_native(binary, repo, case, out)
    if producer["source_tree"] != tree or producer["status"] != "PARTIAL":
        raise ValueError("native receipt mismatch with pinned upstream tree")
    classification = classify(case, repo, producer, out)
    report_file = out / "native-0.json"
    db = out / "evidence.sqlite"
    t0 = time.perf_counter()
    receipt = json.loads(run(str(binary), "evidence", "native-import",
                             "--repo", str(repo), "--rev", base,
                             "--report", str(report_file), "--db", str(db)))
    audit = json.loads(run(str(binary), "evidence", "native-store-audit",
                           "--repo", str(repo), "--rev", base,
                           "--snapshot", receipt["snapshot_id"],
                           "--db", str(db)))
    if audit.get("status") != "REPLAYED_GIT_BOUND_NATIVE_OBSERVATIONS":
        raise ValueError("native evidence audit did not replay")
    replay_s = time.perf_counter() - t0
    (out / "import.json").write_text(json.dumps(receipt, indent=2))
    (out / "audit.json").write_text(json.dumps(audit, indent=2))
    # Controlled, no-semantic-change edit on an INDEPENDENT upstream tree.
    # This is a conservative *candidate discovery* test, not behavioral impact.
    target = repo / case["changed_path"]
    with target.open("ab") as file:
        file.write(b"\n// excession-impact-benchmark-no-semantic-change\n")
    git(repo, "add", "--", case["changed_path"])
    git(repo, "-c", "user.name=Excession Benchmark",
        "-c", "user.email=excession-benchmark@example.invalid",
        "commit", "-qm", "benchmark: controlled source change")
    after = git(repo, "rev-parse", "HEAD")
    start = time.perf_counter()
    impact = json.loads(run(str(binary), "impact", ".", "--db", str(db),
                            "--base", base, "--head", after, "--json",
                            cwd=repo, timeout=180))
    impact_s = time.perf_counter() - start
    (out / "impact.json").write_text(json.dumps(impact, indent=2))
    candidates = {item["entity"]["path"]
                  for item in impact.get("candidate_source_impacts", [])}
    observed = impact.get("configured_observations", [])
    matching_configured = [item for item in observed
                           if item["source"]["path"] == case["changed_path"]]
    impact_result = {
        "candidate_source_count": impact["candidate_source_count"],
        "candidate_paths": sorted(candidates),
        "matching_configured_observation_count": len(matching_configured),
        "safe_to_skip_unlisted_tests": impact["safe_to_skip_unlisted_tests"],
        "native_reference_candidate_comparison": (
            precision_recall(candidates, set(classification["native_vs_oracle"]["reference_paths"]))
            if case["language"] == "java" else None
        ),
    }
    if impact_result["safe_to_skip_unlisted_tests"] is not False:
        raise ValueError("impact incorrectly certified unlisted tests as safe")
    result = {
        "case": case["id"], "repository": case["repository"],
        "language": case["language"], "upstream_commit": base,
        "upstream_tree": tree, "controlled_candidate_commit": after,
        "controlled_mutation": "append-comment-only",
        "scope": producer["scope"], "producer": producer["producer"],
        "source": case["source"], "changed_path": case["changed_path"],
        "report_id": producer["report_id"], "snapshot_id": receipt["snapshot_id"],
        "classification": classification, "impact": impact_result,
        "cost": {
            "native_cold_wall_s": round(cold_s, 4),
            "native_repeat_wall_s": round(repeat_s, 4),
            "independent_oracle_wall_s": classification["independent_oracle_wall_s"],
            "native_import_and_audit_wall_s": round(replay_s, 4),
            "impact_wall_s": round(impact_s, 4),
            "stored_evidence_bytes": db.stat().st_size,
            "report_bytes": report_file.stat().st_size,
            "cached_incremental_native_execution": "NOT_IMPLEMENTED",
        },
        "limitations": [
            "Source records are compiler-scope observations, not full runtime dependency truth",
            "Java bytecode class references and JDK AST source symbols are distinct predicates",
            "Each native invocation reparses or reselects; repeat time is NOT incremental caching",
            "Controlled commit is comment-only and cannot establish actual change breakage",
        ],
    }
    (out / "case-result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()
    binary = args.bin.resolve(strict=True)
    raw_manifest = args.manifest.read_bytes()
    cases = validate_cases(json.loads(raw_manifest))
    args.out_dir.mkdir(parents=True, exist_ok=True)
    if any(args.out_dir.iterdir()):
        raise ValueError("benchmark requires an empty output directory")
    summary = {
        "schema": "excession.native-impact-benchmark-results.v1",
        "manifest_sha256": sha256(raw_manifest),
        "excession_binary_sha256": sha256(binary.read_bytes()),
        "cases": [], "failures": {},
    }
    for case in cases:
        start = time.perf_counter()
        try:
            report = one_case(case, binary, args.out_dir)
            summary["cases"].append(report)
            print(f"{case['id']}: measured {report['classification']['predicate']}",
                  flush=True)
        except (ValueError, RuntimeError, subprocess.TimeoutExpired, OSError, KeyError) as err:
            summary["failures"][case["id"]] = str(err)[:2500]
            print(f"{case['id']}: FAILED: {err}", flush=True)
        print(f"{case['id']}: total {time.perf_counter()-start:.2f}s", flush=True)
    (args.out_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n")
    if summary["failures"] or len(summary["cases"]) != len(cases):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
