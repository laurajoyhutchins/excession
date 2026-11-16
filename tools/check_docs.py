#!/usr/bin/env python3
"""Validate local Markdown targets without network access or a docs server."""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import html
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

FENCE = re.compile(r"^\s{0,3}((?:" + chr(96) + r"){3,}|~{3,})")
INLINE = re.compile(r"!?\[[^\]\n]*\]\(\s*(<[^>\n]+>|[^)\s\n]+)(?:\s+['\"][^)\n]*['\"])?\s*\)")
REFERENCE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*(<[^>\n]+>|\S+)")
IGNORED_SCHEMES = {"http", "https", "mailto", "data", "tel", "git", "ssh"}


@dataclass(frozen=True)
class Problem:
    file: Path
    line: int
    destination: str
    reason: str

    def render(self, root: Path) -> str:
        return f"{self.file.relative_to(root)}:{self.line}: {self.reason}: {self.destination}"


def prose_lines(source: str):
    """Drop fenced examples and indented code, preserving source line numbers."""
    char = ""
    count = 0
    for number, line in enumerate(source.splitlines(), 1):
        match = FENCE.match(line)
        if match:
            marker = match.group(1)
            if not char:
                char, count = marker[0], len(marker)
                continue
            if marker[0] == char and len(marker) >= count:
                char, count = "", 0
                continue
        if char or line.startswith("    ") or line.startswith("\t"):
            continue
        yield number, line


def targets(line: str):
    for match in INLINE.finditer(line):
        yield match.group(1)
    match = REFERENCE.match(line)
    if match:
        yield match.group(1)


def target_path(link: str) -> str | None:
    value = html.unescape(link.strip().strip("<>"))
    if not value or value.startswith("#") or value.startswith("//"):
        return None
    parsed = urlsplit(value)
    if parsed.scheme or parsed.netloc:
        return None
    path = unquote(parsed.path)
    return path or None


def check_document(root: Path, path: Path) -> list[Problem]:
    problems: list[Problem] = []
    for number, line in prose_lines(path.read_text(encoding="utf-8")):
        for link in targets(line):
            source = target_path(link)
            if source is None:
                continue
            if source.startswith("/"):
                problems.append(Problem(path, number, link, "repository-absolute link"))
                continue
            destination = (path.parent / source).resolve()
            if not destination.is_relative_to(root):
                problems.append(Problem(path, number, link, "outside repository"))
            elif not destination.exists():
                problems.append(Problem(path, number, link, "missing local target"))
    return problems


def markdown_files(root: Path) -> list[Path]:
    paths = [root / name for name in ("README.md", "CONTRIBUTING.md", "SECURITY.md")]
    paths.extend((root / "docs").rglob("*.md"))
    paths.extend((root / ".github" / "ISSUE_TEMPLATE").glob("*.md"))
    return sorted(path for path in paths if path.is_file())


def audit(root: Path) -> list[Problem]:
    root = root.resolve()
    return [
        problem
        for path in markdown_files(root)
        for problem in check_document(root, path)
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    root = args.root.resolve()
    problems = audit(root)
    for problem in problems:
        print(problem.render(root), file=sys.stderr)
    if problems:
        print(f"{len(problems)} local documentation link error(s)", file=sys.stderr)
        return 1
    print(f"Local targets verified in {len(markdown_files(root))} Markdown files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
