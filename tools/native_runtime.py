"""Observe the actual Linux native ABI used by Excession.

This is a runtime preflight plus a receipt, not a locked operating-system image
or a claim of reproducible native binary bytes. Never download packages here.
"""
from __future__ import annotations

import ctypes
from ctypes.util import find_library
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys

MIN_SQLITE = (3, 45, 0)


def _digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _loaded_library(name: str) -> tuple[ctypes.CDLL, Path]:
    soname = find_library(name)
    if not soname:
        raise ValueError(f"missing native library: {name}")
    library = ctypes.CDLL(soname)
    # Linux /proc/self/maps exposes the actually mapped file, not the
    # potentially different unversioned linker symlink.
    paths = set()
    for line in Path("/proc/self/maps").read_text().splitlines():
        match = line.split(maxsplit=5)
        if len(match) == 6 and match[-1].startswith("/") and soname in match[-1]:
            paths.add(Path(match[-1]).resolve())
    if len(paths) != 1:
        raise ValueError(f"cannot identify single loaded {name} shared object")
    path = paths.pop()
    if not path.is_file():
        raise ValueError("loaded library disappeared")
    return library, path


def _version(command: list[str]) -> str:
    return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).strip()


def inspect() -> dict:
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise ValueError("native runtime receipt currently supports Linux x86_64")
    if sys.version_info[:2] != (3, 13):
        raise ValueError("expected CPython 3.13 for reproducible Python wheel selection")
    sqlite, sqlite_path = _loaded_library("sqlite3")
    sqlite.sqlite3_libversion.restype = ctypes.c_char_p
    sqlite_version = sqlite.sqlite3_libversion().decode("ascii")
    parts = tuple(int(x) for x in sqlite_version.split("."))
    if parts < MIN_SQLITE:
        raise ValueError("SQLite version below pinned baseline")
    crypto, crypto_path = _loaded_library("crypto")
    crypto.OpenSSL_version.argtypes = [ctypes.c_int]
    crypto.OpenSSL_version.restype = ctypes.c_char_p
    ssl_version = crypto.OpenSSL_version(0).decode("ascii")
    if not ssl_version.startswith("OpenSSL 3."):
        raise ValueError("requires OpenSSL 3 ABI")
    return {
        "schema": "excession.native-runtime-receipt.v1",
        "platform": platform.platform(),
        "python": platform.python_version(),
        "git": _version(["git", "--version"]),
        "sqlite": {"version": sqlite_version, "path": str(sqlite_path), "sha256": _digest(sqlite_path)},
        "crypto": {"version": ssl_version, "path": str(crypto_path), "sha256": _digest(crypto_path)},
        "status": "OBSERVED_NATIVE_ABI_NOT_LOCKED",
    }


if __name__ == "__main__":
    try:
        print(json.dumps(inspect(), sort_keys=True, indent=2))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"NATIVE_ABI_REFUSAL: {error}", file=sys.stderr)
        raise SystemExit(2) from error
