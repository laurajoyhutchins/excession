import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { createHash, timingSafeEqual } from 'node:crypto';
import { isAbsolute, resolve } from 'node:path';

const SHA = /^[a-f0-9]{40}$/i;
const NAME = /^[a-z][a-z0-9_-]{0,39}$/;
const OUTPUT_LIMIT = 2 * 1024 * 1024;
let active = 0;

function settings() {
  const token = process.env.EXCESSION_SERVICE_TOKEN || '';
  if (token.length < 32) throw Error('EXCESSION_SERVICE_TOKEN must have at least 32 characters');
  let repositories;
  try { repositories = JSON.parse(process.env.EXCESSION_REPOSITORIES || '{}'); }
  catch { throw Error('EXCESSION_REPOSITORIES is not JSON'); }
  if (!repositories || Array.isArray(repositories) || typeof repositories !== 'object')
    throw Error('EXCESSION_REPOSITORIES must be an object');
  for (const [name, directory] of Object.entries(repositories)) {
    if (!NAME.test(name) || typeof directory !== 'string' || !isAbsolute(directory))
      throw Error('Invalid configured repository');
    repositories[name] = resolve(directory);
  }
  return { token, repositories, binary: process.env.EXCESSION_BINARY || 'excession' };
}

function authorized(req, token) {
  if (typeof req.headers.authorization !== 'string' ||
      !req.headers.authorization.startsWith('Bearer ')) return false;
  const actual = createHash('sha256').update(req.headers.authorization.slice(7)).digest();
  const expected = createHash('sha256').update(token).digest();
  return timingSafeEqual(actual, expected);
}
function send(res, status, result) {
  res.writeHead(status, { 'content-type':'application/json; charset=utf-8',
    'cache-control':'no-store', 'x-content-type-options':'nosniff' });
  res.end(JSON.stringify(result));
}
function reject(res, status, code) { send(res,status,{error:code}); }
function keysMatch(value, allowed) {
  return value && typeof value === 'object' && !Array.isArray(value) &&
    Object.keys(value).every(k => allowed.includes(k));
}
function validLimit(v) { return v === undefined || Number.isInteger(v) && v >= 1 && v <= 64; }
function validTask(v) { return typeof v === 'string' && v.length >= 3 && v.length <= 4000 && !v.includes('\0'); }
function validFocus(paths) {
  return Array.isArray(paths) && paths.length >= 1 && paths.length <= 16 &&
    paths.every(path => typeof path === 'string' && path.length > 0 && path.length <= 512 &&
      !path.startsWith('/') && !path.includes(',') && !path.includes('\\') &&
      !path.includes('\0') && !path.split('/').some(x => x === '.' || x === '..'));
}

function run(cmd, args, parseJSON=true, timeout=40000) {
  return new Promise(done => {
    let output = '', length = 0, finished = false;
    const child = spawn(cmd,args,{stdio:['ignore','pipe','ignore'],
      env:{...process.env, GIT_TERMINAL_PROMPT:'0'}});
    const timer = setTimeout(() => child.kill('SIGKILL'),timeout);
    const finish = result => { if (finished) return; finished=true; clearTimeout(timer); done(result); };
    child.stdout.on('data', chunk => {
      length += chunk.length;
      if (length > OUTPUT_LIMIT) child.kill('SIGKILL');
      else output += chunk.toString('utf8');
    });
    child.on('error', () => finish({ok:false,code:'PROCESS_UNAVAILABLE'}));
    child.on('close', code => {
      if (length > OUTPUT_LIMIT) return finish({ok:false,code:'OUTPUT_LIMIT'});
      if (code !== 0) return finish({ok:false,code:'EXECUTION_REFUSED'});
      try { finish({ok:true,value:parseJSON ? JSON.parse(output) : output.trim()}); }
      catch { finish({ok:false,code:'INVALID_OUTPUT'}); }
    });
  });
}
async function revision(repo, wanted) {
  if (wanted !== undefined && (typeof wanted !== 'string' || !SHA.test(wanted))) return null;
  const result = await run('git',['-C',repo,'rev-parse','--verify',(wanted || 'HEAD')+'^{commit}'],false,8000);
  return result.ok && SHA.test(result.value) ? result.value.toLowerCase() : null;
}

