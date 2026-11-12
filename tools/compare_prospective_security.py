#!/usr/bin/env python3
"""Preregistered prospective source-pair experiment validation/comparison.

No benchmark entry or scoring predicate can be changed in response to a
scanner result without creating a different experiment version. Candidates
are source-bound alerts, not confirmed exploitability.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

MANIFEST_SCHEMA = "excession.security.prospective.v1"
OID = re.compile(r"[0-9a-f]{40}\Z")
RULES = re.compile(r"[0-9a-f]{40}\Z")
CASES = 6
PHASES = ("vulnerable", "patched")


def load_manifest(path: Path) -> dict:
    raw = path.read_bytes()
    manifest = json.loads(raw)
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise ValueError("unexpected experiment schema")
    cases = manifest.get("cases")
    if not isinstance(cases, list) or len(cases) != CASES:
        raise ValueError("expected exactly six preregistered cases")
    ids, cohorts = set(), set()
    repos, cwes = set(), set()
    for case in cases:
        cid = case["id"]
        if not isinstance(cid, str) or not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", cid):
            raise ValueError("invalid case id")
        if cid in ids:
            raise ValueError("duplicate case id")
        ids.add(cid)
        repo = case["repository"]
        repos.add(repo)
        if repo in ("Kludex/starlette", "aio-libs/aiohttp"):
            raise ValueError("development cases cannot enter prospective cohort")
        if not re.fullmatch(r"GHSA-[23456789cfghjmpqrvwx]{4}-[23456789cfghjmpqrvwx]{4}-[23456789cfghjmpqrvwx]{4}", case["advisory"]):
            raise ValueError("invalid advisory")
        cwe = case["weakness"]
        if not re.fullmatch(r"CWE-[1-9][0-9]*", cwe):
            raise ValueError("invalid weakness family")
        cwes.add(cwe)
        source_file = case["file"]
        if (source_file.startswith("/") or "\\" in source_file or
                any(x in ("", ".", "..") for x in source_file.split("/"))):
            raise ValueError("unsafe source path")
        if not case.get("oracle"):
            raise ValueError("missing independent method-level oracle")
        for phase in PHASES:
            revision = case[phase]
            if not (OID.fullmatch(revision["commit"]) and OID.fullmatch(revision["blob"])):
                raise ValueError("unverified source commit/blob identity")
            key = (repo, revision["commit"], source_file)
            if key in cohorts:
                # Same source revision can appear in more than one independent
                # advisory, but must not be treated as separate observations
                # of the same file under two scoring labels.
                raise ValueError("duplicate source revision/file in cohort")
            cohorts.add(key)
        if case["vulnerable"]["commit"] == case["patched"]["commit"]:
            raise ValueError("identical vulnerable and patched commit")
        if case["vulnerable"]["blob"] == case["patched"]["blob"]:
            raise ValueError("changed file blob identical across phase")
    if len(repos) < 3 or len(cwes) < 2:
        raise ValueError("cohort diversity below preregistered minimum")
    scanner = manifest["scanner"]
    if scanner["name"] != "semgrep" or scanner["version"] != "1.145.0":
        raise ValueError("scanner version changed")
    if scanner["rules_commit"] != "311ca4e9ba59d700624539bf658e3d29b134ee77":
        raise ValueError("frozen scanner rules changed")
    if scanner["config_subtree"] != "python" or scanner["scan_scope"] != "exact changed file only":
        raise ValueError("frozen scan selection changed")
    return manifest


def _cwes(metadata: object) -> set[str]:
    if not isinstance(metadata, dict):
        return set()
    value = metadata.get("cwe", [])
    values = value if isinstance(value, list) else [value]
    return {x for s in values if isinstance(s, str)
            for x in re.findall(r"CWE-[1-9][0-9]*", s, re.I)}


def relevant(report: dict, case: dict) -> dict:
    observations = report.get("results")
    if not isinstance(observations, list) or report.get("errors"):
        raise ValueError("Semgrep report incomplete or contains errors")
    exact = []
    all_target = []
    rules = set()
    for result in observations:
        path = result["path"].removeprefix("./")
        if path == case["file"]:
            all_target.append(result)
            if case["weakness"] in _cwes(result.get("extra", {}).get("metadata", {})):
                exact.append(result)
                rules.add(result["check_id"])
    return {"total": len(observations), "in_file": len(all_target),
            "relevant": len(exact), "relevant_rules": sorted(rules)}


def aggregate(manifest: dict, evidence_dir: Path) -> dict:
    rows = []
    errors = []
    for case in manifest["cases"]:
        phase_reports = {}
        for phase in PHASES:
            path = evidence_dir / f"prospective-{case['id']}" / phase
            try:
                metadata = json.loads((path / "metadata.json").read_text())
                oracle = json.loads((path / "oracle.json").read_text())
                status = json.loads((path / "scan-status.json").read_text())
                assert metadata["case"] == case["id"] and metadata["phase"] == phase
                assert metadata["source_commit"] == case[phase]["commit"]
                assert metadata["source_blob"] == case[phase]["blob"]
                assert oracle["source_blob"] == case[phase]["blob"]
                assert oracle["source_commit"] == case[phase]["commit"]
                assert oracle["status"] in ("OBSERVED_EXPECTED", "OBSERVED_CONTRADICTION",
                                            "INCONCLUSIVE")
                assert status["phase"] == phase and status["case"] == case["id"]
                assert status["scanner_version"] == manifest["scanner"]["version"]
                assert status["rules_commit"] == manifest["scanner"]["rules_commit"]
                assert status["scan_exit"] == 0
                report_bytes = (path / "semgrep.json").read_bytes()
                assert status["report_sha256"] == hashlib.sha256(report_bytes).hexdigest()
                report = json.loads(report_bytes)
                ingest = json.loads((path / "semgrep-ingest.json").read_text())
                audit = json.loads((path / "semgrep-audit.json").read_text())
                assert audit["independently_replayed"] is True
                assert ingest["independently_replayed"] is False
                for k in ("run_id", "source_commit", "report_sha256", "engine", "candidate_count"):
                    assert ingest[k] == audit[k]
                assert audit["source_commit"] == case[phase]["commit"]
                assert audit["report_sha256"] == status["report_sha256"]
                assert audit["engine"] == "semgrep"
                assert audit["candidate_count"] == len(report["results"])
                summary = relevant(report, case)
                summary.update({"phase": phase, "oracle": oracle["status"],
                                "ingest_replayed": True, "scan_status": "COMPLETE"})
                phase_reports[phase] = summary
            except Exception as exc:
                reason = f"{case['id']}/{phase}: {type(exc).__name__}: {exc}"
                errors.append(reason)
                phase_reports[phase] = {"phase": phase, "scan_status": "INCOMPLETE",
                                        "error": reason}
        rows.append({"case": case["id"], "repository": case["repository"],
                     "weakness": case["weakness"], "advisory": case["advisory"],
                     **phase_reports})
    # Unfinished jobs must be visible in result, not silently omitted.
    return {"schema": "excession.security.prospective.results.v1",
            "case_count": len(rows), "complete_pairs": sum(
                r["vulnerable"]["scan_status"] == "COMPLETE"
                and r["patched"]["scan_status"] == "COMPLETE" for r in rows),
            "oracle_confirmed_pairs": sum(
                r["vulnerable"].get("oracle") == "OBSERVED_EXPECTED"
                and r["patched"].get("oracle") == "OBSERVED_EXPECTED" for r in rows),
            "errors": errors, "cases": rows}


def markdown(report: dict) -> str:
    lines = [
        "# Prospective security detection experiment",
        "",
        "Frozen, preregistered Semgrep scan of six historical vulnerability/fix pairs.",
        "No scanner findings were used to select or modify the cases.",
        "",
        "| Case | CWE | Vulnerable relevant hits | Patched relevant hits | Native oracle v/p | Scan |",
        "|---|---|---:|---:|---|---|",
    ]
    for case in report["cases"]:
        v, p = case["vulnerable"], case["patched"]
        lines.append("| " + " | ".join([
            case["case"], case["weakness"],
            str(v.get("relevant", "incomplete")), str(p.get("relevant", "incomplete")),
            f"{v.get('oracle','?')} / {p.get('oracle','?')}",
            "complete" if v["scan_status"] == p["scan_status"] == "COMPLETE" else "INCOMPLETE",
        ]) + " |")
    lines += ["", f"Complete scan pairs: {report['complete_pairs']}/6",
              f"Method-level oracle-confirmed pairs: {report['oracle_confirmed_pairs']}/6",
              "", "**Limitations:** Exact CWE metadata plus file matching measures scanner",
              "candidate production for these labeled cases only. It does not establish",
              "exploitability or general precision/recall. A no-hit observation is not",
              "a safety verdict. Method-level oracles do not execute an entire web server",
              "or distributed application.",
              ""]
    if report["errors"]:
        lines += ["### Incomplete evidence", ""] + [f"- {error}" for error in report["errors"]]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--markdown", type=Path)
    args = parser.parse_args()
    manifest = load_manifest(args.manifest)
    if not args.evidence:
        print(f"frozen cohort: {len(manifest['cases'])} cases")
        return
    result = aggregate(manifest, args.evidence)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    args.markdown.write_text(markdown(result))
    print(markdown(result))
    if result["errors"]:
        raise SystemExit(2)


if __name__ == "__main__":
    main()
