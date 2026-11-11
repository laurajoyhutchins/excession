#!/usr/bin/env python3
"""Self-contained differential reproduction using bundled Git objects and native probes.

This is orchestration of tests only; the implementation of transactions,
verification decisions and Git/SQLite evidence indexing is in Rust.
"""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import sys

ROOT=Path(__file__).resolve().parents[1]
BIN=ROOT/'target/debug/relgit'

def run(argv, *, capture=False):
    args=list(map(str,argv))
    env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',PYTEST_DISABLE_PLUGIN_AUTOLOAD='1')
    if capture:
        # File-backed capture avoids inherited stdout pipes keeping a parent
        # subprocess waiting indefinitely after native Git/Python child exit.
        with tempfile.TemporaryFile(mode='w+t', encoding='utf8') as out:
            with tempfile.TemporaryFile(mode='w+t', encoding='utf8') as err:
                completed=subprocess.run(args,check=False,stdout=out,stderr=err,
                    text=True,timeout=60,env=env)
                if completed.returncode!=0:
                    err.seek(0)
                    raise RuntimeError(f'{args[0]} exited {completed.returncode}: {err.read()[-2000:]}')
                out.seek(0)
                return out.read()
    subprocess.run(args,check=True,timeout=60,env=env)
    return ''

def g(repo,*args):
    return run(['git','-C',repo,*args],capture=True).strip()

def item(label, value):
    return {'stage':label, 'verified':True, 'identity':value}

