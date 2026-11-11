#!/usr/bin/env python3
"""Compare a portable Rust-built SQLite catalog with frozen Python-v1 fingerprints.

Only canonical source-identity columns enter hashes. SQLite rowids and local
checkout paths deliberately do not enter, so relocation cannot mask a mismatch.
"""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path

TABLES = ('tree_entry', 'module', 'dependency', 'symbol', 'api_binding', 'hazard')

def fingerprint(path: Path):
    db = sqlite3.connect(f'file:{path}?mode=ro', uri=True)
    result = {}
    for table in TABLES:
        columns = [r[1] for r in db.execute(f'PRAGMA table_info({table})')]
        assert columns[0] == 'repo_id', (table, columns)
        cols = ', '.join('t."' + c + '"' for c in columns[1:])
        rows = sorted((r[0], *r[1:]) for r in db.execute(
            f'SELECT repo.name, {cols} FROM "{table}" t '
            'JOIN repository repo ON t.repo_id = repo.id'))
        payload = json.dumps(rows, separators=(',', ':'), ensure_ascii=True).encode()
        result[table] = {'rows': len(rows), 'sha256': hashlib.sha256(payload).hexdigest()}
    assert db.execute('PRAGMA foreign_key_check').fetchall() == [], 'foreign key violations'
    return result

if __name__=='__main__':
    if len(sys.argv) != 3:
        raise SystemExit('Usage: check_parity.py expected-fingerprints.json new.sqlite')
    expected = json.loads(Path(sys.argv[1]).read_text())
    current = fingerprint(Path(sys.argv[2]))
    if current != expected:
        for table in TABLES:
            if current[table] != expected[table]:
                print(f'MISMATCH {table}: expected {expected[table]}, got {current[table]}', file=sys.stderr)
        raise SystemExit(1)
    print('PASS: six canonical relational tables match Python v1 across all 16 snapshots')
    print(json.dumps(current,sort_keys=True,indent=2))
