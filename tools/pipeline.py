#!/usr/bin/env python3
"""Trusted-local scanner adapter for Excession. Does not execute arbitrary repository code.

It runs an explicitly installed scanner and calls the Rust authority for Git/SQL
provenance. Any tool output is a static *candidate*, not a confirmed vulnerability.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Sequence

MAX_SECONDS = 300


def run(argv: Sequence[str], *, cwd: Path | None = None, timeout: int = MAX_SECONDS) -> subprocess.CompletedProcess[str]:
    return subprocess.run(list(argv), cwd=cwd, text=True, capture_output=True, timeout=timeout, check=False)


def clean_commit(repo: Path) -> str:
    if not (repo / '.git').exists():
        raise ValueError('expected a Git checkout (.git directory)')
    status = run(['git', '-C', str(repo), 'status', '--porcelain', '--untracked-files=all'])
    if status.returncode != 0 or status.stdout:
        raise ValueError('target checkout must be clean')
    obj = run(['git', '-C', str(repo), 'rev-parse', '--show-object-format'])
    if obj.returncode != 0 or obj.stdout.strip() != 'sha1':
        raise ValueError('only Git SHA-1 repositories supported by this core')
    head = run(['git', '-C', str(repo), 'rev-parse', '--verify', 'HEAD'])
    if head.returncode != 0:
        raise ValueError('cannot pin HEAD')
    return head.stdout.strip()


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as file:
        for chunk in iter(lambda: file.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def outside_checkout(path: Path, repo: Path) -> None:
    if path.resolve().is_relative_to(repo.resolve()):
        raise ValueError('output/report/database must be outside the target checkout')


def scanner_command(args: argparse.Namespace, repo: Path, output: Path) -> list[str]:
    if args.engine == 'semgrep':
        if args.config is None or not Path(args.config).is_file():
            raise ValueError('Semgrep requires a pinned, local rule config file')
        return [args.executable or 'semgrep', 'scan', '--metrics=off', '--json',
                '--config', str(Path(args.config).resolve()), '--output', str(output), '.']
    if args.engine == 'codeql':
        if not args.database or not args.config:
            raise ValueError('CodeQL requires an existing database and a pinned query suite')
        return [args.executable or 'codeql', 'database', 'analyze', str(Path(args.database).resolve()),
                str(Path(args.config).resolve()), '--format=sarif-latest', f'--output={output}']
    raise ValueError('unsupported scanner')


def ingest(binary: str, repo: Path, report: Path, db: Path, engine: str) -> dict:
    receipts: list[dict] = []
    legacy = Path(binary).name == 'relgit'
    for command in ('ingest', 'audit'):
        argv = ([binary, 'security-' + ('ingest' if command == 'ingest' else 'ingest-audit'),
                 str(repo), str(report), str(db), engine] if legacy else
                [binary, 'security', command, '--repo', str(repo), '--report', str(report),
                 '--db', str(db), '--engine', engine])
        process = run(argv, timeout=60)
        if process.returncode:
            raise RuntimeError(f'{command} refused: {process.stderr.strip()}')
        receipt = json.loads(process.stdout)
        receipts.append(receipt)
    if not receipts[1].get('independently_replayed') or receipts[0]['run_id'] != receipts[1]['run_id']:
        raise RuntimeError('independent evidence replay does not match')
    return receipts[1]


def main(argv: Sequence[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['scan', 'import'])
    p.add_argument('--engine', choices=['semgrep', 'codeql'], required=True)
    p.add_argument('--repo', type=Path, required=True)
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--rust', default='core/target/debug/excession')
    p.add_argument('--config')
    p.add_argument('--database', help='prebuilt CodeQL database (not the evidence DB)')
    p.add_argument('--executable')
    a = p.parse_args(argv)
    repo = a.repo.resolve(strict=True)
    report = a.report.resolve()
    evidence = a.evidence.resolve()
    outside_checkout(report, repo)
    outside_checkout(evidence, repo)
    old = clean_commit(repo)
    if a.action == 'scan':
        report.parent.mkdir(parents=True, exist_ok=True)
        # Refuse overwrites: a previous report is evidence, not a disposable cache.
        if report.exists():
            raise ValueError('report already exists; choose a fresh output path')
        cmd = scanner_command(a, repo, report)
        if not shutil.which(cmd[0]):
            raise ValueError(f'scanner executable unavailable: {cmd[0]}')
        process = run(cmd, cwd=repo if a.engine == 'semgrep' else None, timeout=MAX_SECONDS)
        if process.returncode != 0:
            raise RuntimeError(f'{a.engine} exited {process.returncode}: {process.stderr[-2000:]}')
        if not report.is_file():
            raise RuntimeError('scanner produced no report')
    if not report.is_file() or report.stat().st_size > 16 * 1024 * 1024:
        raise ValueError('missing or oversized scanner report')
    if clean_commit(repo) != old:
        raise ValueError('target checkout changed during scan')
    result = ingest(a.rust, repo, report, evidence, a.engine)
    print(json.dumps({'source_commit': old, 'report_sha256': sha256(report),
                      'evidence': str(evidence), 'receipt': result}, indent=2, sort_keys=True))
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError) as e:
        print(f'REFUSED: {e}', file=sys.stderr)
        sys.exit(2)
