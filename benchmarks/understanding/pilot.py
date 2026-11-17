#!/usr/bin/env python3
"""Pinned-code comprehension pilot: source-finding diagnostic, not an AI grader.

The question+oracle manifest is committed separately before this runner. Both
arms use the identical natural-language question and exact public Git commit.
The only automatic score is whether the proposed reading order contains
independently named implementation/test files. Semantic correctness requires
a separately reviewed, blinded human/agent transcript.
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
HEX40 = re.compile(r"^[0-9a-f]{40}$")
SAFE_REPO = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
WORD = re.compile(r"[A-Za-z][A-Za-z0-9_]+")
SOURCE_EXT = (".c", ".h", ".py", ".rs", ".go", ".js", ".ts", ".tsx",
              ".jsx", ".java", ".cc", ".cpp", ".rb", ".sql", ".test", ".tcl")
STOP = set("""a an the and or how what where which when does do is are it
this that under from with than into over for what under differ what after
whether should around during then which whose through their there
sqlite requests code test tested function works happens behavior
""".split())
MAX_RESULTS = 12


class Refusal(Exception):
    pass


def invoke(args: list[str | Path], *, cwd: Path | None = None,
           timeout: int = 120, accept=(0,), max_bytes=32_000_000) -> tuple[bytes, float, int]:
    started = time.monotonic()
    try:
        proc = subprocess.run([str(x) for x in args], cwd=cwd, input=None,
                              capture_output=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired as exc:
        raise Refusal(f"command timeout: {str(args[0])} ({timeout}s)") from exc
    if len(proc.stdout) > max_bytes or len(proc.stderr) > 32768:
        raise Refusal(f"command output limit exceeded: {str(args[0])}")
    if proc.returncode not in accept:
        raise Refusal(f"command failed ({proc.returncode}): {args[0]}: "
                      + proc.stderr.decode("utf-8", "replace")[:1000])
    return proc.stdout, time.monotonic()-started, proc.returncode


def question_terms(question: str) -> list[str]:
    terms = []
    for token in WORD.findall(question.lower()):
        if len(token) >= 4 and token not in STOP and token not in terms:
            terms.append(token)
    if not terms:
        raise Refusal("question has no discriminating search terms")
    return terms[:12]


def checkout(case: dict, destination: Path) -> tuple[str, str]:
    repo, commit = case["repository"], case["commit"]
    if not SAFE_REPO.fullmatch(repo) or not HEX40.fullmatch(commit):
        raise Refusal("invalid pinned GitHub repository or SHA-1")
    invoke(["git", "init", "-q", str(destination)])
    # A public pinned SHA from one vetted official GitHub repository. No
    # user-controlled URL, shell, source hook, test or project code execution.
    invoke(["git", "-C", destination, "fetch", "--no-tags", "--depth=1", "-q",
            f"https://github.com/{repo}.git", commit], timeout=240)
    invoke(["git", "-C", destination, "checkout", "-q", "--detach", "FETCH_HEAD"])
    actual = invoke(["git", "-C", destination, "rev-parse", "HEAD"])[0].decode().strip()
    tree = invoke(["git", "-C", destination, "rev-parse", "HEAD^{tree}"])[0].decode().strip()
    if actual != commit or not HEX40.fullmatch(tree):
        raise Refusal("wrong source identity")
    if invoke(["git", "-C", destination, "status", "--porcelain"])[0].strip():
        raise Refusal("source checkout is dirty")
    return actual, tree


def oracle_validate(case: dict, directory: Path) -> dict:
    anchors = {}
    by_path = collections.defaultdict(list)
    for row in case["oracle"]:
        p = row["path"]
        if p.startswith("/") or any(part in ("", ".", "..") for part in p.split("/")):
            raise Refusal("invalid oracle path")
        if row["role"] not in ("implementation", "mode_and_error", "phase_one", "phase_two", "test"):
            raise Refusal("unsupported oracle role")
        by_path[p].append(row)
    for path, rows in by_path.items():
        source, _, _ = invoke(["git", "-C", directory, "show", f"{case['commit']}:{path}"],
                               max_bytes=16_000_000)
        lines = source.decode("utf-8", "replace").splitlines()
        for row in rows:
            for anchor in row["anchors"]:
                if not anchor or len(anchor) > 240:
                    raise Refusal("empty or oversized source oracle anchor")
                matches = [i+1 for i, line in enumerate(lines) if anchor in line]
                if not matches:
                    raise Refusal(f"oracle anchor absent from pinned source {path}: {anchor}")
                anchors[f"{path}::{anchor}"] = matches
    return {"verified_anchors": anchors, "oracle_paths": sorted(by_path)}


def baseline_git_grep(case: dict, directory: Path) -> dict:
    started = time.monotonic()
    terms = question_terms(case["question"])
    dirs = [p for p in ("src", "test", "tests", "lib", "h11") if (directory/p).is_dir()]
    if not dirs:
        dirs = ["."]
    path_terms = collections.defaultdict(set)
    term_costs = []
    for term in terms:
        output, elapsed, returncode = invoke(
            ["git", "-C", directory, "grep", "-l", "-i", "-F", "-e", term,
             "--", *dirs], timeout=120, accept=(0, 1), max_bytes=4_000_000)
        term_costs.append({"term": term, "elapsed_seconds": round(elapsed, 6),
                           "found": returncode == 0})
        for path in output.decode("utf-8").splitlines():
            if path.endswith(SOURCE_EXT):
                path_terms[path].add(term)
    # Use term breadth, not arbitrary filesystem traversal order. The exact
    # Git grep calls and source-root scope are disclosed to reviewers.
    paths = sorted(path_terms, key=lambda p: (-len(path_terms[p]), p))[:MAX_RESULTS]
    return {"selection": "GIT_GREP_DISTINCT_QUERY_TERMS_V1", "question_terms": terms,
            "paths": paths, "matched_terms": {p: sorted(path_terms[p]) for p in paths},
            "calls": len(terms), "individual_costs": term_costs,
            "elapsed_seconds": round(time.monotonic()-started, 6),
            "returned_source_count": len(paths)}


def excession_locate(case: dict, directory: Path, binary: Path) -> dict:
    args = [
        str(binary), "agent", "locate", "--repo", str(directory),
        "--rev", case["commit"], "--task", case["question"],
        "--limit", str(MAX_RESULTS), "--index-segments", "256",
        "--max-file-bytes", str(8*1024*1024),
        "--max-segment-bytes", str(32*1024*1024), "--json",
    ]
    started = time.monotonic()
    output, _, _ = invoke(args, timeout=240, max_bytes=20_000_000)
    report = json.loads(output)
    if report.get("schema") != "excession.agent-locate.v1" \
            or report.get("source_commit") != case["commit"]:
        raise Refusal("unexpected Excession report identity/schema")
    rows = report["reading_plan"]["steps"]
    paths = [r["source"]["path"] for r in rows]
    if len(rows) > MAX_RESULTS or len(set(paths)) != len(paths):
        raise Refusal("duplicated or over-budget reading plan")
    for row in rows:
        collection = row["reference_collection"]
        refs = report["context"]["test_candidates"] if collection == "context.test_candidates" \
            else report[collection]
        if refs[row["reference_index"]]["source"] != row["source"]:
            raise Refusal("reading plan points to a different source than supporting observation")
        if row["behavior_verified"]:
            raise Refusal("reading plan manufactured verified behavior")
        if row["source"]["source_tree"] != report["source_tree"]:
            raise Refusal("reading plan has stale Git source")
    return {"selection": "EXCESSION_SOURCE_FIRST_READING_PLAN_V1", "paths": paths,
            "candidate_paths": [x["source"]["path"] for x in report["candidates"]],
            "report_id": report["report_id"],
            "index": report["index"], "coverage": report["coverage"],
            "candidate_count": report["candidate_count"],
            "calls": 1, "elapsed_seconds": round(time.monotonic()-started, 6),
            "returned_source_count": len(paths),
            "original_output_bytes": len(output),
            "operational_cache": report.get("cache", {}).get("kind")}


def discovery_proxy(paths: list[str], case: dict) -> dict:
    needed = {r["path"] for r in case["oracle"]}
    tests = {r["path"] for r in case["oracle"] if r["role"] == "test"}
    found = set(paths)
    return {"required_paths": len(needed), "found_required": sorted(found & needed),
            "missing_required": sorted(needed - found),
            "required_path_recall": round(len(found & needed)/len(needed), 6),
            "test_paths": sorted(tests), "test_paths_found": sorted(found & tests),
            "all_required_found": needed <= found,
            "meaning": "PATH_RETRIEVAL_PROXY_NOT_BEHAVIORAL_UNDERSTANDING"}


def run(manifest: Path, binary: Path, output: Path) -> None:
    study = json.loads(manifest.read_text())
    if study["schema"] != "excession.code-understanding-study.v1":
        raise Refusal("unsupported study")
    if len({c["id"] for c in study["cases"]}) != len(study["cases"]):
        raise Refusal("duplicate case identifier")
    if study["arms"] != ["git", "excession"]:
        raise Refusal("only two preregistered arms are supported")
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for case in study["cases"]:
        row = {"case": case["id"], "repository": case["repository"],
               "commit": case["commit"], "status": "INCOMPLETE"}
        results.append(row)
        with tempfile.TemporaryDirectory(prefix="excession-understanding-pilot-") as temp:
            checkout_path = Path(temp)/"repository"
            try:
                actual, tree = checkout(case, checkout_path)
                row["tree"] = tree
                row["oracle_source_verification"] = oracle_validate(case, checkout_path)
                row["status"] = "SOURCE_ORACLE_VALIDATED"
                for arm in study["arms"]:
                    try:
                        metrics = (baseline_git_grep(case, checkout_path) if arm == "git"
                                   else excession_locate(case, checkout_path, binary))
                        metrics["discovery_proxy"] = discovery_proxy(metrics["paths"], case)
                        row[arm] = metrics
                    except (Refusal, KeyError, ValueError, IndexError, json.JSONDecodeError) as exc:
                        row[arm] = {"status": "INCOMPLETE", "reason": str(exc)}
                row["status"] = ("RETRIEVAL_MEASURED" if
                    all(row[a].get("discovery_proxy") for a in study["arms"])
                    else "RETRIEVAL_PARTIAL")
            except (Refusal, ValueError, KeyError) as exc:
                row["reason"] = str(exc)
        (output/f"{case['id']}.json").write_text(json.dumps(row, indent=2, sort_keys=True)+"\n")
    scored = [x for x in results if x["status"] == "RETRIEVAL_MEASURED"]
    summary = {
        "schema": "excession.code-understanding-retrieval-diagnostic.v1",
        "study_id": study["study_id"],
        "study_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
        "excession_bin_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "cases_registered": len(study["cases"]),
        "cases_retrieval_measured": len(scored),
        "gold_status": study["status"],
        "outcome_score": None,
        "outcome_unavailable_reason": "NO_BLINDED_AGENT_RESPONSES_OR_INDEPENDENT_HUMAN_ADJUDICATION",
        "proxy_not_semantic": True,
        "git_all_required_found": sum(x["git"]["discovery_proxy"]["all_required_found"] for x in scored),
        "excession_all_required_found": sum(x["excession"]["discovery_proxy"]["all_required_found"] for x in scored),
        "results": results,
    }
    (output/"summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True)+"\n")
    print(json.dumps({k:v for k,v in summary.items() if k != "results"}, indent=2))
    if len(scored) != len(study["cases"]):
        raise Refusal("some preregistered cases incomplete; retained as failures")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path,
                        default=ROOT/"benchmarks/understanding/study.json")
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    run(args.manifest.resolve(strict=True), args.bin.resolve(strict=True),
        args.out.resolve())


if __name__ == "__main__":
    try:
        main()
    except Refusal as exc:
        raise SystemExit(f"INCOMPLETE: {exc}")
