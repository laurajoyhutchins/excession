#!/usr/bin/env node
'use strict';

// Independent import oracle: TypeScript compiler API only. Neither Excession's
// source resolver nor the SCIP graph is read here. Package imports, nonliteral
// require, dynamic import and unresolved aliases are explicitly out of scope.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

function collect(root, files) {
  const tracked = new Set(files);
  const pairs = new Set();
  let unresolvedRelative = 0;
  let external = 0;
  let dynamic = 0;
  let importSites = 0;
  const options = {
    allowJs: true,
    noEmit: true,
    checkJs: false,
    moduleResolution: ts.ModuleResolutionKind.Node10,
    module: ts.ModuleKind.ESNext,
    resolveJsonModule: false,
  };
  function record(from, specifier) {
    importSites += 1;
    if (!specifier.startsWith('.')) {
      external += 1;
      return;
    }
    const resolved = ts.resolveModuleName(
      specifier, path.resolve(root, from), options, ts.sys
    ).resolvedModule;
    if (!resolved) {
      unresolvedRelative += 1;
      return;
    }
    const to = path.relative(root, path.resolve(resolved.resolvedFileName))
      .split(path.sep).join('/');
    if (!tracked.has(to)) {
      unresolvedRelative += 1;
      return;
    }
    if (from !== to) pairs.add(JSON.stringify([from, to]));
  }
  for (const from of files) {
    const text = fs.readFileSync(path.join(root, from), 'utf8');
    const kind = /\.[mc]?tsx?$/.test(from) ? ts.ScriptKind.TS : ts.ScriptKind.JS;
    const syntax = ts.createSourceFile(from, text, ts.ScriptTarget.Latest, true, kind);
    if (syntax.parseDiagnostics.length) {
      throw new Error('unparseable tracked TypeScript/JavaScript: ' + from);
    }
    function visit(node) {
      if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
        if (node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
          record(from, node.moduleSpecifier.text);
        }
      } else if (ts.isImportEqualsDeclaration(node) &&
                 ts.isExternalModuleReference(node.moduleReference) &&
                 node.moduleReference.expression &&
                 ts.isStringLiteral(node.moduleReference.expression)) {
        record(from, node.moduleReference.expression.text);
      } else if (ts.isCallExpression(node)) {
        if (node.expression.kind === ts.SyntaxKind.ImportKeyword) {
          dynamic += 1;
        } else if (ts.isIdentifier(node.expression) &&
                   node.expression.text === 'require') {
          if (node.arguments.length === 1 &&
              ts.isStringLiteral(node.arguments[0])) {
            record(from, node.arguments[0].text);
          } else {
            dynamic += 1;
          }
        }
      }
      ts.forEachChild(node, visit);
    }
    visit(syntax);
  }
  return {
    schema: 'excession.independent-ts-import-oracle.v1',
    parser: 'typescript-5.8.3-compiler-ast-and-resolveModuleName',
    tracked_files: files.length,
    pairs: [...pairs].map(JSON.parse).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b))),
    import_sites: importSites,
    unresolved_relative: unresolvedRelative,
    external_imports_excluded: external,
    dynamic_or_nonliteral_excluded: dynamic,
    semantic_runtime_dependencies_proven: false,
  };
}

if (require.main === module) {
  assert.equal(process.argv.length, 4, 'usage: node ts_import_oracle.cjs ROOT INVENTORY_JSON');
  const root = fs.realpathSync(process.argv[2]);
  const files = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
  assert(Array.isArray(files) && files.length > 0);
  assert(files.every(x => typeof x === 'string' &&
    !x.startsWith('/') && x.split('/').every(part => part && part !== '.' && part !== '..')));
  assert.equal(new Set(files).size, files.length);
  process.stdout.write(JSON.stringify(collect(root, files)) + '\n');
}
module.exports = { collect };
