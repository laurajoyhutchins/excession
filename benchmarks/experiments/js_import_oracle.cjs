'use strict';
// Independent, Git-file-set-based resolution oracle. Never reads Excession SQL
// edges or imports its production JS adapter/resolution machinery.
const fs = require('node:fs');
const path = require('node:path').posix;
const {execFileSync} = require('node:child_process');
const ts = require(require('node:path').join(
  execFileSync('npm', ['root', '-g'], {encoding: 'utf8'}).trim(), 'typescript'
));
const files = JSON.parse(fs.readFileSync(0, 'utf8'));
const tracked = new Set(files.map(f => f.path));
const edges = [];
const unresolved = [];
const suffixes = ['.js', '.mjs', '.cjs', '.ts'];

function resolveRelative(source, specifier) {
  if (!specifier.startsWith('.')) return null;
  const base = path.normalize(path.join(path.dirname(source), specifier));
  if (base === '..' || base.startsWith('../') || base.startsWith('/')) return null;
  const candidates = [base];
  if (!suffixes.some(ext => base.endsWith(ext))) {
    for (const ext of suffixes) candidates.push(base + ext);
  }
  for (const ext of suffixes) candidates.push(path.join(base, 'index' + ext));
  return candidates.find(candidate => tracked.has(candidate)) ?? null;
}

for (const file of files) {
  const kind = file.path.endsWith('.ts') ? ts.ScriptKind.TS : ts.ScriptKind.JS;
  const source = ts.createSourceFile(
    file.path, file.content, ts.ScriptTarget.Latest, true, kind
  );
  for (const err of source.parseDiagnostics) {
    unresolved.push({
      path: file.path, line: source.getLineAndCharacterOfPosition(err.start || 0).line + 1,
      kind: 'parse-error'
    });
  }
  function add(node, specifier, syntax) {
    const line = source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
    if (!specifier.startsWith('.')) return; // external/package aliases: unproven
    const target = resolveRelative(file.path, specifier);
    if (target) edges.push({source: file.path, target, line, syntax});
    else unresolved.push({
      path: file.path, line, kind: 'unresolved-relative', specifier
    });
  }
  function visit(node) {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node))
        && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
      add(node, node.moduleSpecifier.text,
          ts.isImportDeclaration(node) ? 'import' : 'export');
    }
    if (ts.isCallExpression(node)) {
      let syntax = null;
      if (ts.isIdentifier(node.expression)
          && node.expression.text === 'require') syntax = 'require';
      if (node.expression.kind === ts.SyntaxKind.ImportKeyword) syntax = 'import()';
      if (syntax) {
        const arg = node.arguments[0];
        if (arg && ts.isStringLiteral(arg)) add(node, arg.text, syntax);
        else unresolved.push({
          path: file.path,
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
          kind: 'computed-' + syntax
        });
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
}
edges.sort((a,b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
process.stdout.write(JSON.stringify({edges, unresolved}));
