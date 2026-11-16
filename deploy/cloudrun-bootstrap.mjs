import {mkdtempSync,writeFileSync,chmodSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash,randomBytes} from 'node:crypto';
import {spawn,spawnSync} from 'node:child_process';
const commit='c99a0e1f673ab1d75a87319c1871bcef20700620';
const root=mkdtempSync(join(tmpdir(),'excession-'));
const archive='excession-v0.2.0-x86_64-unknown-linux-musl.tar.gz';
const checksum='df443aa3337a8f0c1945e9fd3bf27b9177154f525ae91106a6ff6fcfa7f5fa7f';
async function get(url) {
  const response=await fetch(url,{signal:AbortSignal.timeout(45000)});
  if(!response.ok) throw Error('download failed: '+response.status);
  return Buffer.from(await response.arrayBuffer());
}
const body=await get('https://github.com/laurajoyhutchins/excession/releases/download/v0.2.0/'+archive);
if(createHash('sha256').update(body).digest('hex')!==checksum)throw Error('artifact digest mismatch');
writeFileSync(join(root,archive),body);
if(spawnSync('tar',['-xzf',join(root,archive),'-C',root],{timeout:30000}).status!==0)
  throw Error('archive extraction failed');
for(const name of ['start.mjs','server.mjs']){
  const url='https://raw.githubusercontent.com/laurajoyhutchins/excession/'+commit+'/deploy/service/'+name;
  writeFileSync(join(root,name),await get(url));
}
if(spawnSync('git',['--version'],{timeout:5000}).status!==0)
  throw Error('git is unavailable in runtime image');
const binary=join(root,'excession','excession');chmodSync(binary,0o755);
const env={...process.env,
  EXCESSION_BINARY:binary,
  EXCESSION_SOURCES:'{"excession":"https://github.com/laurajoyhutchins/excession.git"}',
  EXCESSION_SERVICE_TOKEN:process.env.EXCESSION_SERVICE_TOKEN||randomBytes(32).toString('hex')};
const child=spawn(process.execPath,[join(root,'start.mjs')],{stdio:'inherit',env});
child.on('exit',code=>{process.exitCode=code||1;});