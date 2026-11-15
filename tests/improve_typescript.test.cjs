// Exact Git source -> native TypeScript edit -> Excession replay -> independent
// compiler/runtime acceptance, including intentionally unsupported cases.
'use strict';

const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {pathToFileURL} = require('node:url');
const {spawnSync} = require('node:child_process');
const ts = require('typescript');

const ROOT = path.resolve(__dirname, '..');
const PRODUCER = path.join(ROOT, 'core/adapters/improve_typescript.cjs');
const CLI = process.env.EXCESSION_BIN || path.join(ROOT, 'core/target/debug/excession');
assert.equal(ts.version, '5.8.3', 'must use the pinned TypeScript parser');
function run(cmd, args, cwd, allowedFailure = false) {
  const result = spawnSync(cmd, args, {
    cwd, encoding: 'utf8', timeout: 30000, maxBuffer: 2 * 1024 * 1024,
  });
  if (result.error || (!allowedFailure && result.status !== 0)) {
    throw Error(`${cmd} ${args.join(' ')}: ${result.error || result.stderr}`);
  }
  return result;
}
function fixture(suffix) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'excession-native-ts-'));
  const repo = path.join(root, 'repo');
  fs.mkdirSync(path.join(repo, 'src'), {recursive: true});
  const tsFile = suffix === 'ts';
  fs.writeFileSync(path.join(repo, 'src/main.' + suffix),
    '// Unicode before offsets: snowman ☃\n'
    + `import { used, unused } from './math${tsFile ? '' : '.js'}';\n`
    + `import './side-effect${tsFile ? '' : '.js'}';\n`
    + 'export const answer = used() * 2;\n');
  fs.writeFileSync(path.join(repo, 'src/math.' + suffix),
    'export function used() { return 21; }\n'
    + 'export function unused() { throw new Error("unused function invoked"); }\n');
  fs.writeFileSync(path.join(repo, 'src/side-effect.' + suffix),
    tsFile
      ? '(globalThis as any).__sideEffects = ((globalThis as any).__sideEffects || 0) + 1;\n'
      : 'globalThis.__sideEffects = (globalThis.__sideEffects || 0) + 1;\n');
  if (!tsFile) fs.writeFileSync(path.join(repo, 'package.json'), '{"type":"module"}\n');
  run('git', ['init', '-q', repo]);
  run('git', ['add', '.'], repo);
  run('git', ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
    'commit', '-qm', 'pinned source'], repo);
  const rev = run('git', ['rev-parse', 'HEAD'], repo).stdout.trim();
  return {
    root, repo, rev, suffix, source: 'src/main.' + suffix,
    plan: path.join(root, 'plan.json'), edits: path.join(root, 'edits.json'),
    candidate: path.join(root, 'candidate'),
  };
}
function propose(f, symbol = 'unused', extra = []) {
  return run(process.execPath, [
    PRODUCER, '--repo', f.repo, '--rev', f.rev, '--source', f.source,
    '--plan-out', f.plan, '--edits-out', f.edits,
    ...(symbol ? ['--symbol', symbol] : []), ...extra,
  ], ROOT, true);
}
function replay(f, apply = false) {
  return run(CLI, [
    'improve', 'replay', '--repo', f.repo, '--rev', f.rev,
    '--plan', f.plan, '--edits', f.edits,
    ...(apply ? ['--output', f.candidate] : []),
  ], ROOT);
}
for (const suffix of ['ts', 'js']) {
  test(`native TypeScript producer removes an unused ${suffix} import, retaining module effects`, t => {
    assert.ok(fs.existsSync(CLI), 'build Excession CLI before running native acceptance');
    const f = fixture(suffix);
    t.after(() => {
      if (fs.existsSync(f.candidate)) {
        run('git', ['worktree', 'remove', '--force', f.candidate], f.repo, true);
      }
      fs.rmSync(f.root, {recursive: true, force: true});
    });
    const before = fs.readFileSync(path.join(f.repo, f.source), 'utf8');
    assert.notEqual(propose(f, 'used').status, 0, 'a used import is not a native removal candidate');
    const result = propose(f);
    assert.equal(result.status, 0, result.stderr);
    const proposed = JSON.parse(result.stdout);
    assert.equal(proposed.status, 'NATIVE_EDIT_PROPOSED_UNVERIFIED');
    assert.equal(proposed.ready_to_merge, false);
    const plan = JSON.parse(fs.readFileSync(f.plan));
    const edits = JSON.parse(fs.readFileSync(f.edits));
    assert.equal(plan.source_commit, f.rev);
    assert.equal(plan.candidates[0].finding.locations[0].path, f.source);
    assert.equal(plan.coverage.status, 'PARTIAL');
    assert.equal(edits.native_producer, 'typescript-language-service@5.8.3');
    assert.ok(edits.edits.every(edit => edit.replacement === ''));
    assert.ok(edits.edits.every(edit => edit.end_byte > edit.start_byte));
    const preview = JSON.parse(replay(f).stdout);
    assert.equal(preview.status, 'UNVERIFIED_EDIT_PREVIEW');
    assert.equal(preview.source_changes, 1);
    assert.equal(fs.existsSync(f.candidate), false);
    const applied = JSON.parse(replay(f, true).stdout);
    assert.equal(applied.status, 'APPLIED_UNVERIFIED');
    assert.equal(applied.edit_plan_id, preview.edit_plan_id);
    assert.ok(applied.obligations.includes('NATIVE_PRODUCER_AUTHENTICITY_UNVERIFIED'));
    assert.equal(applied.ready_to_merge, false);
    const after = fs.readFileSync(path.join(f.candidate, f.source), 'utf8');
    assert.equal(fs.readFileSync(path.join(f.repo, f.source), 'utf8'), before);
    assert.ok(after.includes('import { used }'), 'must preserve the used binding');
    assert.ok(after.includes(`import './side-effect${suffix === 'ts' ? '' : '.js'}';`));
    assert.ok(!after.includes('unused }'));
    assert.ok(Buffer.byteLength(after) < Buffer.byteLength(before));
    const diff = run('git', ['diff', '--name-only'], f.candidate).stdout.trim();
    assert.equal(diff, f.source, 'must edit exactly the selected source file');
    if (suffix === 'ts') {
      const outDir = path.join(f.root, 'compiled');
      run(process.execPath, [
        require.resolve('typescript/bin/tsc'), '--strict', '--noUnusedLocals',
        '--target', 'ES2022', '--module', 'commonjs',
        '--moduleResolution', 'node', '--outDir', outDir,
        path.join(f.candidate, 'src/main.ts'),
        path.join(f.candidate, 'src/math.ts'),
        path.join(f.candidate, 'src/side-effect.ts'),
      ], f.candidate);
      const evaluated = run(process.execPath, ['-e',
        'const m=require(process.argv[1]); if(m.answer!==42 || globalThis.__sideEffects!==1) process.exit(2)',
        path.join(outDir, 'main.js')], f.candidate);
      assert.equal(evaluated.status, 0);
    } else {
      const executable = pathToFileURL(path.join(f.candidate, 'src/main.js')).href;
      const evaluated = run(process.execPath, [
        '--input-type=module', '-e',
        'const m=await import(process.argv[1]); if(m.answer!==42 || globalThis.__sideEffects!==1) process.exit(2)',
        executable,
      ], f.candidate);
      assert.equal(evaluated.status, 0);
    }
  });
}

