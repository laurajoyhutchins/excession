'use strict';
// Optional JavaScript adapter. The universal Git/SQLite schema is unchanged.
// TypeScript's production parser is used for JS/TS syntax, never regex rewrites.
const fs = require('node:fs');
const crypto = require('node:crypto');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const globalRoot = execFileSync('npm', ['root', '-g'], {encoding:'utf8'}).trim();
const ts = require(path.join(globalRoot,'typescript'));
const items = JSON.parse(fs.readFileSync(0,'utf8'));
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
  out.push({path:f.path,imports,symbols,hazards,parse_errors:parsed});
}
process.stdout.write(JSON.stringify(out));
