// Native TypeScript language-service edit producer.
// Reads exact Git blob bytes. Never treats an unused-import diagnostic as proof
// that a repository's side effects, public API or configurations are preserved.
'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const {spawnSync} = require('node:child_process');
const ts = require('typescript');

if (ts.version !== '5.8.3') throw Error('requires the pinned TypeScript 5.8.3 language service');

function refuse(message) { throw Error(message); }
function git(repo, ...args) {
  const done = spawnSync('git', ['-C', repo, ...args], {
    encoding: 'buffer', timeout: 10000, maxBuffer: 1024 * 1024,
  });
  if (done.error || done.status !== 0) {
    refuse('Git source lookup failed: ' + (done.stderr || done.error || '').toString('utf8'));
  }
  return done.stdout;
}
function oid(text) {
  if (!/^[0-9a-f]{40}$/.test(text)) refuse('requires a lowercase SHA-1 Git object ID');
  return text;
}
function relativeSource(text) {
  if (!/^[a-zA-Z0-9_./-]{1,300}$/.test(text)
      || text.startsWith('/') || text.includes('\\')
      || text.split('/').some(part => !part || part === '.' || part === '..' || part.startsWith('-'))
      || !/\.(?:ts|tsx|js|jsx)$/.test(text) || text.endsWith('.d.ts')) {
    refuse('unsupported Git source path');
  }
  return text;
}
function field(obj, key) {
  if (obj[key] == null) refuse('missing ' + key);
  return obj[key];
}
function stable(value) {
  if (Array.isArray(value)) return '[' + value.map(stable).join(',') + ']';
  if (value && typeof value === 'object') {
    return '{' + Object.keys(value).sort().map(key =>
      JSON.stringify(key) + ':' + stable(value[key])
    ).join(',') + '}';
  }
  return JSON.stringify(value);
}
function digest(value) { return crypto.createHash('sha256').update(stable(value)).digest('hex'); }
function stamped(value, id) {
  if (id in value) refuse('duplicate content ID');
  return {...value, [id]: digest(value)};
}
function parseArgs(argv) {
  const allowed = new Set(['repo', 'rev', 'source', 'symbol', 'plan-out', 'edits-out']);
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]?.startsWith('--') && argv[i].slice(2);
    if (!key || !allowed.has(key) || args[key] !== undefined || !argv[i + 1]) {
      refuse('requires unique --repo --rev --source --plan-out --edits-out [--symbol]');
    }
    args[key] = argv[i + 1];
  }
  for (const required of ['repo', 'rev', 'source', 'plan-out', 'edits-out']) field(args, required);
  return args;
}
function sourceAt(repo, revision, selected) {
  const commit = oid(git(repo, 'rev-parse', '--verify', revision + '^{commit}').toString().trim());
  const tree = oid(git(repo, 'rev-parse', '--verify', commit + '^{tree}').toString().trim());
  const lines = git(repo, 'ls-tree', '-z', commit, '--', selected).toString('utf8')
    .split('\0').filter(Boolean);
  if (lines.length !== 1) refuse('source path not uniquely present in pinned Git tree');
  const match = lines[0].match(/^(100644|100755) blob ([0-9a-f]{40})\t(.+)$/);
  if (!match || match[3] !== selected) refuse('source is not a regular pinned Git blob');
  const blob = oid(match[2]);
  const bytes = git(repo, 'cat-file', 'blob', blob);
  if (bytes.length > 512 * 1024 || bytes.includes(0)) refuse('source is too large or binary');
  const source = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
  if (!Buffer.from(source, 'utf8').equals(bytes)) refuse('source is not exact UTF-8');
  return {commit, tree, blob, source};
}

