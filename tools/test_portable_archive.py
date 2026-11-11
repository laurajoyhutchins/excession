#!/usr/bin/env python3
"""Smoke-test a release archive extracted outside the source checkout."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import zipfile


def run(*args: str, cwd: Path) -> str:
    output = subprocess.run(
        args, cwd=cwd, check=True, capture_output=True, text=True, encoding="utf-8"
    )
    return output.stdout


def verify(archive: Path) -> None:
    expected = (archive.parent / (archive.name + ".sha256")).read_text(encoding="ascii")
    digest, filename = expected.strip().split("  ", 1)
    assert filename == archive.name
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
    with tempfile.TemporaryDirectory(prefix="excession portable archive Ω ") as folder:
        root = Path(folder)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as z:
                z.extractall(root)
        else:
            with tarfile.open(archive, "r:gz") as tar:
                tar.extractall(root, filter="data")
        installed = root / "excession"
        binary = installed / ("excession.exe" if os.name == "nt" else "excession")
        assert binary.is_file()
        assert (installed / "lib/typescript.js").is_file()
        assert (installed / "lib/TypeScript-LICENSE.txt").is_file()
        source = root / "independent Unicode repo"
        (source / "src").mkdir(parents=True)
        (source / "src/tool.js").write_text(
            "export function answer() { return 42; }\n", encoding="utf-8"
        )
        run("git", "init", "-q", str(source), cwd=root)
        run("git", "-C", str(source), "add", "--all", cwd=root)
        run(
            "git", "-C", str(source), "-c", "user.name=Excession Portable Test",
            "-c", "user.email=portable@example.invalid",
            "commit", "-qm", "fixture", cwd=root,
        )
        assert "excession " in run(str(binary), "--version", cwd=source)
        assert "excession index" in run(str(binary), "--help", cwd=source)
        output = run(
            str(binary), "index", "--repo", str(source),
            "--db", str(root / "portable.sqlite"), "--name", "portable-js",
            "--rev", "HEAD", "--roots", "src", cwd=source,
        )
        assert "GIT_TREE_INDEXED" in output


def main() -> None:
    folder = Path(sys.argv[1])
    archives = list(folder.glob("*.zip")) + list(folder.glob("*.tar.gz"))
    if len(archives) != 1:
        raise SystemExit(f"expected one portable archive, found {len(archives)}")
    verify(archives[0])
    print(f"Verified {archives[0].name}: checksums, executable, JS parser, Git and SQLite")


if __name__ == "__main__":
    main()
