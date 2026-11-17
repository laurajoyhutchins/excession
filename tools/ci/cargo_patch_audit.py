#!/usr/bin/env python3
"""Fail-closed audit of an exact-Git Cargo.lock candidate.

Only same-package, same-source, patch-level updates with unchanged dependency
edges are eligible for the unattended CI merge gate. Not a supply-chain proof.
"""
from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
import sys
import tomllib

import improvement_cycle as common

SCHEMA = "excession.cargo-lock-patch-audit.v1"
LOCK = "core/Cargo.lock"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def version(value: str) -> tuple[int, int, int]:
    if not isinstance(value, str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise common.Refused(f"non-stable or unsupported Cargo semver: {value!r}")
    return tuple(map(int, value.split(".")))


def grouped(doc: dict) -> dict[tuple[str, str], list[dict]]:
    output = defaultdict(list)
    items = doc.get("package")
    if not isinstance(items, list):
        raise common.Refused("Cargo lockfile has no package list")
    for item in items:
        if not isinstance(item, dict) or not isinstance(item.get("name"), str):
            raise common.Refused("invalid Cargo package record")
        output[(item["name"], item.get("source", ""))].append(item)
    return output


def patch_changes(old_bytes: bytes, new_bytes: bytes) -> list[dict]:
    try:
        a = tomllib.loads(old_bytes.decode("utf-8"))
        b = tomllib.loads(new_bytes.decode("utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as exc:
        raise common.Refused(f"invalid Cargo.lock encoding or syntax: {exc}") from exc
    if {k: v for k, v in a.items() if k != "package"} != {k: v for k, v in b.items() if k != "package"}:
        raise common.Refused("Cargo.lock global metadata changed")
    old = grouped(a)
    new = grouped(b)
    if old.keys() != new.keys():
        raise common.Refused("Cargo package identities or sources changed")
    updated = []
    for key in sorted(old):
        left, right = old[key], new[key]
        if len(left) != len(right):
            raise common.Refused(f"package multiplicity changed: {key}")
        # Parsing all versions is deliberate: prerelease/nonstandard packages
        # require review rather than optimistic version sorting.
        left = sorted(left, key=lambda item: version(item["version"]))
        right = sorted(right, key=lambda item: version(item["version"]))
        for before, after in zip(left, right):
            old_version, new_version = version(before["version"]), version(after["version"])
            if old_version == new_version:
                if before != after:
                    raise common.Refused(f"same-version metadata or checksum changed: {key}")
                continue
            if key[1] != REGISTRY or old_version[:2] != new_version[:2] or old_version[2] >= new_version[2]:
                raise common.Refused(f"not a registry patch-only version increase: {key}")
            if {k: v for k, v in before.items() if k not in ("version", "checksum")} != {
                k: v for k, v in after.items() if k not in ("version", "checksum")
            }:
                raise common.Refused(f"package dependency edges or metadata changed: {key}")
            if not before.get("checksum") or not after.get("checksum") or before["checksum"] == after["checksum"]:
                raise common.Refused(f"changed package requires a different registry checksum: {key}")
            updated.append({"name": key[0], "source": key[1],
                            "from": before["version"], "to": after["version"]})
    if not updated:
        raise common.Refused("lockfile changed without a qualifying patch update")
    return updated


def verify(repo: Path, base: str, candidate: str) -> dict:
    base, candidate = common.pin(repo, base), common.pin(repo, candidate)
    parents = common.git(repo, "rev-list", "--parents", "-n", "1", candidate).decode().split()
    if parents != [candidate, base]:
        raise common.Refused("Cargo update candidate must be a direct child of accepted main")
    if common.git_blob(repo, base, common.CATALOG) != common.git_blob(repo, candidate, common.CATALOG):
        raise common.Refused("candidate changed its own improvement authority")
    common.validated_catalog(common.git_blob(repo, base, common.CATALOG))
    diff = common.git(repo, "diff", "--no-renames", "--name-status", "-z", base, candidate)
    if diff != b"M\0core/Cargo.lock\0":
        raise common.Refused("Cargo patch integration may modify only the existing core/Cargo.lock blob")
    old_bytes = common.git_blob(repo, base, LOCK)
    new_bytes = common.git_blob(repo, candidate, LOCK)
    changes = patch_changes(old_bytes, new_bytes)
    for rev in (base, candidate):
        raw = common.git(repo, "ls-tree", "-z", rev, "--", LOCK)
        if not raw.startswith(b"100644 blob ") or not raw.endswith(b"\tcore/Cargo.lock\0"):
            raise common.Refused("Cargo.lock must be a regular non-executable Git blob")
    return {
        "schema": SCHEMA, "recipe_id": "native-cargo-lock-refresh",
        "base_commit": base, "candidate_commit": candidate,
        "base_tree": common.git(repo, "rev-parse", base + "^{tree}").decode().strip(),
        "candidate_tree": common.git(repo, "rev-parse", candidate + "^{tree}").decode().strip(),
        "base_lock_sha256": hashlib.sha256(old_bytes).hexdigest(),
        "candidate_lock_sha256": hashlib.sha256(new_bytes).hexdigest(),
        "patch_updates": changes,
        "status": "PATCH_ONLY_NO_OBSERVED_LOCK_GRAPH_CHANGE_PENDING_CI",
        "remaining_checks": ["Cargo tests", "Verify", "Excession GitHub evidence", "branch-protection"],
        "ready_to_merge": False, "dependency_safety_proved": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--base", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        data = verify(args.repo.resolve(strict=True), args.base, args.candidate)
        result = json.dumps(data, indent=2, sort_keys=True) + "\n"
        if args.report:
            args.report.write_text(result)
        print(result, end="")
        return 0
    except (OSError, UnicodeError, common.Refused) as exc:
        print(f"Cargo patch update REFUSED: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
