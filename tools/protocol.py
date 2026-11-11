#!/usr/bin/env python3
"""Pre-register an advisory-blind benchmark before running scanners.

The frozen protocol is deliberately label-free: it records selected source
commits and test families, not CVEs, patches, or known vulnerable functions.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def git_ref(repo: Path) -> str:
    result = subprocess.run(['git', '-C', str(repo), 'rev-parse', '--verify', 'HEAD'],
                            text=True, capture_output=True, check=True)
    status = subprocess.run(['git', '-C', str(repo), 'status', '--porcelain'],
                            text=True, capture_output=True, check=True)
    if status.stdout: raise ValueError('dirty target repo')
    return result.stdout.strip()


def freeze(inputs: list[Path], families: list[str], output: Path, rules: list[Path] | None = None) -> str:
    if not inputs or not families: raise ValueError('select at least one repo and one invariant family')
    if output.exists(): raise FileExistsError('protocol output exists')
    targets = [{'path': str(repo.resolve()), 'revision': git_ref(repo)} for repo in inputs]
    if len(set(x['path'] for x in targets)) != len(targets): raise ValueError('duplicate target')
    payload = {'schema': 'relgit.security.blind-protocol.v1', 'target_selection': 'user-defined-pre-advisory',
               'targets': targets, 'invariant_families': sorted(set(families)),
               'advisory_matching_allowed': False,
               'pinned_rules': [{'name': p.name, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
                                for p in (rules or [])]}
    raw = (json.dumps(payload, sort_keys=True, indent=2) + '\n').encode()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(raw)
    return hashlib.sha256(raw).hexdigest()


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--repo', type=Path, action='append', required=True)
    p.add_argument('--family', action='append', required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--rules', type=Path, action='append')
    a = p.parse_args()
    print(freeze(a.repo, a.family, a.out, a.rules))
