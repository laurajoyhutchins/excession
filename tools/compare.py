#!/usr/bin/env python3
"""Compare audited static candidate runs; missing/changed alerts are not fixes.

For each run, first run the Rust `security-ingest-audit` command with its exact
source revision and original report. SQL comparison alone is not an audit.
"""
import argparse
import json
from pathlib import Path
import sqlite3


def get(db: Path, run: str) -> dict[str, list[dict]]:
    with sqlite3.connect(f'file:{db.resolve()}?mode=ro', uri=True) as conn:
        info = conn.execute('SELECT status FROM security_ingest_run WHERE run_id=?', (run,)).fetchone()
        if info != ('UNVERIFIED_STATIC_CANDIDATES',):
            raise ValueError('run not found or unsupported state')
        grouped: dict[str, list[dict]] = {}
        for key, candidate_id, path, line, rule, blob in conn.execute(
            'SELECT logical_key,candidate_id,source_path,line,rule_id,source_blob '
            'FROM security_ingest_candidate WHERE run_id=? ORDER BY logical_key,candidate_id', (run,)
        ):
            grouped.setdefault(key, []).append({'candidate_id': candidate_id,
                                                   'path': path, 'line': line,
                                                   'rule': rule, 'blob': blob})
        return grouped


def compare(a: dict, b: dict) -> dict:
    common = a.keys() & b.keys()
    return {'appeared': {k: b[k] for k in sorted(b.keys() - a.keys())},
            'disappeared': {k: a[k] for k in sorted(a.keys() - b.keys())},
            'changed': {k: {'before': a[k], 'after': b[k]} for k in sorted(common) if a[k] != b[k]},
            'persisting_unchanged': sorted(k for k in common if a[k] == b[k]),
            'status': 'DIFF_ONLY_NOT_SECURITY_CONFIRMATION'}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('database', type=Path)
    p.add_argument('left_run_id')
    p.add_argument('right_run_id')
    args = p.parse_args()
    print(json.dumps(compare(get(args.database, args.left_run_id),
                             get(args.database, args.right_run_id)), indent=2))
