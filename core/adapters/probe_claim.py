"""Run a branch's independently authored regression tests and capture cross-boundary calls."""
import argparse, json, os, sys, time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('claim',choices=['headers','reader']);p.add_argument('--output',required=True);args=p.parse_args()
import pytest
root=Path.cwd().resolve()
sys.path.insert(0,str(root))
import h11
if Path(h11.__file__).resolve().parents[1] != root: raise RuntimeError("wrong package origin")
assert (root/'h11/_readers.py').exists()
class Count:
    passed=0;failed=0;skipped=0
    def pytest_runtest_logreport(self,report):
        if report.when=='call':
            if report.passed:self.passed+=1
            if report.failed:self.failed+=1
            if report.skipped:self.skipped+=1
count=Count();calls=[]
def profile(frame,event,retval):
    if not str(frame.f_code.co_filename).startswith(str(root/'h11')+'/'):return
    if event=='call' and frame.f_code.co_qualname.startswith('ChunkedReader.') and frame.f_code.co_filename.endswith('/_readers.py'):
        calls.append({'kind':'reader-method','name':frame.f_code.co_qualname})
    if event=='call' and frame.f_code.co_name=='normalize_and_validate' and frame.f_code.co_filename.endswith('/_headers.py'):
        headers=frame.f_locals.get('headers');parsed=frame.f_locals.get('_parsed')
        calls.append({'kind':'header-interface','headers':'empty' if headers==[] else 'other','_parsed':parsed})
        if args.claim=='reader' and (headers!=[] or parsed is not False):
            raise AssertionError('reader contract crossed header interface with unexpected input')
start=time.perf_counter()
sys.setprofile(profile)
try:
    status=pytest.main(['-q','-p','no:cacheprovider','h11/tests/test_headers.py' if args.claim=='headers' else 'tests/test_reader_contract.py'],plugins=[count])
finally:
    sys.setprofile(None)
r={'claim':args.claim,'passed':count.passed,'failed':count.failed,'skipped':count.skipped,'exit_code':int(status),'cross_boundary':calls,'duration_ms':round((time.perf_counter()-start)*1000,3),'python':sys.version.split()[0],'pytest':pytest.__version__,'module_origin':str(Path(h11.__file__).resolve())}
Path(args.output).write_text(json.dumps(r,sort_keys=True,indent=2)+'\n')
print(json.dumps({k:v for k,v in r.items() if k!='cross_boundary'},sort_keys=True))
if status!=0 or count.failed or count.skipped:raise SystemExit(1)