function languageService(filename, source) {
  // Pinned source plus the installed TypeScript standard library only.
  // Never resolve imports from dirty files in the user's working tree.
  const options = {
    allowJs: true, checkJs: true, noUnusedLocals: true, noResolve: true,
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Node10, types: [],
  };
  const root = '/__excession_pinned__';
  const file = path.posix.join(root, filename);
  const libdir = path.dirname(ts.getDefaultLibFilePath(options));
  const library = name => {
    const canonical = path.resolve(name);
    if (canonical !== libdir && !canonical.startsWith(libdir + path.sep)) return undefined;
    return ts.sys.readFile(canonical);
  };
  const host = {
    getScriptFileNames: () => [file],
    getScriptVersion: () => '0',
    getScriptSnapshot: name => {
      const text = name === file ? source : library(name);
      return text == null ? undefined : ts.ScriptSnapshot.fromString(text);
    },
    getCurrentDirectory: () => root,
    getCompilationSettings: () => options,
    getDefaultLibFileName: ts.getDefaultLibFilePath,
    fileExists: name => name === file || library(name) !== undefined,
    readFile: name => name === file ? source : library(name),
    readDirectory: () => [],
    directoryExists: name => name === root || name === libdir || name.startsWith(libdir + path.sep),
    useCaseSensitiveFileNames: () => true,
    getNewLine: () => '\n',
  };
  return {service: ts.createLanguageService(host), file};
}
function nativeFix(source, filename, selectedSymbol) {
  const {service, file} = languageService(filename, source);
  const syntax = ts.createSourceFile(file, source, ts.ScriptTarget.ES2022, true);
  if (syntax.parseDiagnostics.length) refuse('native TypeScript parser found a syntax error');
  const imports = new Map();
  for (const node of syntax.statements) {
    if (!ts.isImportDeclaration(node)) continue;
    const bindings = node.importClause?.namedBindings;
    if (!bindings || !ts.isNamedImports(bindings) || bindings.elements.length < 2) continue;
    for (const item of bindings.elements) {
      const name = item.name.text;
      if (!/^[a-zA-Z_$][a-zA-Z0-9_$]{0,63}$/.test(name)) continue;
      if (selectedSymbol && name !== selectedSymbol) continue;
      imports.set(item.name.getStart(syntax), {node, name, length: item.name.getWidth(syntax)});
    }
  }
  const matches = [];
  for (const diag of service.getSemanticDiagnostics(file)) {
    if (diag.code !== 6133) continue;
    const found = imports.get(diag.start);
    if (!found || diag.length !== found.length) continue;
    const fixes = service.getCodeFixesAtPosition(
      file, diag.start, diag.start + diag.length, [diag.code], {}, {},
    ).filter(fix => fix.fixName === 'unusedIdentifier');
    if (fixes.length !== 1 || fixes[0].changes.length !== 1
        || fixes[0].changes[0].fileName !== file) continue;
    const operations = fixes[0].changes[0].textChanges.map(edit => ({
      start: edit.span.start,
      end: edit.span.start + edit.span.length,
      replacement: edit.newText,
    })).sort((a, b) => a.start - b.start);
    if (!operations.length || operations.some(e =>
      e.replacement !== '' || e.start >= e.end
      || e.start < found.node.getStart(syntax) || e.end > found.node.getEnd()
    )) continue;
    // Only a pure deletion within an existing multi-binding import is
    // supported. The module import itself cannot be removed.
    const merged = [];
    for (const edit of operations) {
      const prior = merged.at(-1);
      if (prior && edit.start <= prior.end) {
        if (edit.start < prior.end) refuse('ambiguous native overlapping fixes');
        prior.end = edit.end;
      } else merged.push({...edit});
    }
    let edited = source;
    for (const edit of [...merged].reverse()) {
      edited = edited.slice(0, edit.start) + edited.slice(edit.end);
    }
    const parsed = ts.createSourceFile(file, edited, ts.ScriptTarget.ES2022, true);
    if (edited === source || parsed.parseDiagnostics.length) continue;
    const edits = merged.map(edit => ({
      start_byte: Buffer.byteLength(source.slice(0, edit.start), 'utf8'),
      end_byte: Buffer.byteLength(source.slice(0, edit.end), 'utf8'),
      replacement: '',
    }));
    matches.push({name: found.name, diagnostic: diag.code, edits});
  }
  if (matches.length !== 1) refuse(
    'expected exactly one native unused named-import fix; select --symbol or refuse ambiguous source',
  );
  return matches[0];
}
function outPath(repo, target) {
  if (!path.isAbsolute(target) || fs.existsSync(target)) refuse('output artifact must be a new absolute path');
  const root = fs.realpathSync(repo);
  const parent = fs.realpathSync(path.dirname(target));
  if (parent === root || parent.startsWith(root + path.sep)) {
    refuse('artifact destination cannot be inside the original source checkout');
  }
  return path.join(parent, path.basename(target));
}
function main() {
  const options = parseArgs(process.argv.slice(2));
  const repo = fs.realpathSync(options.repo);
  const selected = relativeSource(options.source);
  const first = sourceAt(repo, options.rev, selected);
  const fix = nativeFix(first.source, selected, options.symbol);
  const evidence = {
    native_tool: 'typescript-language-service',
    native_version: ts.version,
    native_action: 'unusedIdentifier',
    diagnostic_code: fix.diagnostic,
    source_blob: first.blob,
    coverage: 'SINGLE_PINNED_SOURCE_FILE_ONLY',
    imported_symbol: fix.name,
  };
  const candidate = stamped({
    kind: 'unused_import_native_diagnostic',
    finding: {kind: 'unused_import_native_diagnostic', locations: [{path: selected, symbol: fix.name}]},
    priority: 'REVIEW_PRODUCER_EVIDENCE',
    edit_capability: 'TYPESCRIPT_LS_UNUSED_IMPORT',
    producer_evidence: evidence,
    automatic_deletion_authorized: false,
  }, 'candidate_id');
  const plan = stamped({
    schema: 'excession.improvement-plan.v1',
    producer: 'typescript-native',
    status: 'OBSERVATIONS_REQUIRE_TRANSFORMATION',
    source_commit: first.commit,
    source_tree: first.tree,
    producer_report_id: digest(evidence),
    coverage: {status: 'PARTIAL', observed: 'one source, selected TypeScript diagnostic'},
    candidate_count: 1,
    candidates: [candidate],
    ready_to_merge: false,
    automatic_deletion_authorized: false,
  }, 'plan_id');
  const native = stamped({
    schema: 'excession.improvement-native-edits.v1',
    native_producer: 'typescript-language-service@' + ts.version,
    source_commit: first.commit,
    source_tree: first.tree,
    improvement_plan_id: plan.plan_id,
    candidate_id: candidate.candidate_id,
    edits: fix.edits.map(edit => ({
      path: selected, original_blob: first.blob, ...edit,
    })),
  }, 'edit_plan_id');
  const planOut = outPath(repo, options['plan-out']);
  const editsOut = outPath(repo, options['edits-out']);
  if (planOut === editsOut) refuse('plan and edit artifacts require distinct destinations');
  fs.writeFileSync(planOut, stable(plan) + '\n', {flag: 'wx', mode: 0o600});
  try {
    fs.writeFileSync(editsOut, stable(native) + '\n', {flag: 'wx', mode: 0o600});
  } catch (error) {
    fs.unlinkSync(planOut);
    throw error;
  }
  process.stdout.write(JSON.stringify({
    status: 'NATIVE_EDIT_PROPOSED_UNVERIFIED', source_commit: first.commit,
    source: selected, symbol: fix.name, removed_ranges: fix.edits.length,
    plan_id: plan.plan_id, edit_plan_id: native.edit_plan_id,
    ready_to_merge: false,
  }) + '\n');
}
try { main(); } catch (error) {
  console.error('TypeScript native improvement refused:', error.message);
  process.exitCode = 1;
}
