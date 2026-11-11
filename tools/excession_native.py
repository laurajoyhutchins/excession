#!/usr/bin/env python3
"""Compatibility adapter for the Rust-owned native dependency discovery CLI.

Git provenance, historical source staging, toolchain identity, and contracts
are implemented by `excession evidence discover-native`. This thin wrapper
preserves the previous Python test/benchmark entrypoint without duplicating
those security-sensitive operations.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from excession_discover import Tree, Unsupported


def binary():
    override = os.environ.get('EXCESSION_BIN')
    if override:
        return Path(override)
    repo = Path(__file__).resolve().parents[1]
    local = repo / 'core' / 'target' / 'debug' / 'excession'
    if local.is_file():
        return local
    found = shutil.which('excession')
    if found:
        return Path(found)
    raise Unsupported('build the Rust excession CLI or set EXCESSION_BIN')


def discover(repo, revision, entry, language):
    argv = [str(binary()), 'evidence', 'discover-native', '--repo', str(repo),
            '--rev', revision, '--entry', entry, '--language', language,
            '--trusted-local', 'true']
    try:
        completed = subprocess.run(argv, capture_output=True, timeout=65)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Unsupported(f'Rust native discovery unavailable: {exc}') from exc
    if completed.returncode:
        raise Unsupported(completed.stderr[:700].decode('utf-8', errors='replace'))
    if len(completed.stdout) > 1024 * 1024:
        raise Unsupported('native declaration exceeds output bound')
    try:
        result = json.loads(completed.stdout)
        return result['declaration'], result['stats']
    except (ValueError, KeyError, TypeError) as exc:
        raise Unsupported('Rust native discovery returned invalid response') from exc


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repo', type=Path)
    parser.add_argument('entry')
    parser.add_argument('--revision', default='HEAD')
    parser.add_argument('--language', choices=['go', 'javascript', 'typescript', 'rust'], required=True)
    parser.add_argument('--trusted-local', action='store_true')
    parser.add_argument('--stats', type=Path)
    args = parser.parse_args()
    try:
        if not args.trusted_local:
            raise Unsupported('native compiler use requires explicit --trusted-local')
        declaration, stats = discover(args.repo, args.revision, args.entry, args.language)
        if args.stats:
            args.stats.write_text(json.dumps(stats, sort_keys=True) + '\n')
        print(json.dumps(declaration, sort_keys=True, separators=(',', ':')))
    except Unsupported as exc:
        print('UNSUPPORTED: ' + str(exc), file=sys.stderr)
        sys.exit(2)


if __name__ == '__main__':
    main()