async function agent(route, input, config) {
  const {repositories,binary} = config;
  if (!input || typeof input !== 'object' || !Object.hasOwn(repositories,input.repository))
    return {status:400,error:'UNKNOWN_REPOSITORY'};
  const repo = repositories[input.repository];
  let argv = ['agent'];
  let pinned = null;
  if (route === '/v1/agent/locate') {
    if (!keysMatch(input,['repository','task','revision','limit']) ||
        !validTask(input.task) || !validLimit(input.limit))
      return {status:400,error:'INVALID_ARGUMENTS'};
    pinned = await revision(repo,input.revision);
    if (!pinned) return {status:422,error:'REVISION_NOT_AVAILABLE'};
    argv.push('locate','--repo',repo,'--rev',pinned,'--task',input.task,
      '--limit',String(input.limit ?? 20),'--json');
  } else if (route === '/v1/agent/context') {
    if (!keysMatch(input,['repository','revision','focus','task','limit']) ||
        !validFocus(input.focus) || !validLimit(input.limit) ||
        input.task !== undefined && !validTask(input.task))
      return {status:400,error:'INVALID_ARGUMENTS'};
    pinned = await revision(repo,input.revision);
    if (!pinned) return {status:422,error:'REVISION_NOT_AVAILABLE'};
    argv.push('context','--repo',repo,'--rev',pinned,
      '--focus',input.focus.join(','),'--limit',String(input.limit ?? 32));
    if (input.task) argv.push('--task',input.task);
  } else if (route === '/v1/agent/verify') {
    if (!keysMatch(input,['repository','base','candidate','limit']) ||
        typeof input.base !== 'string' || !SHA.test(input.base) ||
        typeof input.candidate !== 'string' || !SHA.test(input.candidate) ||
        !validLimit(input.limit)) return {status:400,error:'INVALID_ARGUMENTS'};
    const base = await revision(repo,input.base);
    const candidate = await revision(repo,input.candidate);
    if (!base || !candidate) return {status:422,error:'REVISION_NOT_AVAILABLE'};
    argv.push('verify','--repo',repo,'--base',base,'--candidate',candidate,
      '--limit',String(input.limit ?? 32));
  } else return {status:404,error:'NOT_FOUND'};
  const output = await run(binary,argv);
  if (!output.ok) return {status:502,error:output.code};
  if (!output.value || typeof output.value !== 'object')
    return {status:502,error:'INVALID_OUTPUT'};
  return {status:200,data:{schema:'excession.gateway.v1',repository:input.repository,
    revision:pinned, operation:route.split('/').at(-1), result:output.value}};
}

async function body(req) {
  let size = 0; const chunks = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > 16384) throw Error('TOO_LARGE');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

export function serve({port=Number(process.env.PORT || 8080),host='0.0.0.0'}={}) {
  const config = settings();
  const server = createServer(async (req,res) => {
    if (req.method === 'GET' && req.url === '/healthz')
      return send(res,200,{ok:true,service:'excession'});
    if (!authorized(req,config.token)) return reject(res,401,'UNAUTHORIZED');
    if (req.method === 'GET' && req.url === '/v1/repositories') {
      const entries=[];
      for (const name of Object.keys(config.repositories).sort())
        entries.push({name,revision:await revision(config.repositories[name])});
      return send(res,200,{schema:'excession.repositories.v1',repositories:entries});
    }
    if (req.method !== 'POST' || !['/v1/agent/locate','/v1/agent/context','/v1/agent/verify'].includes(req.url))
      return reject(res,404,'NOT_FOUND');
    if (active >= 2) return reject(res,429,'BUSY');
    if (req.headers['content-type']?.split(';')[0] !== 'application/json')
      return reject(res,415,'JSON_REQUIRED');
    let input;
    try { input = await body(req); }
    catch { return reject(res,400,'INVALID_JSON_OR_SIZE'); }
    active += 1;
    try {
      const result=await agent(req.url,input,config);
      if (result.data) send(res,result.status,result.data);
      else reject(res,result.status,result.error);
    } catch { reject(res,500,'INTERNAL_ERROR'); }
    finally { active -= 1; }
  });
  server.listen(port,host);
  return server;
}
if (process.argv[1] && import.meta.url.endsWith(process.argv[1])) serve();
