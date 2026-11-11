"""Read-only CPython AST adapter for RelGit's Rust indexing core.

Input: JSON [{"name": str, "path": str, "content": str}]. Output:
JSON [{"name": str, "path": str, "imports": [], "symbols": [],
       "exports": [], "hazards": []}]. This does not execute target code.

This interpreter-dependent adapter deliberately remains Python. Import resolution,
AST and ast.unparse are CPython semantics, not properties of Git or SQLite.
"""
import ast
import hashlib
import importlib.util
import json
import sys


def digest(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(",", ":"),ensure_ascii=False).encode()).hexdigest()


def imports(node, source_name, source_path, available):
    if isinstance(node, ast.Import):
        choices = [alias.name for alias in node.names]
        syntax = 'import'
    elif isinstance(node, ast.ImportFrom):
        package = source_name if source_path.endswith('/__init__.py') else source_name.rsplit('.', 1)[0]
        if node.level:
            try:
                base = importlib.util.resolve_name('.' * node.level + (node.module or ''), package)
            except ImportError:
                return []
        else:
            base = node.module or ''
        choices = [base]
        choices += [base + '.' + alias.name for alias in node.names if alias.name != '*']
        syntax = 'from'
    else:
        return []
    out = []
    for choice in choices:
        parts = choice.split('.')
        matched = ['.'.join(parts[:i]) for i in range(1, len(parts)+1) if '.'.join(parts[:i]) in available]
        if matched:
            target = max(matched, key=len)
            edge = {'target':target, 'line':node.lineno, 'syntax':syntax}
            if edge not in out: out.append(edge)
    return out


def symbols(tree):
    result=[]
    def visit(nodes, prefix=''):
        for node in nodes:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                ident=prefix+node.name
                kind = 'class' if isinstance(node,ast.ClassDef) else ('async_function' if isinstance(node,ast.AsyncFunctionDef) else 'function')
                signature=ast.unparse(node.args) if not isinstance(node,ast.ClassDef) else ast.unparse(ast.Tuple(elts=node.bases,ctx=ast.Load()))
                result.append({'name':ident,'kind':kind,'signature':signature,'body_fingerprint':digest(ast.dump(node,include_attributes=False)),'line':node.lineno})
                if isinstance(node,ast.ClassDef):visit(node.body,ident+'.')
    visit(tree.body)
    return result


def exports(tree):
    explicit=None; targets={}
    for node in tree.body:
        if isinstance(node,ast.Assign):
            if any(isinstance(t,ast.Name) and t.id=='__all__' for t in node.targets):
                if isinstance(node.value,(ast.List,ast.Tuple)) and all(isinstance(x,ast.Constant) and isinstance(x.value,str) for x in node.value.elts):
                    explicit={x.value for x in node.value.elts}
            for t in node.targets:
                if isinstance(t,ast.Name) and t.id!='__all__':targets[t.id]='<assignment>'
        elif isinstance(node,(ast.ClassDef,ast.FunctionDef,ast.AsyncFunctionDef)):
            targets[node.name]='<definition>'
        elif isinstance(node,ast.ImportFrom):
            for a in node.names:
                if a.name!='*':targets[a.asname or a.name]=('.'*node.level)+(node.module or '')+':'+a.name
        elif isinstance(node,ast.Import):
            for a in node.names:targets[a.asname or a.name.split('.')[0]]=a.name
    return [{'name':n,'target':t,'explicitly_exported':int(n in explicit if explicit is not None else not n.startswith('_'))} for n,t in sorted(targets.items())]


def parse_item(item,available):
    mod=item['name'];path=item['path'];code=item['content']
    result={'name':mod,'path':path,'imports':[],'symbols':[],'exports':[],'hazards':[]}
    try:tree=ast.parse(code,filename=path)
    except (SyntaxError,UnicodeDecodeError):
        result['hazards'].append({'line':1,'reason':'unparseable source'})
        return result
    for stmt in ast.walk(tree):
        if isinstance(stmt,(ast.Import,ast.ImportFrom)):
            result['imports'].extend(imports(stmt,mod,path,available))
            if isinstance(stmt,ast.ImportFrom) and any(a.name=='*' for a in stmt.names):
                result['hazards'].append({'line':stmt.lineno,'reason':'wildcard import'})
        if isinstance(stmt,ast.Call):
            fn=stmt.func
            maybe=fn.id if isinstance(fn,ast.Name) else fn.attr if isinstance(fn,ast.Attribute) else ''
            if maybe in ('__import__','import_module','exec','eval'):
                result['hazards'].append({'line':stmt.lineno,'reason':'dynamic: '+maybe})
    result['symbols']=symbols(tree)
    if path.endswith('/__init__.py'): result['exports']=exports(tree)
    return result


def main():
    items=json.load(sys.stdin)
    if not isinstance(items,list): raise ValueError('invalid module input')
    for item in items:
        if not isinstance(item,dict) or set(item)!= {'name','path','content'}: raise ValueError('invalid input record')
        if not all(isinstance(v,str) for v in item.values()): raise ValueError('invalid input types')
    available={item['name'] for item in items}
    if len(available)!=len(items):raise ValueError('duplicate logical module names')
    result=[parse_item(item,available) for item in items]
    print(json.dumps(result,sort_keys=True,separators=(',',':'),ensure_ascii=False))

if __name__=='__main__': main()
