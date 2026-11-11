'use strict';
// TypeScript-native, exact-span conservative effect inference. Never claims
// JavaScript semantic equivalence; only a uniquely transportable source edit.
const fs = require('node:fs');
const cp = require('node:child_process');
const path = require('node:path');
const crypto = require('node:crypto');
const ts = require(path.join(cp.execFileSync('npm', ['root','-g'], {encoding:'utf8'}).trim(), 'typescript'));
function refuse(msg) { throw Error(msg); }
const hash = text => crypto.createHash('sha256').update(text).digest('hex');
function parse(text) {
  const tree = ts.createSourceFile('relgit.js', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  if (tree.parseDiagnostics.length) refuse(ts.flattenDiagnosticMessageText(tree.parseDiagnostics[0].messageText, '\n'));
  return tree;
}
function locate(text, qualname, index) {
  const tree = parse(text);
  const parts = qualname.split('.');
  if (!(parts.length === 1 || parts.length === 2) || !parts.every(x => /^[A-Za-z_$][\w$]*$/.test(x))) refuse('unsupported JS target name');
  let bodies;
  if (parts.length === 1) {
    bodies = tree.statements.filter(n => ts.isFunctionDeclaration(n) && n.name?.text === parts[0] && n.body).map(n => n.body);
  } else {
    const classes = tree.statements.filter(n => ts.isClassDeclaration(n) && n.name?.text === parts[0]);
    if (classes.length !== 1) refuse('ambiguous JS class');
    bodies = classes[0].members.filter(n => ts.isMethodDeclaration(n) && n.name.getText(tree) === parts[1] && n.body).map(n => n.body);
  }
  if (bodies.length !== 1 || !Number.isSafeInteger(index) || index < 0 || index >= bodies[0].statements.length) refuse('ambiguous JS method or statement');
  const stmt = bodies[0].statements[index];
  if (!ts.isReturnStatement(stmt) && !ts.isExpressionStatement(stmt)) refuse('unsupported JS statement');
  if (!stmt.expression) refuse('JS statement lacks expression');
  if (ts.isExpressionStatement(stmt) && !ts.isCallExpression(stmt.expression)) refuse('JS expression must be a call');
  return {tree, stmt, expr:stmt.expression, range:[stmt.getStart(tree),stmt.getEnd()],text};
}
function descendants(root) {
  const a=[];
  function walk(n,depth) {
    if (ts.isExpression(n)) a.push({node:n,depth});
    ts.forEachChild(n,child => walk(child,depth+1));
  }
  walk(root,0); return a;
}
function fingerprint(node, tree) {
  return hash(`${node.kind}:${node.getText(tree)}`);
}
function infer(base,left,right,qualname,index) {
  const b=locate(base,qualname,index), l=locate(left,qualname,index), r=locate(right,qualname,index);
  if(b.stmt.kind !== l.stmt.kind || b.stmt.kind !== r.stmt.kind) refuse('statement kind changed');
  if(ts.isExpressionStatement(b.stmt) &&
    (b.expr.expression.getText(b.tree) !== l.expr.expression.getText(l.tree) ||
     b.expr.expression.getText(b.tree) !== r.expr.expression.getText(r.tree))) refuse('call target changed');
  for(const other of [l,r]) {
    if(base.slice(0,b.range[0])!==other.text.slice(0,other.range[0]) ||
       base.slice(b.range[1])!==other.text.slice(other.range[1])) refuse('undeclared changes outside JS statement');
  }
  if(fingerprint(b.expr,b.tree)===fingerprint(l.expr,l.tree) || fingerprint(b.expr,b.tree)===fingerprint(r.expr,r.tree)) refuse('one branch has no expression effect');
  const leftNodes=descendants(l.expr);
  const candidates=[];
  for(const {node:old,depth} of descendants(b.expr)) {
    const start=old.getStart(b.tree), end=old.getEnd();
    if(start< b.range[0] || end> b.range[1]) continue;
    const prefix=base.slice(0,start),suffix=base.slice(end);
    if(!right.startsWith(prefix) || !right.endsWith(suffix)) continue;
    const changed=right.slice(prefix.length,right.length-suffix.length);
    if(!changed || changed===base.slice(start,end)) continue;
    const newNode=descendants(r.expr).find(({node}) => node.getStart(r.tree)===start && node.getEnd()===start+changed.length);
    if(!newNode) continue;
    const matching=leftNodes.filter(({node})=> node.kind===old.kind && node.getText(l.tree)===old.getText(b.tree));
    if(matching.length!==1) continue;
    const leftAnchor=matching[0].node;
    const ls=leftAnchor.getStart(l.tree),le=leftAnchor.getEnd();
    const merged=left.slice(0,ls)+changed+left.slice(le);
    let m;
    try {m=locate(merged,qualname,index);} catch(e){continue;}
    const rightThenLeft=right.slice(0,r.range[0]) + left.slice(l.range[0],ls)+changed+left.slice(le,l.range[1]) +right.slice(r.range[1]);
    if(merged!==rightThenLeft) continue;
    candidates.push({depth, old,newNode:newNode.node,merged,m});
  }
  if(!candidates.length) refuse('no unique preserved JS subexpression');
  const deepest=Math.max(...candidates.map(x=>x.depth));
  const matches=candidates.filter(x=>x.depth===deepest);
  if(matches.length!==1) refuse('ambiguous JS preserved subexpression');
  const c=matches[0];
  return {content:c.merged,calculus:'UNIQUE_SUBEXPRESSION_CONTEXT_V1',statement_index:index,
    anchor_before_sha256:fingerprint(c.old,b.tree),anchor_after_sha256:fingerprint(c.newNode,r.tree),
    base_statement_sha256:fingerprint(b.stmt,b.tree),left_statement_sha256:fingerprint(l.stmt,l.tree),
    right_statement_sha256:fingerprint(r.stmt,r.tree),merged_statement_sha256:fingerprint(c.m.stmt,c.m.tree),
    anchor_path:[['ts_depth',c.depth]],source_confluence:true};
}
try {
  const v=JSON.parse(fs.readFileSync(0,'utf8'));
  process.stdout.write(JSON.stringify(infer(v.base,v.left,v.right,v.qualname,v.index)));
} catch(e) {process.stderr.write('INFER_EFFECT_REFUSAL: '+e.message+'\n');process.exit(2);}
