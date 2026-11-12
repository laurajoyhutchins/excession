'use strict';
// Optional JavaScript adapter. The universal Git/SQLite schema is unchanged.
// TypeScript's production parser is used for JS/TS syntax, never regex rewrites.
const fs = require('node:fs');
const crypto = require('node:crypto');
// Rust supplies an explicit trusted sidecar. Direct adapter tests may use
// the locally locked package, but the installed CLI never resolves via cwd.
const ts = process.env.EXCESSION_TYPESCRIPT_JS
  ? require(process.env.EXCESSION_TYPESCRIPT_JS)
  : require('typescript');
if (ts.version !== '5.8.3') throw Error('Unsupported TypeScript parser version');
const input = JSON.parse(fs.readFileSync(0,'utf8'));
const items = Array.isArray(input) ? input : input.entries;
if (!Array.isArray(items)) throw Error('Expected source entries');

// Native TypeScript resolution over pinned Git contents, never ambient files.
const posix = require('node:path').posix;
function gitSnapshot(entries, configuration) {
  const prefix = '/excession-git';
  const files = new Map();
  const source = new Set();
  const directories = new Set(['/', prefix]);
  function absolute(relative) {
    if (typeof relative !== 'string' || !relative || relative.startsWith('/') ||
        relative.includes('\\') || relative.split('/').some(p => !p || p === '.' || p === '..')) {
      throw Error('Invalid Git-relative adapter path');
    }
    return posix.join(prefix, relative);
  }
  for (const item of [...entries, ...configuration]) {
    const full = absolute(item.path);
    if (typeof item.content !== 'string' || files.has(full)) throw Error('Invalid or duplicate Git-backed source');
    files.set(full, item.content);
    for (let dir = posix.dirname(full); dir.startsWith(prefix); dir = posix.dirname(dir)) {
      directories.add(dir);
      if (dir === prefix) break;
    }
  }
  for (const item of entries) source.add(absolute(item.path));
  const host = {
    fileExists: name => files.has(posix.normalize(name)),
    readFile: name => files.get(posix.normalize(name)),
    directoryExists: name => directories.has(posix.normalize(name)),
    realpath: name => posix.normalize(name),
    useCaseSensitiveFileNames: () => true,
  };
  const cached = new Map();
  function compilerOptions(relative) {
    let dir = posix.dirname(absolute(relative));
    let config;
    while (dir.startsWith(prefix)) {
      for (const filename of ['tsconfig.json', 'jsconfig.json']) {
        const candidate = posix.join(dir, filename);
        if (files.has(candidate)) { config = candidate; break; }
      }
      if (config || dir === prefix) break;
      dir = posix.dirname(dir);
    }
    if (cached.has(config || 'default')) return cached.get(config || 'default');
    const defaults = {
      allowJs: true,
      moduleResolution: ts.ModuleResolutionKind.Node10,
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
    };
    let result = {options: defaults, problem: null};
    if (config) {
      const parsed = ts.parseConfigFileTextToJson(config, files.get(config));
      if (parsed.error || !parsed.config || typeof parsed.config !== 'object') {
        result = {options:null, problem:'invalid compiler configuration'};
      } else if (parsed.config.extends || parsed.config.references || parsed.config.compilerOptions?.plugins) {
        result = {options:null, problem:'unsupported compiler configuration inheritance/references/plugins'};
      } else {
        const converted = ts.convertCompilerOptionsFromJson(parsed.config.compilerOptions || {}, posix.dirname(config));
        result = converted.errors.length
          ? {options:null, problem:'unsupported compiler options'}
          : {options:{...defaults, ...converted.options}, problem:null};
      }
    }
    cached.set(config || 'default', result);
    return result;
  }
  return function resolve(path, specifier) {
    const {options, problem} = compilerOptions(path);
    if (problem) return {target_path:null, resolution_issue:problem};
    const resolved = ts.resolveModuleName(specifier, absolute(path), options, host).resolvedModule;
    if (!resolved) return {target_path:null, resolution_issue:'unresolved source import'};
    const filename = posix.normalize(resolved.resolvedFileName);
    if (!source.has(filename)) return {target_path:null, resolution_issue:'resolved target outside indexed source'};
    return {target_path:filename.slice(prefix.length + 1)};
  };
}

const resolveImport = Array.isArray(input) ? null : gitSnapshot(items, input.configs || []);
const printer = ts.createPrinter({removeComments:true});
const out=[];
for (const f of items) {
  const kind=f.path.endsWith('.ts')?ts.ScriptKind.TS:ts.ScriptKind.JS;
  const source=ts.createSourceFile(f.path,f.content,ts.ScriptTarget.Latest,true,kind);
  const imports=[];
  const symbols=[];
  const hazards=[];
  const parsed=source.parseDiagnostics.map(x=>({line:source.getLineAndCharacterOfPosition(x.start||0).line+1,message:ts.flattenDiagnosticMessageText(x.messageText,' ')}));
  function digest(t) {return crypto.createHash('sha256').update(t).digest('hex')}
  function addSymbol(n,name,skind) {
    symbols.push({name,kind:skind,signature:'',body_fingerprint:digest(printer.printNode(ts.EmitHint.Unspecified,n,source)),line:source.getLineAndCharacterOfPosition(n.getStart(source)).line+1});
  }
  for(const node of source.statements) {
    if(ts.isClassDeclaration(node) && node.name) addSymbol(node,node.name.text,'class');
    else if(ts.isFunctionDeclaration(node) && node.name) addSymbol(node,node.name.text,'function');
    else if(ts.isVariableStatement(node)) for(const dec of node.declarationList.declarations) if(ts.isIdentifier(dec.name)) addSymbol(dec,dec.name.text,'variable');
  }
  function visit(node) {
    if((ts.isImportDeclaration(node)||ts.isExportDeclaration(node))&&node.moduleSpecifier&&ts.isStringLiteral(node.moduleSpecifier)) {
      imports.push({specifier:node.moduleSpecifier.text,line:source.getLineAndCharacterOfPosition(node.getStart(source)).line+1,syntax:ts.isImportDeclaration(node)?'import':'export'});
    }
    if(ts.isCallExpression(node)) {
      const arg=node.arguments[0];
      let imp='';
      if(ts.isIdentifier(node.expression)&&(node.expression.text==='require'||node.expression.text==='eval')) imp=node.expression.text;
      if(node.expression.kind===ts.SyntaxKind.ImportKeyword) imp='import()';
      if(imp) {
        const line=source.getLineAndCharacterOfPosition(node.getStart(source)).line+1;
        if(arg&&ts.isStringLiteral(arg)&&imp!=='eval') imports.push({specifier:arg.text,line,syntax:imp});
        else hazards.push({line,reason:'dynamic:'+imp});
      }
    }
    ts.forEachChild(node,visit);
  }
  visit(source);
  if (resolveImport) {
    for (const item of imports) {
      const result = resolveImport(f.path, item.specifier);
      Object.assign(item, result);
      if (result.resolution_issue) {
        hazards.push({line:item.line, reason:result.resolution_issue+': '+item.specifier});
      }
    }
  }
  out.push({path:f.path,imports,symbols,hazards,parse_errors:parsed});
}
process.stdout.write(JSON.stringify(out));
