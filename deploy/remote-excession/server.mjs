// Thin HTTP bridge to the *real* Excession CLI.
// There is no source parser, arbitrary command API, repository URL, or dynamic
// checkout here. The build/deployer supplies an immutable allowlist of local
// Git repositories. This process only invokes fixed read-only CLI commands.
import http from 'node:http';
import { createHash, timingSafeEqual } from 'node:crypto';
import { spawn } from 'node:child_process';
import { realpathSync, statSync } from 'node:fs';
import { isAbsolute } from 'node:path';

const MAX_INPUT = 16 * 1024;
const MAX_OUTPUT = 2 * 1024 * 1024;
const SHA = /^[a-f0-9]{40}$/;
const ALIAS = /^[a-z][a-z0-9_-]{0,39}$/;
const ROUTES = new Set([
  '/v1/repositories', '/v1/agent/locate', '/v1/agent/context',
  '/v1/agent/verify'
]);
const deny = (status, code) => ({ status, body: { error: code } });
const json = (res, status, body) => {
  res.writeHead(status, { 'content-type': 'application/json; charset=utf-8',
    'cache-control': 'no-store', 'x-content-type-options': 'nosniff' });
  res.end(JSON.stringify(body));
};

export function configuration(env = process.env) {
  const token = env.EXCESSION_SERVICE_TOKEN;
  if (typeof token !== 'string' || token.length < 32 || token.length > 4096)
    throw new Error('EXCESSION_SERVICE_TOKEN must be 32..4096 characters');
  let defined;
  try { defined = JSON.parse(env.EXCESSION_REPOSITORIES ?? '{}'); }
  catch { throw new Error('invalid EXCESSION_REPOSITORIES JSON'); }
  if (!defined || typeof defined !== 'object' || Array.isArray(defined) ||
      !Object.keys(defined).length || Object.keys(defined).length > 32)
    throw new Error('EXCESSION_REPOSITORIES must map 1..32 aliases to checkout paths');
  const repositories = {};
  for (const [alias, location] of Object.entries(defined)) {
    if (!ALIAS.test(alias) || typeof location !== 'string' || !isAbsolute(location))
      throw new Error('invalid repository alias or path');
    const real = realpathSync(location);
    if (!statSync(real).isDirectory()) throw new Error('repository is not a directory');
    repositories[alias] = real;
  }
  return {
    token, repositories,
    binary: env.EXCESSION_BIN || '/usr/local/bin/excession',
    git: env.GIT_BIN || '/usr/bin/git',
    port: Number(env.PORT || '8080')
  };
}

function equalToken(expected, received) {
  // Fixed-length digests avoid leaking secret length to a remote caller.
  if (typeof received !== 'string' || !received.startsWith('Bearer ') ||
      received.length > 8192) return false;
  const expectedHash = createHash('sha256').update(expected).digest();
  const providedHash = createHash('sha256').update(received.slice(7)).digest();
  return timingSafeEqual(expectedHash, providedHash);
}

function validKeys(value, allowed, required) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  return Object.keys(value).every(key => allowed.includes(key)) &&
    required.every(key => Object.hasOwn(value, key));
}
function validSha(value) { return typeof value === 'string' && SHA.test(value); }
function validLimit(value) {
  return value === undefined || (Number.isSafeInteger(value) && value >= 1 && value <= 64);
}
function validAlias(value, cfg) {
  return typeof value === 'string' && Object.hasOwn(cfg.repositories, value);
}
function validTask(value) {
  return typeof value === 'string' && value.length >= 3 && value.length <= 2000 &&
    !value.includes('\0');
}
function validFocus(value) {
  return Array.isArray(value) && value.length >= 1 && value.length <= 16 &&
    value.every(p => typeof p === 'string' && p.length >= 1 && p.length <= 512 &&
      !p.startsWith('/') && !p.includes('\\') && !p.includes(',') &&
      !p.includes('\0') && !p.split('/').some(x => x === '.' || x === '..'));
}

function commandFor(path, input, cfg) {
  if (!validAlias(input.repository, cfg)) return null;
  const repo = cfg.repositories[input.repository];
  const limit = input.limit === undefined ? '32' : String(input.limit);
  if (path === '/v1/agent/locate') {
    if (!validKeys(input, ['repository','task','revision','limit'], ['repository','task']) ||
        !validTask(input.task) || !validLimit(input.limit) ||
        (input.revision !== undefined && !validSha(input.revision))) return null;
    return ['agent','locate','--repo',repo,'--task',input.task,
      '--rev',input.revision ?? 'HEAD','--limit',limit,'--json'];
  }
  if (path === '/v1/agent/context') {
    if (!validKeys(input, ['repository','focus','revision','task','limit'],
      ['repository','focus']) || !validFocus(input.focus) || !validLimit(input.limit) ||
      (input.revision !== undefined && !validSha(input.revision)) ||
      (input.task !== undefined && !validTask(input.task))) return null;
    return ['agent','context','--repo',repo,'--rev',input.revision ?? 'HEAD',
      '--focus',input.focus.join(','),'--limit',limit,
      ...(input.task ? ['--task',input.task] : [])];
  }
  if (path === '/v1/agent/verify') {
    if (!validKeys(input, ['repository','base','candidate','limit'],
      ['repository','base','candidate']) ||
      !validSha(input.base) || !validSha(input.candidate) ||
      input.base === input.candidate || !validLimit(input.limit)) return null;
    return ['agent','verify','--repo',repo,'--base',input.base,
      '--candidate',input.candidate,'--limit',limit];
  }
  return null;
}

