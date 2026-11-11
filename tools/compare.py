#!/usr/bin/env python3
"""Compare two *audited* static candidate runs, without claiming a security fix.

Run IDs and audit should be verified by the Rust binary before this report is
used. A missing candidate can indicate a code move, rule change, or an actual
fix; this is only a source-diff triage aid.
"""
import argparse
import json
from pathlib import Path
import sqlite3


def get(db: Path, run: str) -> dict[str, dict]:
    with sqlite3.connect(f'file:{db.resolve()}?mode=ro', uri=True) as conn:
        info = conn.execute('SELECT status FROM security_ingest_run WHERE run_id=?', (run,)).fetchone()
        if info != ('UNVERIFIED_STATIC_CANDIDATES',):
            raise ValueError('run not found or unsupported state')
        return {row[0]: {'path': row[1], 'line': row[2], 'rule': row[3], 'blob': row[4]}
                for row in conn.execute('SELECT logical_key,source_path,line,rule_id,source_blob FROM security_ingest_candidate WHERE run_id=?', (run,))}


def compare(a: dict, b: dict) -> dict:
    return {'appeared': {k: b[k] for k in sorted(b.keys() - a.keys())},
            'disappeared': {k: a[k] for k in sorted(a.keys() - b.keys())},
            'persisting': sorted(a.keys() & b.keys()),
            'status': 'DIFF_ONLY_NOT_SECURITY_CONFIRMATION'}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('database', type=Path)
    p.add_argument('left_run_id')
    p.add_argument('right_run_id')
    a = p.parse_args()
    print(json.dumps(compare(get(a.database, a.left_run_id), get(a.database, a.right_run_id)), indent=2))
