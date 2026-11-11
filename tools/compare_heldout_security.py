#!/usr/bin/env python3
"""Compare preregistered upstream security scans without manufacturing accuracy."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def relative_match(candidate: str, filename: str) -> bool:
    """Only exact upstream source file locations count as file-local alerts."""
    candidate = candidate.replace("\\", "/").removeprefix("./")
    return candidate == filename or candidate.endswith("/" + filename)


def semgrep_observations(payload: dict, filename: str) -> dict:
    if not isinstance(payload.get("results"), list):
        raise ValueError("Semgrep report has no results list")
    if payload.get("errors"):
        raise ValueError(f"Semgrep reported configuration/scan errors: {payload['errors']!r}")
    all_hits = payload["results"]
    targeted = [
        x for x in all_hits if relative_match(x.get("path", ""), filename)
    ]
    return {
        "total_alerts": len(all_hits),
        "target_file_alerts": len(targeted),
        "target_rules": sorted({x["check_id"] for x in targeted}),
        "target_locations": sorted({
            f"{x['path']}:{x.get('start', {}).get('line', '?')}"
            for x in targeted
        }),
    }


def codeql_observations(payload: dict, filename: str) -> dict:
    runs = payload.get("runs")
    if not isinstance(runs, list) or not runs:
        raise ValueError("CodeQL SARIF has no runs")
    hits: list[dict] = []
    for run in runs:
        if not isinstance(run.get("results"), list):
            raise ValueError("CodeQL SARIF run has no results")
        hits.extend(run["results"])
    targeted: list[tuple[str, str]] = []
    for hit in hits:
        for location in hit.get("locations", []):
            physical = location.get("physicalLocation", {})
            path = physical.get("artifactLocation", {}).get("uri", "")
            if relative_match(path, filename):
                targeted.append((hit.get("ruleId", "<no-rule>"),
                                 f"{path}:{physical.get('region', {}).get('startLine', '?')}"))
                break
    return {
        "total_alerts": len(hits),
        "target_file_alerts": len(targeted),
        "target_rules": sorted({rule for rule, _ in targeted}),
        "target_locations": sorted({pos for _, pos in targeted}),
    }


def read_json(path: Path) -> dict:
    if not path.is_file():
        raise ValueError(f"missing evidence file: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def summarize(manifest: dict, evidence_dir: Path) -> dict:
    if manifest.get("schema") != "excession.security.heldout.v1":
        raise ValueError("unsupported preregistration manifest")
    result: list[dict] = []
    for case in manifest["cases"]:
        for phase in ("vulnerable", "patched"):
            folder = evidence_dir / f"heldout-{case['id']}-{phase}"
            metadata = read_json(folder / "metadata.json")
            if (metadata.get("case") != case["id"] or metadata.get("phase") != phase
                or metadata.get("source_commit") != case[phase]["commit"]
                or metadata.get("source_blob") != case[phase]["blob"]):
                raise ValueError(f"source/provenance mismatch: {folder}")
            oracle = read_json(folder / "native-oracle.json")
            if (oracle.get("source_commit") != case[phase]["commit"]
                or oracle.get("source_blob") != case[phase]["blob"]
                or oracle.get("behavior", {}).get("outside_read") is not (phase == "vulnerable")
                or oracle.get("behavior", {}).get("normal_read") is not True):
                raise ValueError(f"native oracle mismatch: {folder}")
            for engine, report, parse in (
                ("semgrep", "semgrep.json", semgrep_observations),
                ("codeql", "codeql.sarif", codeql_observations),
            ):
                receipt = read_json(folder / f"{engine}-audit.json")
                if receipt.get("independently_replayed") is not True:
                    raise ValueError(f"missing independently replayed {engine} audit: {folder}")
                _ = read_json(folder / f"{engine}-ingest.json")
                obs = parse(read_json(folder / report), case["file"])
                result.append({
                    "case": case["id"], "phase": phase,
                    "engine": engine, "source_commit": case[phase]["commit"],
                    "behavioral_outside_read": oracle["behavior"]["outside_read"],
                    **obs,
                })
    comparisons = []
    for case in manifest["cases"]:
        for engine in ("semgrep", "codeql"):
            vuln = next(x for x in result if x["case"] == case["id"]
                        and x["engine"] == engine and x["phase"] == "vulnerable")
            fixed = next(x for x in result if x["case"] == case["id"]
                         and x["engine"] == engine and x["phase"] == "patched")
            comparisons.append({
                "case": case["id"], "engine": engine,
                "vulnerable_target_file_alerts": vuln["target_file_alerts"],
                "patched_target_file_alerts": fixed["target_file_alerts"],
                "shared_rule_ids": sorted(set(vuln["target_rules"]) & set(fixed["target_rules"])),
                "interpretation": "source-file alert coverage only; patched alert not automatically false positive",
            })
    return {
        "schema": "excession.security.heldout-comparison.v1",
        "complete": len(result) == 2 * len(manifest["cases"]) * 2,
        "cases": result, "comparisons": comparisons,
        "claim_boundary": manifest["claim_boundary"],
    }


def markdown(report: dict) -> str:
    rows = [
        "## Preregistered historical vulnerability/fix scans",
        "",
        "| Case | Scanner | Vulnerable file alerts | Patched file alerts |",
        "|---|---|---:|---:|",
    ]
    for x in report["comparisons"]:
        rows.append(f"| {x['case']} | {x['engine']} | "
                    f"{x['vulnerable_target_file_alerts']} | {x['patched_target_file_alerts']} |")
    rows += ["", "Counts are file-local alerts, not distinct vulnerabilities.",
             "A patched-source alert is not automatically a false positive.",
             "Upstream method behavior was exercised independently of scanner results.",
             "No precision, recall, exploitability or deployment-coverage estimate is claimed."]
    return "\n".join(rows) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    args = parser.parse_args()
    report = summarize(read_json(args.manifest), args.evidence)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown.write_text(markdown(report), encoding="utf-8")
    print(markdown(report))


if __name__ == "__main__":
    main()
