"""Generate and verify local Tree-sitter grammar binary inventory.

Recording is explicitly separate from verification. Only the trusted setup
phase may prefetch grammars; verify never downloads, executes, or imports them.
The lock is platform-specific and excludes mutable remote manifest metadata.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import stat
import sys

LANGUAGES = ("c", "go", "java", "javascript", "python", "rust", "typescript")
SCHEMA = "excession.grammar-artifacts.v1"
PACKAGE = "tree-sitter-language-pack"
SUFFIXES = {".so", ".dylib", ".dll"}


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _files(root: Path) -> dict[str, str]:
    """Hash only native grammar libraries, fail closed on links or empty caches."""
    if not root.is_dir() or root.is_symlink():
        raise ValueError("grammar cache directory missing or symlinked")
    result = {}
    for item in sorted(root.rglob("*")):
        if item.is_symlink():
            raise ValueError("symlink in grammar cache")
        if not item.is_file() or item.suffix not in SUFFIXES:
            continue
        if not stat.S_ISREG(item.stat().st_mode):
            raise ValueError("non-regular grammar artifact")
        relative = item.relative_to(root).as_posix()
        if relative.startswith("/") or any(x in ("", ".", "..") for x in relative.split("/")):
            raise ValueError("unsafe grammar artifact path")
        result[relative] = _sha256(item)
    if not result:
        raise ValueError("no native grammar libraries found")
    return result


def make_inventory(root: Path, version: str, downloaded: set[str]) -> dict:
    absent = set(LANGUAGES) - downloaded
    if absent:
        raise ValueError("missing prefetched grammars: " + ", ".join(sorted(absent)))
    return {
        "schema": SCHEMA,
        "package": PACKAGE,
        "version": version,
        "platform": "linux-x86_64-gnu",
        "languages": list(LANGUAGES),
        "libraries": _files(root),
    }


def verify_inventory(expected: dict, root: Path, version: str, downloaded: set[str]) -> None:
    if not isinstance(expected, dict) or set(expected) != {
        "schema", "package", "version", "platform", "languages", "libraries"
    }:
        raise ValueError("invalid grammar inventory fields")
    if (expected["schema"] != SCHEMA or expected["package"] != PACKAGE
            or expected["version"] != version or expected["platform"] != "linux-x86_64-gnu"
            or expected["languages"] != list(LANGUAGES)):
        raise ValueError("wrong grammar inventory identity")
    if not isinstance(expected["libraries"], dict) or not expected["libraries"]:
        raise ValueError("invalid grammar inventory libraries")
    if any(not isinstance(k, str) or not isinstance(v, str)
           or len(v) != 64 or any(c not in "0123456789abcdef" for c in v)
           for k, v in expected["libraries"].items()):
        raise ValueError("invalid library digests")
    observed = make_inventory(root, version, downloaded)
    if observed != expected:
        raise ValueError("grammar artifact bytes or inventory differ from lock")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("record", "verify"))
    parser.add_argument("--lock", type=Path, default=Path("requirements/locks/grammar-linux-x86_64.json"))
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
        parser.error("grammar artifact lock currently supports Linux x86_64 only")
    try:
        import tree_sitter_language_pack as pack
        from importlib.metadata import version
        # The package's effective cache dir already honors
        # TREE_SITTER_LANGUAGE_PACK_CACHE_DIR; never download during verify.
        root = Path(pack.cache_dir())
        downloaded = set(pack.downloaded_languages())
        installed = version(PACKAGE)
        if args.action == "record":
            inventory = make_inventory(root, installed, downloaded)
            print(json.dumps(inventory, sort_keys=True, indent=2))
        else:
            expected = json.loads(args.lock.read_text(encoding="utf-8"))
            verify_inventory(expected, root, installed, downloaded)
            print("Grammar binary inventory matches committed SHA-256 lock")
        return 0
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"GRAMMAR_LOCK_REFUSAL: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
