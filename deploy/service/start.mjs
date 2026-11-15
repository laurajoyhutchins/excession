// Bootstrap only operator-configured Git remotes; requests never choose a remote.
import { spawnSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { serve } from './server.mjs';

const sources = JSON.parse(process.env.EXCESSION_SOURCES || '{}');
const root = resolve(process.env.EXCESSION_REPO_ROOT || '/tmp/excession-repos');
if (!sources || typeof sources !== 'object' || Array.isArray(sources) || !Object.keys(sources).length)
  throw Error('Configure EXCESSION_SOURCES as {"alias":"https://github.com/owner/repo.git"}');
mkdirSync(root,{recursive:true});
const repos = {};
for (const [alias,url] of Object.entries(sources)) {
  if (!/^[a-z][a-z0-9_-]{0,39}$/.test(alias) ||
      typeof url !== 'string' || !/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+(?:\.git)?$/.test(url))
    throw Error('Only explicit GitHub HTTPS repository sources are supported by bootstrap');
  const directory=join(root,alias);
  // Full history is necessary for base/candidate verification. Never use shallow clones.
  const clone=spawnSync('git',['clone','--quiet','--',url,directory],
    {encoding:'utf8',timeout:120000,env:{...process.env,GIT_TERMINAL_PROMPT:'0'}});
  if (clone.status !== 0) throw Error('Repository bootstrap failed: '+alias);
  repos[alias]=directory;
}
process.env.EXCESSION_REPOSITORIES=JSON.stringify(repos);
serve();
