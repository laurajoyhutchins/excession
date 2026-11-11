#!/usr/bin/env python3
"""Build deterministic platform archives around the native Excession executable.

Only the JavaScript parser sidecar is packaged. Python, Node, Git and native
SQLite requirements are described in docs/PORTABILITY.md.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
from pathlib import Path
import tarfile
import tomllib
import zipfile


def build_archive(binary: Path, target: str, out_dir: Path) -> tuple[Path, Path]:
    repo = Path(__file__).resolve().parent.parent
    package = tomllib.loads((repo / "core/Cargo.toml").read_text(encoding="utf-8"))
    version = package["package"]["version"]
    parser = repo / "node_modules/typescript"
    parser_version = package_version(parser)
    if parser_version != "5.8.3":
        raise ValueError(f"expected locked TypeScript 5.8.3, found {parser_version}")
    binary_name = "excession.exe" if "windows" in target else "excession"
    if binary.name != binary_name:
        raise ValueError(f"expected {binary_name}, found {binary.name}")
    if not binary.is_file():
        raise FileNotFoundError(binary)

    files = {
        f"excession/{binary_name}": (binary.read_bytes(), 0o755),
        "excession/LICENSE": ((repo / "LICENSE").read_bytes(), 0o644),
        "excession/README.md": ((repo / "README.md").read_bytes(), 0o644),
        "excession/PORTABILITY.md": ((repo / "docs/PORTABILITY.md").read_bytes(), 0o644),
        "excession/THIRD_PARTY.md": ((repo / "THIRD_PARTY.md").read_bytes(), 0o644),
        "excession/lib/typescript.js": ((parser / "lib/typescript.js").read_bytes(), 0o644),
        "excession/lib/TypeScript-LICENSE.txt": ((parser / "LICENSE.txt").read_bytes(), 0o644),
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"excession-v{version}-{target}"
    archive = out_dir / (stem + (".zip" if "windows" in target else ".tar.gz"))
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as out:
            for name, (data, mode) in sorted(files.items()):
                entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = (0o100000 | mode) << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                out.writestr(entry, data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    else:
        with archive.open("wb") as stream:
            with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w|") as out:
                    for name, (data, mode) in sorted(files.items()):
                        entry = tarfile.TarInfo(name)
                        entry.size = len(data)
                        entry.mode = mode
                        entry.uid = entry.gid = 0
                        entry.uname = entry.gname = ""
                        entry.mtime = 0
                        out.addfile(entry, io.BytesIO(data))
    checksum = out_dir / (archive.name + ".sha256")
    checksum.write_text(
        f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n",
        encoding="ascii",
    )
    return archive, checksum


def package_version(parser: Path) -> str:
    import json

    return str(json.loads((parser / "package.json").read_text(encoding="utf-8"))["version"])


def main() -> None:
    cli = argparse.ArgumentParser()
    cli.add_argument("--binary", type=Path, required=True)
    cli.add_argument("--target", required=True)
    cli.add_argument("--output", type=Path, default=Path("dist"))
    args = cli.parse_args()
    archive, checksum = build_archive(args.binary, args.target, args.output)
    print(archive)
    print(checksum)


if __name__ == "__main__":
    main()