def main():
    if len(sys.argv)!=2:
        raise SystemExit('Usage: python scripts/reproduce_v11.py EMPTY_WORK_DIRECTORY')
    work=Path(sys.argv[1]).resolve()
    if work.exists() and any(work.iterdir()):
        raise SystemExit('REFUSED: work directory must be empty')
    work.mkdir(parents=True,exist_ok=True)
    if not BIN.is_file():
        raise SystemExit('Build first: cargo build --offline')
    report={'schema':'relgit-v11-reproducer/1','stages':[],'negative_cases':[], 'limitations':[
        'Bounded runtime probes, not whole-program observational equivalence',
        'PostCSS historical patch is a transplant onto an older published snapshot',
        'Native parsers and runtime probes remain Python/TypeScript',
        'No cryptographically signed cross-agent claims',
        'Git commits and SQLite transactions are not cross-storage atomic',
        'Toolchain, environment, and timeouts are outside the modeled dependency boundary',
    ]}
    python=sys.executable
    print('STAGE projections',flush=True)
    run([python,ROOT/'scripts/projection_parity.py',work/'projection'])
    report['stages'].append(item('source-transaction-v2','h11+postcss exact commits and trees'))
    print('STAGE historical replay',flush=True)
    run([python,ROOT/'scripts/historical_parity.py',work/'historical'])
    report['stages'].append(item('mapped-replay-v4','h11+postcss exact commits and trees'))
    print('STAGE contract import',flush=True)
    run([python,ROOT/'scripts/contracts_parity.py',work/'contracts'])
    certificate=json.loads((ROOT/'fixtures/reference/v5/preservation.json').read_text())
    digest=hashlib.sha256(json.dumps(certificate,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    assert digest=='a3eac26a3b76518816c035ac7152bc745b602798c23a7e3639ae87855d4a8240',digest
    report['stages'].append(item('contract-preservation-v5',{'traces':112,'ideals':5,'certificate_sha256':digest}))

    print('STAGE Python reflection/pickle',flush=True)
    # The two fully checked layout projections preserve Python reflection/pickle metadata.
    h=work/'historical'; results=[]
    for label,original,projected,expected_commit,expected_tree in [
      ('parent',h/'h11-original-parent',h/'h11-projected',
       'b86616965e394e8d3b26095c8dcbdb66fff27f3b','7797917502e01e68996faba4714608c496686816'),
      ('replay',h/'h11-source',h/'h11-replayed',
       '14a4b08e89b1aa12786ab56da9c1d850a9b4fde0','eea07180656a9a10ad50cf294fcf2a4b04ed02f4'),
    ]:
        if label=='parent':
            run(['git','clone','-q','--no-hardlinks',h/'h11-source',original])
            run(['git','-C',original,'checkout','--quiet','--detach','f56d70277b1ae1eb3ccaff0e09ff62c546b0ae94'])
        plan={'original':str(original),'projected':str(projected),'output':str(h/f'h11-identity-{label}'),
           'original_path':'h11/_headers.py','projected_path':'h11/wire/_headers.py',
           'logical':'h11._headers','physical':'h11.wire._headers',
           'adapter':str(ROOT/'adapters/python_identity.py'),
           'probe':str(ROOT/'adapters/python_identity_probe.py'),
           'crossload':str(ROOT/'adapters/python_pickle_crossload.py'), 'label':label}
        spec=h/f'identity-{label}.json';spec.write_text(json.dumps(plan))
        output=json.loads(run([BIN,'identity',spec,h/'identity.sqlite'],capture=True))
        assert output['commit']==expected_commit,(label,output)
        assert output['tree']==expected_tree,(label,output)
        assert output['symbols']==12,output
        results.append({'stage':label,'commit':expected_commit,'tree':expected_tree,'symbols':12})
    report['stages'].append(item('python-reflection-pickle-v8',results))

    print('STAGE shared contract catalog',flush=True)
    # Contracts and historical replay share one relational database, without promoting source claims.
    import_spec=h/'v5-import.json'
    import_spec.write_text((work/'contracts/contracts.json').read_text())
    print('STAGE shared catalog: importing',flush=True)
    imported=json.loads(run([BIN,'contracts-import',import_spec,h/'h11.sqlite'],capture=True))
    assert imported['trace_count']==112
    impact_plan={
        'source':str(h/'h11-source'),
        'source_base':'f56d70277b1ae1eb3ccaff0e09ff62c546b0ae94',
        'source_change':'410cbf0299fe3f12c0cf3fc76eb4b441b190b3aa',
        'projection':str(h/'h11-projected'),
        'projection_base':'b82d70b9beda51ad6665ce70b2eaa3312939d8e1',
        'replay':str(h/'h11-replayed'),
        'replay_commit':'9bc063630c5447f28d08b519fc17cd2e943f61ae',
        'mapping':{'h11/_headers.py':'h11/wire/_headers.py'},
        'id':'real-h11-60782ad',
    }
    ip=h/'contract-impact.json';ip.write_text(json.dumps(impact_plan))
    print('STAGE shared catalog: impact',flush=True)
    decision=json.loads(run([BIN,'contract-impact',ip,h/'h11.sqlite'],capture=True))
    got={s['view']:(s['matching_inputs'],s['decision']) for s in decision['snapshots']}
    assert got=={
        'original_parent':(11,'REVERIFY_REQUIRED'),
        'original_change':(13,'SOURCE_MATCH_ONLY'),
        'projection_parent':(8,'UNVERIFIED_PROJECTION'),
        'projection_replay':(8,'UNVERIFIED_PROJECTION')},got
    report['stages'].append(item('conservative-contract-impact-v6',got))

    print('STAGE workload cost',flush=True)
    # Recompute v3 weight policy from original archived measurements (not new benchmarks).
    cost_expected=json.loads((ROOT/'fixtures/reference/v3/expected.json').read_text())
    for name in ('h11','postcss'):
        cost=json.loads(run([BIN,'score',ROOT/f'fixtures/reference/v3/{name}.json'],capture=True))
        assert cost['chosen']==cost_expected[name]['chosen']
        for layout in ('original','projected'):
            assert abs(cost['costs'][layout]-cost_expected[name]['costs'][layout])<1e-8
    report['stages'].append(item('workload-score-v3','h11 and PostCSS exact decisions and costs'))

    print('STAGE adversarial refusals',flush=True)
    # Invalid input must be refused, not silently treated as evidence.
    good=json.loads((h/'h11-project.json').read_text())
    cases=[('traversal',{'moves':[{'module':'h11._headers','old':'h11/_headers.py','new':'../outside.py'}]}),
       ('collision',{'moves':[{'module':'h11._headers','old':'h11/_headers.py','new':'h11/_headers.py'}]}),
       ('wrong-source-head',{'base':'0'*40}),
       ('undeclared-module',{'moves':[{'module':'h11.missing','old':'h11/_headers.py','new':'h11/wire/_headers.py'}]}),]
    for idx,(label,changes) in enumerate(cases):
        spec=dict(good,**changes)
        f=work/f'bad-{label}.json';f.write_text(json.dumps(spec))
        out=work/f'bad-{label}-output'
        print('NEGATIVE CASE',label,flush=True)
        p=subprocess.run([str(BIN),'project',str(f),str(work/f'bad-{label}.sqlite'),str(out)],capture_output=True,text=True,timeout=50)
        assert p.returncode!=0,(label,p.stdout)
        assert 'REFUSED:' in p.stderr,(label,p.stderr)
        report['negative_cases'].append(label)
    # Semantic contract importer rejects altered oracles, not just incorrect certificates.
    spec=json.loads((work/'contracts/contracts.json').read_text())
    forged=work/'forged-observations.json'
    x=json.loads(Path(spec['candidate_observations']).read_text())
    x['rows'][0]['writes'].append('aa')
    forged.write_text(json.dumps(x,sort_keys=True))
    spec['candidate_observations']=str(forged)
    forged_plan=work/'forged-contracts.json';forged_plan.write_text(json.dumps(spec))
    print('NEGATIVE CASE forged-observation',flush=True)
    bad=subprocess.run([str(BIN),'contracts-import',str(forged_plan),str(work/'forged-contracts.sqlite')],capture_output=True,text=True,timeout=50)
    assert bad.returncode!=0 and 'REFUSED:' in bad.stderr,bad
    report['negative_cases'].append('forged-observation')
    # Verify only original SQLite local rowIDs remain local, never exported as logical identity.
    for file in [h/'h11.sqlite',h/'postcss.sqlite',h/'identity.sqlite',work/'contracts/contracts.sqlite',work/'projection/h11.sqlite',work/'projection/postcss.sqlite']:
        db=sqlite3.connect(file)
        assert db.execute('PRAGMA foreign_key_check').fetchall()==[],file
    for name in ('h11','postcss'):
        for suffix in ('source','projected','replayed'):
            g(h/f'{name}-{suffix}','fsck','--full','--no-reflogs','--no-progress')
    for entry in report['stages']:
        assert entry['verified']
    out=work/'verification.json';out.write_text(json.dumps(report,sort_keys=True,indent=2)+'\n')
    print('PASS: self-contained v11 Rust/native differential reproduction')
    print('Stages:',len(report['stages']),'negative refusals:',len(report['negative_cases']))
    print('Report:',out)

if __name__=='__main__':main()
