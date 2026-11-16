import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { configuration, createServer, route } from './server.mjs';

function fixture() {
  const directory = mkdtempSync(join(tmpdir(),'excession-remote-'));
  execFileSync('git',['init','-q',directory]);
  mkdirSync(join(directory,'src'));
  writeFileSync(join(directory,'src/example.py'),'def renew_lease(): return True\n');
  execFileSync('git',['-C',directory,'add','.']);
  execFileSync('git',['-C',directory,'-c','user.name=Test',
    '-c','user.email=test@example.invalid','commit','-qm','sample']);
  const head = execFileSync('git',['-C',directory,'rev-parse','HEAD'],{encoding:'utf8'}).trim();
  return { directory, head, config:{
    token:'a'.repeat(64),repositories:{sample:directory},
    binary:'/not-present/excession',git:'git',port:0
  }};
}

test('startup requires a secret and an explicit local checkout allowlist', () => {
  assert.throws(() => configuration({}),/TOKEN/);
  assert.throws(() => configuration({EXCESSION_SERVICE_TOKEN:'a'.repeat(64),
    EXCESSION_REPOSITORIES:'{}'}),/1..32/);
  assert.throws(() => configuration({EXCESSION_SERVICE_TOKEN:'a'.repeat(64),
    EXCESSION_REPOSITORIES:'{"sample":"https://example.com/repo"}'}),/path/);
});

test('only the four fixed read-only endpoint shapes reach Excession', async () => {
  const f = fixture();
  try {
    const seen = [];
    const execute = async (bin,args) => {
      seen.push({bin,args});
      return {ok:true,output:JSON.stringify({schema:'excession.agent-locate.v1',
        source_commit:f.head,coverage:{status:'PARTIAL'}})};
    };
    const found = await route('POST','/v1/agent/locate',
      {repository:'sample',task:'renew lease',revision:f.head}, f.config, execute);
    assert.equal(found.status,200);
    assert.equal(found.body.report.source_commit,f.head);
    assert.deepEqual(seen[0].args.slice(0,4),
      ['agent','locate','--repo',f.directory]);
    const forbidden = [
      {repository:'sample',task:'renew lease',command:'rm -rf /'},
      {repository:'../outside',task:'renew lease'},
      {repository:'sample',task:'x',revision:'HEAD~2'},
      {repository:'sample',task:'renew lease',limit:1000},
    ];
    for (const body of forbidden) {
      assert.equal((await route('POST','/v1/agent/locate',
        body,f.config,execute)).status,400);
    }
    assert.equal(seen.length,1);
    assert.equal((await route('POST','/v1/agent/verify',
      {repository:'sample',base:f.head,candidate:f.head},f.config,execute)).status,400);
    assert.equal((await route('POST','/v1/agent/context',
      {repository:'sample',focus:['../secrets']},f.config,execute)).status,400);
    assert.equal((await route('POST','/v1/agent/other',{},f.config,execute)).status,404);
  } finally { rmSync(f.directory,{recursive:true,force:true}); }
});

test('HTTP gateway refuses anonymous callers and returns real Git identities', async () => {
  const f = fixture();
  const server = createServer(f.config);
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
  const base = 'http://127.0.0.1:' + server.address().port;
  try {
    const anonymous = await fetch(base+'/v1/repositories');
    assert.equal(anonymous.status,401);
    const headers = {Authorization:'Bearer '+f.config.token};
    const repo = await fetch(base+'/v1/repositories',{headers});
    assert.equal(repo.status,200);
    const payload = await repo.json();
    assert.equal(payload.schema,'excession.repositories.v1');
    assert.equal(payload.repositories[0].head,f.head);
    assert.match(payload.repositories[0].tree,/^[a-f0-9]{40}$/);
    const dangerous = await fetch(base+'/v1/agent/locate',{
      method:'POST',headers:{...headers,'Content-Type':'application/json'},
      body:JSON.stringify({repository:'sample',task:'renew lease',shell:'bash'})});
    assert.equal(dangerous.status,400);
    const badEndpoint = await fetch(base+'/v1/agent/execute',{headers});
    assert.equal(badEndpoint.status,404);
  } finally {
    await new Promise(resolve=>server.close(resolve));
    rmSync(f.directory,{recursive:true,force:true});
  }
});
