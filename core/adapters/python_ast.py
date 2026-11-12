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
            target = available[max(matched, key=len)]
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


def parse_item(item,available,import_name):
    mod=item['name'];path=item['path'];code=item['content']
    result={'name':mod,'path':path,'imports':[],'symbols':[],'exports':[],'hazards':[]}
    try:tree=ast.parse(code,filename=path)
    except (SyntaxError,UnicodeDecodeError):
        result['hazards'].append({'line':1,'reason':'unparseable source'})
        return result
    for stmt in ast.walk(tree):
        if isinstance(stmt,(ast.Import,ast.ImportFrom)):
            result['imports'].extend(imports(stmt,import_name,path,available))
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


def importable_names(items):
    """Separate stable SQL identities from importable names in regular packages.

    The highest tracked __init__.py ancestor supplies the package boundary.
    A tree without an observed package marker has no inferred sys.path root:
    retain its indexed name rather than guessing a directory to discard.
    """
    paths={item['path'] for item in items}
    package_dirs={path.rsplit('/',1)[0] for path in paths
                  if path.endswith('/__init__.py')}
    available={item['name']:item['name'] for item in items}
    if len(available)!=len(items): raise ValueError('duplicate logical module names')
    chosen={}
    for item in items:
        indexed=item['name']
        components=item['path'].split('/')
        package_start=next(
            (i for i in range(len(components)-1)
             if '/'.join(components[:i+1]) in package_dirs),None
        )
        if package_start is None:
            import_name=indexed
        else:
            relative=components[package_start:]
            if relative[-1]=='__init__.py':
                relative=relative[:-1]
            else:
                relative[-1]=relative[-1][:-3]
            import_name='.'.join(relative)
        if not import_name: raise ValueError('empty importable module identity')
        previous=available.get(import_name)
        if previous is not None and previous!=indexed:
            raise ValueError('ambiguous importable Python module: '+import_name)
        available[import_name]=indexed
        chosen[indexed]=import_name
    return available,chosen


def main():
    payload=json.load(sys.stdin)
    if isinstance(payload,list):
        # Original full-analysis input remains unchanged.
        items=payload
        catalog=items
    elif isinstance(payload,dict) and set(payload)=={'catalog','entries'}:
        # Rust supplies the entire module catalog, but only changed source
        # contents. Import resolution must never use the changed subset alone.
        catalog=payload['catalog']
        items=payload['entries']
        if not isinstance(catalog,list):
            raise ValueError('invalid module catalog')
        if any(not isinstance(entry,dict) or
               set(entry)!={'name','path'} or
               not all(isinstance(value,str) for value in entry.values())
               for entry in catalog):
            raise ValueError('invalid catalog entry')
        if len({entry['name'] for entry in catalog})!=len(catalog):
            raise ValueError('ambiguous module catalog')
        full={entry['name']:entry['path'] for entry in catalog}
        if len({entry['path'] for entry in catalog})!=len(catalog):
            raise ValueError('duplicate catalog path')
    else:
        raise ValueError('invalid module input')
    if not isinstance(items,list):
        raise ValueError('invalid module records')
    for item in items:
        if not isinstance(item,dict) or set(item)!= {'name','path','content'}:
            raise ValueError('invalid input record')
        if not all(isinstance(v,str) for v in item.values()):
            raise ValueError('invalid input types')
        if isinstance(payload,dict) and full.get(item['name'])!=item['path']:
            raise ValueError('source not in immutable module catalog')
    if len({item['name'] for item in items})!=len(items):
        raise ValueError('duplicate source module')
    available,names=importable_names(catalog)
    result=[parse_item(item,available,names[item['name']]) for item in items]
    print(json.dumps(result,sort_keys=True,separators=(',',':'),ensure_ascii=False))

if __name__=='__main__': main()
