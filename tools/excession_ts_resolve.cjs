'use strict';
// Compiler-assisted module resolution, not an evaluator. Never require()
// project source: only the trusted installed TypeScript compiler API.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const Module = require('node:module');
const builtins = new Set(Module.builtinModules.map(x => x.replace(/^node:/, '')));
function fail(message) { throw new Error('UNSUPPORTED: ' + message); }
const [apiPath, rootArg, entry] = process.argv.slice(2);
const root = fs.realpathSync(rootArg);
const ts = require(apiPath);
function inside(file) {
  const real = fs.realpathSync(file);
  const rel = path.relative(root, real).split(path.sep).join('/');
  if (!rel || rel.startsWith('../') || rel === '..' || path.isAbsolute(rel)) fail('dependency outside snapshot: ' + file);
  return rel;
}
function exists(file) { return fs.existsSync(file) && fs.statSync(file).isFile(); }
const entryFile = path.join(root, entry);
if (!exists(entryFile)) fail('missing entry');
let configPath;
for (let dir = path.dirname(entryFile); dir.startsWith(root); dir = path.dirname(dir)) {
  const candidate = path.join(dir, 'tsconfig.json');
  if (exists(candidate)) { configPath = candidate; break; }
  if (dir === root) break;
}
let options = {allowJs: true, checkJs: false, resolveJsonModule: true,
               moduleResolution: ts.ModuleResolutionKind.Node10,
               target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS,
               types: [], noEmit: true};
const configuration = new Set();
if (configPath) {
  configuration.add(inside(configPath));
  const read = ts.readConfigFile(configPath, ts.sys.readFile);
  if (read.error) fail(ts.flattenDiagnosticMessageText(read.error.messageText, '\n'));
  if (read.config.extends || read.config.references) fail('tsconfig extends/references need additional resolution');
  if (read.config.compilerOptions?.plugins) fail('tsconfig plugins not supported');
  const parsed = ts.parseJsonConfigFileContent(read.config, ts.sys, path.dirname(configPath));
  if (parsed.errors.length) fail(parsed.errors.map(d => ts.flattenDiagnosticMessageText(d.messageText,' ')).join('; '));
  options = {...options, ...parsed.options, allowJs:true, noEmit:true, types: []};
}
// Input identity binds the effective compiler options, including aliases.
const optionsHash = crypto.createHash('sha256').update(JSON.stringify(
  Object.fromEntries(Object.entries(options).sort(([a],[b])=>a.localeCompare(b))))).digest('hex');
const sourceSet = new Set();
const pending = [entryFile];
function trackConfigs(file) {
  let dir = path.dirname(file);
  while (dir.startsWith(root)) {
    const pkg = path.join(dir, 'package.json');
    if (exists(pkg)) configuration.add(inside(pkg));
    if (dir === root) break;
    dir = path.dirname(dir);
  }
}
function queueResolved(spec, origin, mode) {
  if (spec.startsWith('node:') && builtins.has(spec.slice(5))) return;
  if (builtins.has(spec)) return;
  if (mode === 'require') {
    // Node's own CommonJS package resolution binds actual runtime code rather
    // than a TypeScript .d.ts sibling or a guessed extension.
    let runtime;
    try { runtime = Module.createRequire(origin).resolve(spec); }
    catch (e) { fail('Node cannot resolve require('+spec+'): '+e.message); }
    inside(runtime);
    pending.push(runtime);
    return;
  }
  const resolved = ts.resolveModuleName(spec, origin, options, ts.sys);
  if (!resolved.resolvedModule) fail('unresolved import ' + spec + ' in ' + inside(origin));
  let file = resolved.resolvedModule.resolvedFileName;
  if (!exists(file)) fail('resolved module absent: ' + file);
  const matched = inside(file);
  if (matched.endsWith('.d.ts') && !origin.endsWith('.d.ts')) {
    // TypeScript may resolve type declarations instead of the JS runtime file.
    // Include the sibling implementation when present; otherwise refuse rather
    // than claiming to have found the runtime implementation.
    const stem = file.slice(0,-5);
    const candidates = [stem+'.js', stem+'.cjs', stem+'.mjs', stem+'.ts', stem+'.tsx'];
    const implementation = candidates.filter(exists);
    if (implementation.length !== 1) fail('declaration-only or ambiguous runtime module: ' + spec);
    pending.push(implementation[0]);
  }
  pending.push(file);
}
while (pending.length) {
  const file = pending.pop();
  const rel = inside(file);
  if (sourceSet.has(rel)) continue;
  sourceSet.add(rel);
  trackConfigs(file);
  if (rel.endsWith('.json')) continue; // Imported JSON is a literal input.
  const source = fs.readFileSync(file, 'utf8');
  const kind = rel.endsWith('.ts') || rel.endsWith('.tsx') || rel.endsWith('.mts') || rel.endsWith('.cts') || rel.endsWith('.d.ts') ? ts.ScriptKind.TS : ts.ScriptKind.JS;
  const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, kind);
  if (parsed.parseDiagnostics.length) fail('syntax error in '+rel);
  const pre = ts.preProcessFile(source, true, true);
  if (pre.typeReferenceDirectives.length || pre.libReferenceDirectives.length) fail('ambient type/lib references need explicit resolution: '+rel);
  for (const ref of pre.referencedFiles) {
    const referenced = path.resolve(path.dirname(file), ref.fileName);
    if (!exists(referenced)) fail('unresolved triple-slash reference: '+ref.fileName);
    pending.push(referenced);
  }
  const imports = [];
  function visit(node) {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      if (node.moduleSpecifier) {
        if (!ts.isStringLiteral(node.moduleSpecifier)) fail('computed import/export: '+rel);
        imports.push([node.moduleSpecifier.text, 'import']);
      }
    } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
      if (!node.moduleReference.expression || !ts.isStringLiteral(node.moduleReference.expression)) fail('computed import-equals: '+rel);
      imports.push([node.moduleReference.expression.text, 'import']);
    } else if (ts.isCallExpression(node)) {
      const name = ts.isIdentifier(node.expression) ? node.expression.text : null;
      if (name === 'require' || node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        if (node.arguments.length !== 1 || !ts.isStringLiteralLike(node.arguments[0])) fail('dynamic import/require: '+rel);
        imports.push([node.arguments[0].text, name === 'require' ? 'require' : 'import']);
      } else if (ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'require') {
        fail('indirect property require requires explicit effects: '+rel);
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(parsed);
  for (const [imported, mode] of imports) queueResolved(imported, file, mode);
}
for (const f of ['package-lock.json','npm-shrinkwrap.json','pnpm-lock.yaml','yarn.lock','tsconfig.json','jsconfig.json']) {
  if (exists(path.join(root,f))) configuration.add(f);
}
process.stdout.write(JSON.stringify({sources:[...sourceSet].sort(),configuration:[...configuration].sort(),
    module_resolution:String(options.moduleResolution),options_sha256:optionsHash})+'\n');
