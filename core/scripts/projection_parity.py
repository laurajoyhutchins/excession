"""Cross-implementation differential check of Rust projections against RelGit v2 fixtures."""
import os,json,sys,shutil,subprocess,pathlib
ROOT=pathlib.Path(__file__).resolve().parents[1]
SRC=ROOT/'fixtures/reference/v2'
work=pathlib.Path(sys.argv[1]).resolve()
work.mkdir(parents=True, exist_ok=True)
report=json.loads((SRC/'transaction_report.json').read_text())
for name,lang,logical,fromp,top,probe,compat in [
 ('h11','python','h11._state','h11/_state.py','h11/protocol/_state.py',
  ['python','-c',(SRC/'probes/h11_behavior.py').read_text()],
  ['python','-c', '''import h11,h11._state,h11.protocol._state,pickle
assert h11._state is h11.protocol._state
assert h11.CLIENT is h11.protocol._state.CLIENT
assert h11._state.ConnectionState is h11.protocol._state.ConnectionState
assert pickle.loads(pickle.dumps(h11._state.ConnectionState)) is h11._state.ConnectionState
print('h11 compatibility OK')''']),
 ('postcss','javascript','postcss.lib.parser','postcss/lib/parser.js','postcss/engine/parser.js',
  ['node','-e',(SRC/'probes/postcss_behavior.js').read_text()],
  ['node','-e', '''const assert=require('node:assert/strict');
const before=require('./postcss/lib/parser');const after=require('./postcss/engine/parser');
assert.strictEqual(before,after);const postcss=require('./postcss');
assert.equal(postcss.parse('p { x: y; }').first.first.prop,'x');
console.log('PostCSS compatibility OK');'''])]:
    source=work/(name+'-source')
    subprocess.run(['git','clone','--local','--no-hardlinks','-q',str(SRC/(name+'.bundle')),str(source)],check=True)
    base=report[name]['base']
    subprocess.run(['git','-C',str(source),'checkout','--detach','-q',base],check=True)
    spec={'source':str(source),'name':name,'base':base,'language':lang,
          'moves':[{'module':logical,'old':fromp,'new':top}],
          'probe':probe,'compatibility':compat,
          'adapter_script':str(ROOT/'adapters'/('python_rewrite.py' if lang=='python' else 'js_transform.cjs'))}
    path=work/(name+'.json');path.write_text(json.dumps(spec))
    db=work/(name+'.sqlite');dest=work/(name+'-projected')
    result=subprocess.run([str(ROOT/'target/debug/relgit'),'project',str(path),str(db),str(dest)],capture_output=True,text=True)
    print(name,'exit',result.returncode,'stderr',result.stderr[-1000:]);print(result.stdout[:1000]);
    if result.returncode:sys.exit(1)
    got=json.loads(result.stdout)
    exp=report[name]
    assert got['projection_tree']==exp['projection_tree'],(name,got['projection_tree'],exp['projection_tree'])
    assert got['projection_commit']==exp['projection_commit'],(name,got['projection_commit'],exp['projection_commit'])
    assert got['edit_count']==exp['edit_count'],(name,got['edit_count'],exp['edit_count'])
print('EXACT PROJECTION PARITY PASS')
