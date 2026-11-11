'use strict';
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const path = require('node:path');
const test = require('node:test');

const ROOT = path.resolve(__dirname, '..');
const TYPESCRIPT_VERSION = '5.8.3';

function runAdapter(name, input) {
  const result = spawnSync(process.execPath, [path.join(ROOT, 'core', 'adapters', name)], {
    input: JSON.stringify(input),
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}

test('the JS/TS parser is the locally pinned compiler', () => {
  const ts = require('typescript');
  assert.equal(ts.version, TYPESCRIPT_VERSION);
  assert.ok(require.resolve('typescript').startsWith(path.join(ROOT, 'node_modules')));
});

test('JS adapter extracts imports and declarations with the local parser', () => {
  const rows = runAdapter('js_adapter.cjs', [{
    path: 'src/a.ts',
    content: 'import { value } from "./b";\nexport function answer() { return value; }\n',
  }]);
  assert.equal(rows.length, 1);
  assert.ok(rows[0].imports.some(entry => entry.specifier === './b'));
  assert.ok(rows[0].symbols.some(entry => entry.name === 'answer'));
});

test('JS transform rewrites only a moved relative import', () => {
  const rows = runAdapter('js_transform.cjs', {
    paths: ['src/a.js', 'src/b.js'],
    moves: { 'src/a.js': 'lib/a.js' },
    files: [{
      path: 'src/a.js',
      content: "const b = require('./b');\n",
    }],
  });
  assert.equal(rows.length, 1);
  assert.equal(rows[0].new_path, 'lib/a.js');
  assert.equal(rows[0].content, "const b = require('../src/b');\n");
  assert.equal(rows[0].edits.length, 1);
});
