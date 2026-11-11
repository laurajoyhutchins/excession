"""Bounded h11 test-call evidence using CPython 3.13 local code monitoring.

Only the three precisely declared code objects are observed. This avoids an
interpreter-wide tracing callback and preserves the original function objects.
The independent tests are unmodified. Fails closed when code objects differ.
"""
import argparse
import json
import sys
import time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('claim', choices=['headers', 'reader'])
p.add_argument('--output')
p.add_argument('--stdout-only', action='store_true')
a = p.parse_args()
if a.stdout_only == bool(a.output):
    raise SystemExit('provide exactly one of --output or --stdout-only')
# Use POSIX alarm to bound tests; the caller can additionally enforce a process timeout.
import signal
signal.alarm(35)
if not hasattr(sys, 'monitoring'):
    raise SystemExit('CPython monitoring unavailable; fail closed')
import pytest
root = Path.cwd().resolve()
sys.path.insert(0, str(root))
import h11
from h11 import _readers, _headers
if Path(h11.__file__).resolve().parents[1] != root:
    raise RuntimeError('wrong package origin')

class Count:
    passed = failed = skipped = 0
    def pytest_runtest_logreport(self, report):
        if report.when == 'call':
            if report.passed: self.passed += 1
            if report.failed: self.failed += 1
            if report.skipped: self.skipped += 1

count = Count()
calls = []
expected = {
    _headers.normalize_and_validate.__code__: 'header-interface',
    _readers.ChunkedReader.__init__.__code__: 'reader-method',
    _readers.ChunkedReader.__call__.__code__: 'reader-method',
}
assert len(expected) == 3
for code in expected:
    if Path(code.co_filename).resolve().parents[1] != root:
        raise RuntimeError('monitored code object is not from checkout')

mon = sys.monitoring
mon.use_tool_id(mon.PROFILER_ID, 'relgit-constrained-contract-probe')

def on_start(code, offset):
    if code not in expected or offset != 0:
        raise RuntimeError('unexpected monitoring code or offset')
    frame = sys._getframe(1)
    if frame.f_code is not code:
        raise RuntimeError('monitor frame mismatch')
    if expected[code] == 'reader-method':
        calls.append({'kind': 'reader-method', 'name': code.co_qualname})
    else:
        headers = frame.f_locals.get('headers')
        parsed = frame.f_locals.get('_parsed')
        calls.append({'kind': 'header-interface',
                      'headers': 'empty' if headers == [] else 'other',
                      '_parsed': parsed})
        if a.claim == 'reader' and (headers != [] or parsed is not False):
            raise AssertionError('reader contract crossed unverified header interface')

mon.register_callback(mon.PROFILER_ID, mon.events.PY_START, on_start)
for code in expected:
    mon.set_local_events(mon.PROFILER_ID, code, mon.events.PY_START)
start = time.perf_counter()
try:
    status = pytest.main(['-q','-p','no:cacheprovider',
                         'h11/tests/test_headers.py' if a.claim=='headers'
                         else 'tests/test_reader_contract.py'],plugins=[count])
finally:
    for code in expected:
        mon.set_local_events(mon.PROFILER_ID, code, 0)
    mon.register_callback(mon.PROFILER_ID, mon.events.PY_START, None)
    mon.free_tool_id(mon.PROFILER_ID)
r = {'claim':a.claim,'passed':count.passed,'failed':count.failed,
     'skipped':count.skipped,'exit_code':int(status),'cross_boundary':calls,
     'duration_ms':round((time.perf_counter()-start)*1000,3),
     'python':sys.version.split()[0],'pytest':pytest.__version__,
     'module_origin':str(Path(h11.__file__).resolve())}
if a.stdout_only:
    # A bounded interface value observed by the same native branch execution.
    # This is not an independent contract proof beyond the empty-header case.
    r['empty_header_result'] = json.dumps(
        [[value.hex() for value in triple]
         for triple in _headers.normalize_and_validate([])], sort_keys=True)
    print('RELGIT_OBSERVATION:' + json.dumps(r,sort_keys=True,separators=(',',':')))
else:
    Path(a.output).write_text(json.dumps(r,sort_keys=True,indent=2)+'\n')
    print(json.dumps({k:v for k,v in r.items() if k!='cross_boundary'},sort_keys=True))
if status!=0 or count.failed or count.skipped:
    raise SystemExit(1)
