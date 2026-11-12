#!/usr/bin/env python3
"""Verify published archives against exact source and produce a release manifest.

Checksums are integrity records, not authentication signatures. This script
never claims build attestation, execution trust or semantic correctness.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib
import zipfile

OID = re.compile(r"[0-9a-f]{40}\Z")
TARGET = re.compile(r"[a-z0-9_][a-z0-9_.-]*\Z")
SIDECAR = "excession/lib/typescript.js"


def sha256(contents: bytes) -> str:
    return hashlib.sha256(contents).hexdigest()


def target_from_name(filename: str, version: str) -> str:
    prefix = f"excession-v{version}-"
    suffix = ".zip" if filename.endswith(".zip") else ".tar.gz"
    if not filename.startswith(prefix) or not filename.endswith(suffix):
        raise ValueError(f"unrecognized archive version or format: {filename}")
    target = filename[len(prefix):-len(suffix)]
    if not TARGET.fullmatch(target):
        raise ValueError(f"invalid release target: {target!r}")
    return target


def sidecar_sha256(archive: Path) -> str:
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            return sha256(package.read(SIDECAR))
    with tarfile.open(archive, "r:gz") as package:
        member = package.getmember(SIDECAR)
        source = package.extractfile(member)
        if source is None or not member.isfile():
            raise ValueError(f"parser sidecar not a regular file in {archive.name}")
        return sha256(source.read())


def build_manifest(dist: Path, version: str, source_commit: str) -> dict:
    if OID.fullmatch(source_commit) is None:
        raise ValueError("release source must be an exact lowercase Git SHA-1 commit")
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[a-z0-9.-]+)?", version) is None:
        raise ValueError("invalid release version")
    archives = sorted((*dist.glob("excession-v*.tar.gz"), *dist.glob("excession-v*.zip")))
    if not archives:
        raise ValueError("no release archives")
    assets = []
    seen = set()
    for archive in archives:
        target = target_from_name(archive.name, version)
        if target in seen:
            raise ValueError(f"duplicate archive target: {target}")
        seen.add(target)
        checksum = archive.parent / (archive.name + ".sha256")
        expected = checksum.read_text(encoding="ascii").strip()
        actual = sha256(archive.read_bytes())
        if expected != f"{actual}  {archive.name}":
            raise ValueError(f"checksum missing, malformed or incorrect: {archive.name}")
        assets.append({
            "target": target,
            "filename": archive.name,
            "sha256": actual,
            "sidecar_sha256": sidecar_sha256(archive),
        })
    return {
        "schema": "excession.release-manifest.v1",
        "version": version,
        "source_commit": source_commit,
        "assets": assets,
        "attestation_status": "NOT_ATTESTED",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--tag", help="verify exact tag and Cargo version on release jobs")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    version = tomllib.loads((repo / "core/Cargo.toml").read_text())["package"]["version"]
    checkout = subprocess.check_output(
        ["git", "rev-parse", "--verify", "HEAD"], cwd=repo, text=True
    ).strip()
    if checkout != args.source_commit:
        raise SystemExit(f"source mismatch: checkout {checkout} != {args.source_commit}")
    if args.tag is not None:
        if args.tag != f"v{version}":
            raise SystemExit(f"tag {args.tag} does not match Cargo version {version}")
        tagged = subprocess.check_output([
            "git", "rev-parse", "--verify", f"refs/tags/{args.tag}^{{commit}}"
        ], cwd=repo, text=True).strip()
        if tagged != checkout:
            raise SystemExit("tag points at a different source commit")
    manifest = build_manifest(args.dist, version, checkout)
    destination = args.dist / "release-manifest.json"
    destination.write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    print(f"Validated {len(manifest['assets'])} exact-source release archive(s): {destination}")


if __name__ == "__main__":
    main()
