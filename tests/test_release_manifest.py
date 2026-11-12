"""Release manifest verification exercises actual compressed fixture archives."""
from __future__ import annotations
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

from tools.release_manifest import build_manifest


def create_archive(dist: Path, target: str, version: str = "0.2.0") -> Path:
    archive = dist / f"excession-v{version}-{target}.tar.gz"
    with tarfile.open(archive, "w:gz") as out:
        payload = b"synthetic parser-sidecar fixture"
        member = tarfile.TarInfo("excession/lib/typescript.js")
        member.size = len(payload)
        out.addfile(member, io.BytesIO(payload))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (dist / (archive.name + ".sha256")).write_text(
        f"{digest}  {archive.name}\n", encoding="ascii"
    )
    return archive


class ManifestTests(unittest.TestCase):
    def test_exact_archive_provenance_without_false_attestation(self):
        with tempfile.TemporaryDirectory() as directory:
            dist = Path(directory)
            archive = create_archive(dist, "x86_64-unknown-linux-musl")
            manifest = build_manifest(dist, "0.2.0", "a" * 40)
            assert manifest["schema"] == "excession.release-manifest.v1"
            assert manifest["attestation_status"] == "NOT_ATTESTED"
            assert manifest["source_commit"] == "a" * 40
            assert manifest["assets"][0]["sha256"] == hashlib.sha256(archive.read_bytes()).hexdigest()
            assert len(manifest["assets"][0]["sidecar_sha256"]) == 64
            json.dumps(manifest, sort_keys=True)

    def test_mutated_archive_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            dist = Path(directory)
            archive = create_archive(dist, "x86_64-unknown-linux-musl")
            archive.write_bytes(archive.read_bytes() + b"tamper")
            with self.assertRaisesRegex(ValueError, "checksum"):
                build_manifest(dist, "0.2.0", "a" * 40)

    def test_wrong_version_and_invalid_commit_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            dist = Path(directory)
            create_archive(dist, "x86_64-unknown-linux-musl")
            with self.assertRaisesRegex(ValueError, "version"):
                build_manifest(dist, "0.3.0", "a" * 40)
            with self.assertRaisesRegex(ValueError, "exact lowercase"):
                build_manifest(dist, "0.2.0", "HEAD")

    def test_missing_sidecar_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            dist = Path(directory)
            create_archive(dist, "x86_64-unknown-linux-musl")
            archive = next(dist.glob("*.tar.gz"))
            # Replace package contents with an invalid empty tar, update checksum.
            with tarfile.open(archive, "w:gz"):
                pass
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            (dist / (archive.name + ".sha256")).write_text(
                f"{digest}  {archive.name}\n", encoding="ascii"
            )
            with self.assertRaises(KeyError):
                build_manifest(dist, "0.2.0", "a" * 40)


if __name__ == "__main__":
    unittest.main()
