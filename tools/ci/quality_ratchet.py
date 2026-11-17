#!/usr/bin/env python3
"""Fail on newly introduced, evidence-backed quality violations in exact Git trees.

Only changed tracked paths are examined. The baseline is the previous Git tree,
not a mutable counter or a report produced by the candidate under test.
"""
from __future__ import annotations

import argparse
from collections import Counter
import fnmatch
import json
from pathlib import Path
import re
import subprocess
import sys

CATALOG = "quality/ratchet.json"


class Refusal(Exception):
    """Invalid input or unverifiable repository state."""


def git(repo: Path, *args: str, missing: bool = False) -> bytes | None:
    proc = subprocess.run(["git", "-C", str(repo), *args], capture_output=True)
    if proc.returncode:
        if missing and proc.returncode == 128 and b"does not exist in" in proc.stderr:
            return None
        raise Refusal(f"git {args[0]} failed: {proc.stderr.decode(errors='replace').strip()}")
    return proc.stdout


def commit(repo: Path, rev: str) -> str:
    return git(repo, "rev-parse", "--verify", "--end-of-options", rev + "^{commit}").decode().strip()


def blob(repo: Path, rev: str, path: str, *, missing: bool = False) -> str | None:
    if missing and not git(repo, "ls-tree", "--name-only", rev, "--", path):
        return None
    value = git(repo, "show", f"{rev}:{path}")
    return None if value is None else value.decode("utf-8")


def catalog(repo: Path, rev: str, *, optional: bool = False) -> list[dict]:
    raw = blob(repo, rev, CATALOG, missing=optional)
    if raw is None:
        return []
    try:
        doc = json.loads(raw)
        if doc["schema"] != "excession.quality-ratchet.v1" or not isinstance(doc["rules"], list):
            raise ValueError("unexpected schema or rules")
        rules, ids = [], set()
        for rule in doc["rules"]:
            if set(rule) != {"id", "paths", "pattern", "rationale", "evidence"}:
                raise ValueError("rule fields must be exact")
            if (not isinstance(rule["id"], str) or not re.fullmatch(r"[a-z][a-z0-9-]+", rule["id"])
                    or rule["id"] in ids):
                raise ValueError("invalid or duplicate rule ID")
            if (not isinstance(rule["paths"], list) or not rule["paths"]
                    or any(not isinstance(p, str) or not p or p.startswith("/") or ".." in p.split("/") for p in rule["paths"])):
                raise ValueError("invalid scope")
            if any(not isinstance(rule[k], str) or not rule[k].strip() for k in ("pattern", "rationale", "evidence")):
                raise ValueError("missing rule explanation")
            re.compile(rule["pattern"])
            ids.add(rule["id"])
            rules.append(rule)
        return rules
    except (KeyError, TypeError, ValueError, re.error) as exc:
        raise Refusal(f"invalid ratchet catalog at {rev}: {exc}") from exc


def changed_paths(repo: Path, base: str, head: str) -> list[tuple[str, str | None]]:
    raw = git(repo, "diff", "--name-status", "--find-renames", "-z", base, head)
    parts = raw.split(b"\0")
    result = []
    i = 0
    while i < len(parts) - 1:
        status = parts[i].decode("ascii")
        old = parts[i + 1].decode("utf-8")
        i += 2
        if status.startswith(("R", "C")):
            current = parts[i].decode("utf-8")
            i += 1
        elif status == "D":
            continue
        else:
            current = old
        result.append((current, old if status != "A" else None))
    return result


def findings(source: str | None, rule: dict) -> Counter:
    if source is None:
        return Counter()
    regex = re.compile(rule["pattern"])
    return Counter(line.strip() for line in source.splitlines() if regex.search(line))


def check(repo: Path, base: str, head: str) -> dict:
    before = {r["id"]: r for r in catalog(repo, base, optional=True)}
    after = {r["id"]: r for r in catalog(repo, head)}
    altered = sorted(rid for rid, rule in before.items() if after.get(rid) != rule)
    violations = []
    paths = changed_paths(repo, base, head)
    for path, old_path in paths:
        matching = [r for r in after.values() if any(fnmatch.fnmatchcase(path, g) for g in r["paths"])]
        if not matching:
            continue
        candidate = blob(repo, head, path)
        prior = blob(repo, base, old_path) if old_path else None
        for rule in matching:
            old_counts = findings(prior, rule)
            for line_number, line in enumerate(candidate.splitlines(), 1):
                normalized = line.strip()
                if re.search(rule["pattern"], line):
                    if old_counts[normalized]:
                        old_counts[normalized] -= 1
                    else:
                        violations.append({"rule": rule["id"], "path": path,
                                           "line": line_number, "text": normalized[:240]})
    return {"schema": "excession.quality-ratchet-report.v1", "base": base,
            "candidate": head, "changed_files": len(paths), "rules": len(after),
            "altered_or_removed_rules": altered, "new_violations": violations,
            "status": "PASS" if not (altered or violations) else "FAIL"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check"])
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--base", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        repo = args.repo.resolve(strict=True)
        base = commit(repo, args.base)
        head = commit(repo, args.candidate)
        result = check(repo, base, head)
    except (OSError, UnicodeError, Refusal) as exc:
        print(f"ratchet REFUSED: {exc}", file=sys.stderr)
        return 2
    output = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.report:
        args.report.write_text(output, encoding="utf-8")
    print(output, end="")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