function execute(program, args, timeoutMs = 30000) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(program, args, {
        shell: false, stdio: ['ignore','pipe','ignore'], timeout: timeoutMs,
        env: { PATH: process.env.PATH ?? '/usr/bin:/bin',
          HOME: process.env.HOME ?? '/tmp', LANG: 'C.UTF-8' }
      });
    } catch { resolve({ ok:false }); return; }
    let size = 0;
    const chunks = [];
    let oversized = false;
    child.stdout.on('data', chunk => {
      size += chunk.length;
      if (size > MAX_OUTPUT) { oversized = true; child.kill(); }
      else chunks.push(chunk);
    });
    child.on('error', () => resolve({ ok:false }));
    child.on('close', (code, signal) => {
      if (oversized || code !== 0 || signal) resolve({ ok:false });
      else resolve({ ok:true, output:Buffer.concat(chunks).toString('utf8') });
    });
  });
}
async function gitRef(cfg, repo, selector) {
  const result = await execute(cfg.git,
    ['-C',repo,'rev-parse','--verify',selector], 10000);
  if (!result.ok) return null;
  const sha = result.output.trim();
  return validSha(sha) ? sha : null;
}

export async function route(method, path, data, cfg, run = execute) {
  if (!ROUTES.has(path)) return deny(404,'NOT_FOUND');
  if (path === '/v1/repositories') {
    if (method !== 'GET') return deny(405,'METHOD_NOT_ALLOWED');
    const repositories = [];
    for (const [name, repo] of Object.entries(cfg.repositories)) {
      const head = await gitRef(cfg, repo, 'HEAD');
      const tree = await gitRef(cfg, repo, 'HEAD^{tree}');
      if (!head || !tree) return deny(503,'GIT_SOURCE_UNAVAILABLE');
      repositories.push({ name, head, tree });
    }
    return { status:200, body: { schema:'excession.repositories.v1', repositories } };
  }
  if (method !== 'POST') return deny(405,'METHOD_NOT_ALLOWED');
  const args = commandFor(path,data,cfg);
  if (!args) return deny(400,'INVALID_ARGUMENTS');
  // All three commands read exact Git revisions. Requests may not name
  // a checkout path, shell command, archive, URL, or receipt file.
  const result = await run(cfg.binary,args);
  if (!result.ok) return deny(502,'EXCESSION_REFUSED');
  let report;
  try { report = JSON.parse(result.output); }
  catch { return deny(502,'INVALID_EXCESSION_RESPONSE'); }
  const expected = {
    '/v1/agent/locate':'excession.agent-locate.v1',
    '/v1/agent/context':'excession.agent-context.v1',
    '/v1/agent/verify':'excession.agent-verification.v1'
  }[path];
  if (!report || report.schema !== expected)
    return deny(502,'INVALID_EXCESSION_RESPONSE');
  return { status:200, body: {
    schema:'excession.gateway.v1', repository:data.repository,
    report
  }};
}

export function createServer(cfg) {
  return http.createServer(async (req, res) => {
    if (!equalToken(cfg.token, req.headers.authorization)) {
      json(res, 401, {error:'UNAUTHORIZED'});
      return;
    }
    const path = (req.url || '').split('?')[0];
    if (!ROUTES.has(path)) { json(res,404,{error:'NOT_FOUND'}); return; }
    let bytes = 0;
    const chunks = [];
    try {
      for await (const chunk of req) {
        bytes += chunk.length;
        if (bytes > MAX_INPUT) {
          json(res,413,{error:'REQUEST_TOO_LARGE'});
          return;
        }
        chunks.push(chunk);
      }
    } catch { json(res,400,{error:'INVALID_REQUEST'}); return; }
    let input = {};
    if (req.method === 'POST') {
      try { input=JSON.parse(Buffer.concat(chunks).toString('utf8')); }
      catch { json(res,400,{error:'INVALID_JSON'}); return; }
    }
    try {
      const response = await route(req.method,path,input,cfg);
      json(res,response.status,response.body);
    } catch {
      json(res,503,{error:'UPSTREAM_UNAVAILABLE'});
    }
  });
}

if (process.argv[1] && import.meta.url === new URL('file://' + process.argv[1]).href) {
  const cfg = configuration();
  if (!Number.isInteger(cfg.port) || cfg.port < 1 || cfg.port > 65535)
    throw new Error('invalid PORT');
  createServer(cfg).listen(cfg.port, '0.0.0.0', () => {
    console.error('Excession read-only gateway listening');
  });
}
