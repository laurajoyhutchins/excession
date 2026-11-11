'use strict';
// Language adapter: TypeScript compiler AST identifies literal module references.
// It only rewrites string contents; no regex-based source parsing or formatting.
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const input = JSON.parse(fs.readFileSync(0,'utf8'));
const entries = new Set(input.paths);
const moved = new Map(Object.entries(input.moves));
function fail(message) { throw Error(message); }
function resolvePath(from, spec) {
  const prefix = path.posix.normalize(path.posix.join(path.posix.dirname(from),spec));
  for(const candidate of [prefix,prefix+'.js',prefix+'.cjs',prefix+'.mjs',prefix+'.ts',prefix+'.json',prefix+'/index.js',prefix+'/index.cjs',prefix+'/index.mjs']) {
    if(entries.has(candidate)) return candidate;
  }
  return null;
}
function relativePath(newFrom,newTo,oldSpec,originalTarget) {
  let spec = path.posix.relative(path.posix.dirname(newFrom),newTo);
  if(!spec.startsWith('.')) spec='./'+spec;
  const hasExtension = /\.(js|mjs|cjs|ts|json)$/.test(oldSpec);
  if(!hasExtension && newTo.endsWith('.js')) spec=spec.slice(0,-3);
  if(!hasExtension && originalTarget.endsWith('/index.js') && !moved.has(originalTarget)) {
    spec=spec.slice(0,-('/index'.length));
  }
  return spec;
}
const output=[];
for(const f of input.files){
  const kind=f.path.endsWith('.ts')?ts.ScriptKind.TS:ts.ScriptKind.JS;
  const tree=ts.createSourceFile(f.path,f.content,ts.ScriptTarget.Latest,true,kind);
  if(tree.parseDiagnostics.length) fail(`parse failed: ${f.path}: ${ts.flattenDiagnosticMessageText(tree.parseDiagnostics[0].messageText,' ')}`);
  const newPath=moved.get(f.path)||f.path;
  const edits=[]; const hazards=[];
  function record(node,syntax){
    if(!node || !ts.isStringLiteralLike(node)){hazards.push(`non-literal ${syntax}`);return;}
    const spec=node.text;
    if(!spec.startsWith('.')) return;
    const target=resolvePath(f.path,spec);
    if(!target){
      if(newPath!==f.path) fail(`unresolved relative ${syntax} in moved module ${f.path}: ${spec}`);
      hazards.push(`unresolved relative ${syntax}: ${spec}`);
      return;
    }
    const targetNew=moved.get(target)||target;
    const updated=relativePath(newPath,targetNew,spec,target);
    if(updated===spec) return;
    const start=node.getStart(tree),end=node.getEnd();
    const raw=f.content.slice(start,end),q=raw[0];
    if(!['\'','"','`'].includes(q)||raw[raw.length-1]!==q || raw.slice(1,-1)!==spec) fail(`unsupported escaped import literal in ${f.path}: ${raw}`);
    edits.push({start:start+1,end:end-1,before:spec,after:updated,target,newTarget:targetNew,syntax});
  }
  function visit(node){
    if(newPath!==f.path) {
      if(ts.isIdentifier(node) && (node.text==='__dirname'||node.text==='__filename'))
        fail(`location-sensitive JS runtime name ${node.text} in ${f.path}`);
      if(ts.isMetaProperty(node) && node.keywordToken===ts.SyntaxKind.ImportKeyword)
        fail(`import.meta is location sensitive in ${f.path}`);
    }
    if((ts.isImportDeclaration(node)||ts.isExportDeclaration(node))&&node.moduleSpecifier)
      record(node.moduleSpecifier,ts.isImportDeclaration(node)?'import':'export');
    if(ts.isCallExpression(node)){
      let kind=null;
      if(ts.isIdentifier(node.expression) && node.expression.text==='require') kind='require';
      if(node.expression.kind===ts.SyntaxKind.ImportKeyword) kind='import()';
      if(ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression) && node.expression.expression.text==='require' && node.expression.name.text==='resolve') kind='require.resolve';
      if(kind) record(node.arguments[0],kind);
    }
    ts.forEachChild(node,visit);
  }
  visit(tree);
  if(moved.has(f.path) && hazards.length) fail(`dynamic/unresolved import in moved module ${f.path}: ${hazards.join(', ')}`);
  let next=f.content;
  for(const edit of edits.sort((a,b)=>b.start-a.start)){
    if(next.slice(edit.start,edit.end)!==edit.before) fail(`offset corruption in ${f.path}`);
    next=next.slice(0,edit.start)+edit.after+next.slice(edit.end);
  }
  const parsed=ts.createSourceFile(newPath,next,ts.ScriptTarget.Latest,true,kind);
  if(parsed.parseDiagnostics.length) fail(`generated JS invalid: ${newPath}`);
  output.push({old_path:f.path,new_path:newPath,content:next,edits,hazards});
}
process.stdout.write(JSON.stringify(output));
