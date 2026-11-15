import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { serve } from '../server.mjs';

const root=mkdtempSync(join(tmpdir(),'excession-http-test-'));
const repo=join(root,'repo'); mkdirSync(repo);
const git=(...args)=>{
  const p=spawnSync('git',['-C',repo,...args],{encoding:'utf8'});
  assert.equal(p.status,0,p.stderr); return p.stdout.trim();
};
git('init','-q'); git('config','user.email','test@example.org');
git('config','user.name','Test');
writeFileSync(join(repo,'tracked.rs'),'fn main() {}\n');
git('add','tracked.rs'); git('commit','-qm','test');
const head=git('rev-parse','HEAD');
const stub=join(root,'excession');
writeFileSync(stub,'#!/usr/bin/env node\nconsole.log(JSON.stringify({schema:"stub",argv:process.argv.slice(2)}));\n',{mode:0o755});
process.env.EXCESSION_SERVICE_TOKEN='x'.repeat(48);
process.env.EXCESSION_REPOSITORIES=JSON.stringify({demo:repo});
process.env.EXCESSION_BINARY=stub;
const server=serve({port:0,host:'127.0.0.1'});
await new Promise(ok=>server.once('listening',ok));
const url='http://127.0.0.1:'+server.address().port;
const request=(path,method='POST',input,token=process.env.EXCESSION_SERVICE_TOKEN)=>fetch(url+path,{
  method,headers:{authorization:'Bearer '+token,'content-type':'application/json'},
  ...(input===undefined?{}:{body:JSON.stringify(input)})});

test('anonymous callers cannot invoke analyses or enumerate repositories',async()=>{
  assert.equal((await fetch(url+'/v1/repositories')).status,401);
  assert.equal((await request('/v1/agent/locate','POST',{repository:'demo',task:'task'},'bad')).status,401);
});
test('enumeration exposes only configured alias with pinned commit',async()=>{
  const r=await request('/v1/repositories','GET');
  assert.equal(r.status,200); assert.deepEqual((await r.json()).repositories,[{name:'demo',revision:head}]);
});
test('locate supplies exact git revision and argv, not a shell string',async()=>{
  const r=await request('/v1/agent/locate','POST',{repository:'demo',task:'find parsing bugs',limit:4});
  assert.equal(r.status,200); const b=await r.json();
  assert.equal(b.revision,head);
  assert.deepEqual(b.result.argv,['agent','locate','--repo',repo,'--rev',head,'--task','find parsing bugs','--limit','4','--json']);
});
test('context forwards bounded tracked relative files',async()=>{
  const r=await request('/v1/agent/context','POST',{repository:'demo',focus:['tracked.rs'],limit:2});
  assert.equal(r.status,200); assert.ok((await r.json()).result.argv.includes('tracked.rs'));
});
test('verify uses only full commit SHAs and is read-only',async()=>{
  const r=await request('/v1/agent/verify','POST',{repository:'demo',base:head,candidate:head});
  assert.equal(r.status,200); assert.equal((await r.json()).revision,null);
});
test('input refusal and no arbitrary paths/remotes/commands',async()=>{
  for(const input of [
    {repository:'other',task:'test'},
    {repository:'demo',task:'abc',command:'sh'},
    {repository:'demo',task:'abc',revision:'HEAD'},
    {repository:'demo',task:'abc',limit:10000}
  ]) assert.notEqual((await request('/v1/agent/locate','POST',input)).status,200);
  const p=await request('/v1/agent/context','POST',{repository:'demo',focus:['../etc/passwd']});
  assert.equal(p.status,400);
});
test('service health is basic and does not expose repo metadata',async()=>{
  const r=await fetch(url+'/healthz');assert.equal(r.status,200);
  assert.deepEqual(await r.json(),{ok:true,service:'excession'});
});
test.after(()=>{server.close();rmSync(root,{recursive:true,force:true});});
