"""Cross-implementation historical h11 and PostCSS replay, Git object parity."""
from pathlib import Path
import subprocess,json,sys,os
root=Path(__file__).resolve().parents[1]
src=root/'fixtures/reference/v4'
work=Path(sys.argv[1]).resolve();work.mkdir(parents=True,exist_ok=True)
configs=json.loads((src/'projections.json').read_text())
manifests=json.loads((src/'upstream.json').read_text())
for name,language,module,old,new,probe,compat in [
 ('h11','python','h11._headers','h11/_headers.py','h11/wire/_headers.py',
  ['python','-c',(src/'probes/h11_behavior.py').read_text()],
  ['python','-c',"import h11,h11._headers,h11.wire._headers; assert h11._headers is h11.wire._headers; assert h11._headers.Headers is h11.wire._headers.Headers;print('h11 alias OK')"]),
 ('postcss','javascript','postcss.lib.parser','postcss/lib/parser.js','postcss/engine/parser.js',
  ['node','-e',(src/'probes/postcss_behavior.js').read_text()],
  ['node','-e',"const assert=require('node:assert/strict'); const a=require('./postcss/lib/parser'), b=require('./postcss/engine/parser'); assert.strictEqual(a,b);console.log('postcss alias OK')"])
]:
    base=configs[name]['base'];source=work/(name+'-source');output=work/(name+'-projected')
    subprocess.run(['git','clone','-q','--local','--no-hardlinks',str(src/(name+'.bundle')),str(source)],check=True)
    subprocess.run(['git','-C',str(source),'checkout','-q','--detach',base],check=True)
    spec={'source':str(source),'name':name,'base':base,'language':language,'moves':[{'module':module,'old':old,'new':new}],
          'probe':probe,'compatibility':compat,'adapter_script':str(root/'adapters'/('python_rewrite.py' if language=='python' else 'js_transform.cjs'))}
    sp=work/(name+'-project.json');sp.write_text(json.dumps(spec))
    db=work/(name+'.sqlite');binary=root/'target/debug/excession'
    r=subprocess.run([str(binary),'research','project',str(sp),str(db),str(output)],capture_output=True,text=True)
    print(name,'project',r.returncode,r.stderr[-1000:]);assert r.returncode==0
    info=json.loads(r.stdout)
    assert info['projection_tree']==configs[name]['projection_tree'],(info['projection_tree'],configs[name]['projection_tree'])
    assert info['projection_commit']==configs[name]['projection_commit'],(info['projection_commit'],configs[name]['projection_commit'])
    man=manifests[name]
    env=dict(os.environ,GIT_AUTHOR_NAME='Historical upstream patch',GIT_AUTHOR_EMAIL='relgit@example.invalid',
        GIT_COMMITTER_NAME='RelGit Test',GIT_COMMITTER_EMAIL='relgit@example.invalid',
        GIT_AUTHOR_DATE='2026-11-09T12:00:00+0000',GIT_COMMITTER_DATE='2026-11-09T12:00:00+0000')
    subprocess.run(['git','-C',str(source),'apply',str(src/man['fixture_patch'])],check=True)
    subprocess.run(['git','-C',str(source),'add','-A'],check=True)
    subprocess.run(['git','-C',str(source),'commit','-q','-m','Replay authentic upstream historical patch on pinned local fixture'],check=True,env=env)
    manifest=dict(man,fixture_patch=str(src/man['fixture_patch']))
    mpath=work/(name+'-manifest.json');mpath.write_text(json.dumps(manifest))
    replay={'source':str(source),'projection':str(output),'projection_id':info['id'],'name':name,
        'manifest':str(mpath),'output':str(work/(name+'-replayed')),'native_tests':[
            ['python','-m','pytest','-q','-p','no:cacheprovider','h11/tests/test_headers.py'] if name=='h11' else
            ['node','-e',(src/'probes/postcss_behavior.js').read_text()]
        ]}
    rpath=work/(name+'-replay.json');rpath.write_text(json.dumps(replay))
    rr=subprocess.run([str(binary),'research','replay',str(rpath),str(db)],capture_output=True,text=True)
    print(name,'replay',rr.returncode,rr.stderr[-1800:]);print(rr.stdout[:1100]);assert rr.returncode==0
    replay_result=json.loads(rr.stdout)
    expected={'h11':'87a0a610d6078f3922013f642aa3eaaad6590b53','postcss':'dba02ee93ad6ac251f515f37d0469715912d0ecb'}[name]
    assert replay_result['projected_tree']==expected,(name,replay_result['projected_tree'],expected)
    expectedcommit={'h11':'9bc063630c5447f28d08b519fc17cd2e943f61ae','postcss':'ca5be16826310fcd7d966f5d75f3852396d52b8f'}[name]
    assert replay_result['projected_commit']==expectedcommit,(name,replay_result['projected_commit'],expectedcommit)
print('HISTORICAL REAL GIT PARITY PASS')
