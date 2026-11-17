#!/usr/bin/env node
'use strict';

// Source-level semantic oracle from TypeScript's compiler, not from SCIP
// occurrences or Excession's graph. Conservative symbol identity only:
// exclude instance methods, indirect calls, dynamic imports and unresolved
// declarations. A resolved source symbol is not a runtime dispatch proof.
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

function oracle(root, relativeFiles) {
  const files = [...relativeFiles].sort();
  const tracked = new Set(files);
  const abs = files.map(f => path.resolve(root, f));
  const program = ts.createProgram(abs, {
    allowJs: true, checkJs: false, noEmit: true, skipLibCheck: true,
    module: ts.ModuleKind.CommonJS,
    moduleResolution: ts.ModuleResolutionKind.Node10,
    target: ts.ScriptTarget.ES2022,
  });
  const checker = program.getTypeChecker();
  const sites = new Map();
  const unknown = {dynamic_calls: 0, unresolved_calls: 0,
                   unresolved_types: 0, instance_methods_excluded: 0,
                   external_declarations: 0};

  function resolve(symbol, from, line, kind) {
    if (!symbol) {
      unknown[kind === 'type' ? 'unresolved_types' : 'unresolved_calls']++;
      return;
    }
    if (symbol.flags & ts.SymbolFlags.Alias) {
      symbol = checker.getAliasedSymbol(symbol);
    }
    if (!symbol || !Array.isArray(symbol.declarations)) {
      unknown[kind === 'type' ? 'unresolved_types' : 'unresolved_calls']++;
      return;
    }
    let matched = false;
    for (const declaration of symbol.declarations) {
      const target = path.relative(root, declaration.getSourceFile().fileName)
        .split(path.sep).join('/');
      if (!tracked.has(target) || from === target) continue;
      // The call/type site itself must be anchored in source. Do not treat
      // a package's external .d.ts declaration as a reachable Git definition.
      if (declaration.getSourceFile().isDeclarationFile) {
        unknown.external_declarations++;
        continue;
      }
      const key = JSON.stringify([from, target, line, kind]);
      sites.set(key, {source: from, target, line, kind,
        symbol: symbol.getName()});
      matched = true;
    }
    if (!matched) unknown[kind === 'type' ? 'unresolved_types' : 'unresolved_calls']++;
  }

  for (const f of files) {
    const file = program.getSourceFile(path.resolve(root, f));
    if (!file) throw new Error('TS compiler did not load a tracked source: ' + f);
    if (file.parseDiagnostics.length) {
      throw new Error('Syntax error in tracked source: ' + f);
    }
    function visit(node) {
      if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
        const callee = node.expression;
        if (ts.isIdentifier(callee)) {
          const symbol = checker.getSymbolAtLocation(callee);
          resolve(symbol, f, file.getLineAndCharacterOfPosition(callee.getStart(file)).line + 1,
                  'call');
        } else if (ts.isPropertyAccessExpression(callee)) {
          // A namespace import's direct exported property can have a stable
          // symbol definition. A general value receiver is dynamic dispatch.
          if (ts.isIdentifier(callee.expression)) {
            const receiver = checker.getSymbolAtLocation(callee.expression);
            const namespace = receiver && (receiver.declarations || []).some(
              d => ts.isNamespaceImport(d) || ts.isImportEqualsDeclaration(d));
            if (namespace) {
              resolve(checker.getSymbolAtLocation(callee.name), f,
                      file.getLineAndCharacterOfPosition(callee.getStart(file)).line + 1,
                      'call');
            } else {
              unknown.instance_methods_excluded++;
            }
          } else {
            unknown.instance_methods_excluded++;
          }
        } else {
          unknown.dynamic_calls++;
        }
      } else if (ts.isTypeReferenceNode(node)) {
        const symbol = checker.getSymbolAtLocation(node.typeName);
        resolve(symbol, f,
                file.getLineAndCharacterOfPosition(node.typeName.getStart(file)).line + 1,
                'type');
      } else if (ts.isExpressionWithTypeArguments(node)) {
        // Only heritage types, not arbitrary expressions.
        const symbol = checker.getSymbolAtLocation(node.expression);
        resolve(symbol, f,
                file.getLineAndCharacterOfPosition(node.expression.getStart(file)).line + 1,
                'type');
      }
      ts.forEachChild(node, visit);
    }
    visit(file);
  }
  return {
    schema: 'excession.independent-typescript-symbol-sites.v1',
    oracle: 'typescript_5_8_3_getSymbolAtLocation_declaring_source',
    sites: [...sites.values()].sort((a,b) =>
      JSON.stringify([a.source,a.line,a.kind,a.target])
        .localeCompare(JSON.stringify([b.source,b.line,b.kind,b.target]))),
    counts: {
      call: [...sites.values()].filter(s => s.kind === 'call').length,
      type: [...sites.values()].filter(s => s.kind === 'type').length,
    },
    unsupported: unknown,
    runtime_dispatch_proven: false,
    cross_language_links_proven: false,
    shared_compiler_family_with_scip_typescript: true,
  };
}

if (require.main === module) {
  if (process.argv.length !== 4) throw new Error('Usage: ts_symbol_oracle.cjs ROOT INVENTORY_JSON');
  const root = fs.realpathSync(process.argv[2]);
  const files = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
  if (!Array.isArray(files) || files.length === 0 ||
      new Set(files).size !== files.length ||
      files.some(f => typeof f !== 'string' || path.isAbsolute(f) ||
        f.split('/').some(p => !p || p === '.' || p === '..'))) {
    throw new Error('Invalid pinned Git source inventory');
  }
  process.stdout.write(JSON.stringify(oracle(root, files)) + '\n');
}
module.exports = {oracle};
