'use strict';
const assert=require('node:assert/strict');
const crypto=require('node:crypto');
const postcss=require('./postcss');
const output=[];
for(let i=0;i<100;i++) {
  const css=`p:nth-child(${i+1}) { color: red; margin: ${i}px }`;
  const ast=postcss.parse(css);
  assert.equal(ast.first.first.value,'red');
  const result=postcss(root=>root.walkDecls('color',d=>{d.value='blue'})).process(css,{from:undefined}).css;
  assert.ok(result.includes('blue'));
  output.push([ast.first.selector, result]);
}
console.log(JSON.stringify({sha256:crypto.createHash('sha256').update(JSON.stringify(output)).digest('hex'),
  trials:output.length, exported_type:typeof postcss, parser_type:typeof postcss.parse}));
