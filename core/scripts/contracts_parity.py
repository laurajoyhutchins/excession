"""Re-run original Git-pinned h11 preservation verifier under Rust catalog import."""
import json,sys,subprocess,os
from pathlib import Path
root=Path(__file__).resolve().parents[1]
ref=root/'fixtures/reference/v5'
work=Path(sys.argv[1]).resolve();work.mkdir(parents=True,exist_ok=True)
base=work/'h11-baseline';cand=work/'h11-candidate'
for repo in [base,cand]:
    subprocess.run(['git','clone','-q','--no-hardlinks',str(ref/'h11_seed.bundle'),str(repo)],check=True)
    subprocess.run(['git','-C',str(repo),'remote','remove','origin'],check=True)
    subprocess.run(['git','-C',str(repo),'config','user.name','RelGit Reproducibility Fixture'],check=True)
    subprocess.run(['git','-C',str(repo),'config','user.email','relgit@example.invalid'],check=True)
old='        write(b"%x\\r\\n" % len(data))\n'
new=('        digits = b"0123456789abcdef"\n'
    '        number = len(data)\n'
    '        encoded = bytearray()\n'
    '        while number:\n'
    '            encoded.append(digits[number & 15])\n'
    '            number >>= 4\n'
    '        encoded.reverse()\n'
    '        write(bytes(encoded) + b"\\r\\n")\n')
p=cand/'h11/_writers.py';s=p.read_text();assert s.count(old)==1;p.write_text(s.replace(old,new))
subprocess.run(['git','-C',str(cand),'add','h11/_writers.py'],check=True)
env=dict(os.environ,GIT_AUTHOR_DATE='2026-11-09T12:00:00+00:00',GIT_COMMITTER_DATE='2026-11-09T12:00:00+00:00')
subprocess.run(['git','-C',str(cand),'commit','-qm','fixture: isolated ChunkedWriter integer encoder'],env=env,check=True)
sha=lambda p:subprocess.check_output(['git','-C',str(p),'rev-parse','HEAD'],text=True).strip()
assert sha(base)=='c9288d5252e280e92220386fc7f22a595e522109',sha(base)
assert sha(cand)=='b0c71a158989ea8326b5015739ebd656b1e01db9',sha(cand)
spec=dict(baseline=str(base),candidate=str(cand),vendor=str(ref/'vendor/h11_preservation'),
 contracts=str(ref/'contracts.json'),links=str(ref/'h11_claim_links.json'),
 baseline_observations=str(ref/'baseline-observations.json'),
 candidate_observations=str(ref/'candidate-observations.json'),
 certificate=str(ref/'preservation.json'))
sp=work/'contracts.json';sp.write_text(json.dumps(spec))
db=work/'contracts.sqlite'
subprocess.run(['git','-C',str(cand),'status','--porcelain'],check=True)
r=subprocess.run([str(root/'target/debug/excession'),'research','contracts-import',str(sp),str(db)],capture_output=True,text=True)
print('contract import',r.returncode,r.stdout[:1500],r.stderr[-1600:]);assert r.returncode==0
result=json.loads(r.stdout);assert result['trace_count']==112
assert result['contract_ideals']==5
assert len(result['claims'])==3
print('EXACT PINNED NATIVE CERTIFICATE + RELATIONAL CONTRACT PARITY PASS')