test('native producer refuses to remove a sole named import and preserve unknown module effects', t => {
  const f = fixture('js');
  t.after(() => fs.rmSync(f.root, {recursive: true, force: true}));
  fs.writeFileSync(path.join(f.repo, f.source), "import { unused } from './math.js';\n");
  run('git', ['add', '.'], f.repo);
  run('git', ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
    'commit', '-qm', 'sole import case'], f.repo);
  f.rev = run('git', ['rev-parse', 'HEAD'], f.repo).stdout.trim();
  const result = propose(f);
  assert.notEqual(result.status, 0);
  assert.ok(!fs.existsSync(f.plan));
  assert.ok(!fs.existsSync(f.edits));
});

test('native producer rejects non-source paths, source files not in Git, and ambiguous names', t => {
  const f = fixture('ts');
  t.after(() => fs.rmSync(f.root, {recursive: true, force: true}));
  const invalid = run(process.execPath, [
    PRODUCER, '--repo', f.repo, '--rev', f.rev,
    '--source', '../private.ts', '--plan-out', f.plan, '--edits-out', f.edits,
  ], ROOT, true);
  assert.notEqual(invalid.status, 0);
  assert.notEqual(propose(f, 'not_imported').status, 0);
  assert.ok(!fs.existsSync(f.plan));
});
