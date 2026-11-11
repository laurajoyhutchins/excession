"""Python-native AST rewriting adapter. All repository, Git and SQLite mutations live in Rust.
This is a constrained extraction of the original RelGit v2 AST transformation.
It neither creates commits nor decides behavioral preservation.
"""
from __future__ import annotations
import ast
import importlib.util
import json
from pathlib import Path
import re
import sys
from typing import Any

class ProjectionError(ValueError):
    pass

def package_for(path: str) -> str:
    # Treat test files as an ordinary namespace package for import resolution.
    if path.endswith('/__init__.py'):
        return path[:-12].replace('/','.')
    return path.rsplit('/',1)[0].replace('/','.')


def absolute_name(from_path: str, level: int, module: str | None) -> str:
    if level==0: return module or ''
    return importlib.util.resolve_name('.'*level+(module or ''),package_for(from_path))


def relative_name(from_path: str, target: str) -> str:
    src=package_for(from_path).split('.')
    dst=target.split('.')
    common=0
    while common<min(len(src),len(dst)) and src[common]==dst[common]: common+=1
    if common==0: return target
    return '.'*(len(src)-common+1)+'.'.join(dst[common:])


def normalize_ast(source: bytes, from_path: str, new_to_old: dict[str,str],
                  internal: set[str]) -> str:
    """AST comparison uses original logical import targets, not source locations."""
    tree=ast.parse(source)
    for node in ast.walk(tree):
        if isinstance(node,ast.ImportFrom):
            absolute=absolute_name(from_path,node.level,node.module)
            canonical=new_to_old.get(absolute,absolute)
            if canonical in internal:
                node.level=0; node.module=canonical
        elif isinstance(node,ast.Import):
            for alias in node.names:
                alias.name=new_to_old.get(alias.name,alias.name)
    return ast.dump(tree,include_attributes=False)


def patch_python(source: bytes, old_path: str, new_path: str,
                 old_to_new: dict[str,str], old_names: set[str]) -> tuple[bytes,list[dict[str,Any]]]:
    tree=ast.parse(source,filename=old_path)
    if old_path!=new_path:
        for element in ast.walk(tree):
            if isinstance(element,ast.Name) and element.id in ('__file__','__name__','__package__','__spec__'):
                raise ProjectionError(f'location-sensitive Python runtime name {element.id} in {old_path}')
            if isinstance(element,ast.Call):
                call=element.func
                name=call.id if isinstance(call,ast.Name) else call.attr if isinstance(call,ast.Attribute) else ''
                if name in ('__import__','import_module','exec','eval'):
                    raise ProjectionError(f'dynamic import/evaluation in moved Python module {old_path}')
    lines=source.splitlines(keepends=True)
    offsets=[0]
    for line in lines: offsets.append(offsets[-1]+len(line))
    changes=[]
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            absolute=absolute_name(old_path,node.level,node.module)
            # A 'from package import submodule' statement can refer to a moved module.
            if node.module is None:
                targets=[absolute+'.'+a.name for a in node.names]
                if any(t in old_to_new for t in targets):
                    raise ProjectionError(f'ambiguous from-import module alias in {old_path}:{node.lineno}')
            resolved=old_to_new.get(absolute,absolute)
            if node.level and absolute not in old_names and not any(absolute.startswith(n+'.') for n in old_names):
                if old_path!=new_path:
                    raise ProjectionError(f'unresolved relative import in moved module {old_path}:{node.lineno}')
            if node.level:
                projected=relative_name(new_path,resolved)
            else:
                projected=resolved
            original='.'*node.level+(node.module or '')
            if projected==original: continue
            start=offsets[node.lineno-1]+node.col_offset
            # Source is bytes: AST offsets are also byte offsets.
            tail=source[start:]
            match=re.match(rb'from[ \t]+([.a-zA-Z_0-9]+)[ \t]+import\b',tail)
            if not match or match.group(1).decode('ascii')!=original:
                raise ProjectionError(f'unsupported Python import syntax {old_path}:{node.lineno}')
            left=start+match.start(1);right=start+match.end(1)
            changes.append((left,right,projected.encode('ascii'),absolute,resolved))
        elif isinstance(node,ast.Import):
            for alias in node.names:
                if alias.name in old_to_new:
                    # Without an explicit alias, `import a.b` binds `a`, and
                    # rewriting it would change name lookup. Keep old shim route.
                    if not alias.asname:
                        continue
                    # Preserve aliased absolute module import with a token edit.
                    start=offsets[node.lineno-1]+node.col_offset
                    end=offsets[node.end_lineno-1]+node.end_col_offset
                    segment=source[start:end]
                    exact=alias.name.encode('ascii')
                    matches=list(re.finditer(rb'(?<![\w.])'+re.escape(exact)+rb'(?=\s+as\s+)',segment))
                    if len(matches)!=1: raise ProjectionError('cannot safely rewrite absolute aliased import')
                    match=matches[0]
                    changes.append((start+match.start(),start+match.end(),old_to_new[alias.name].encode(),alias.name,old_to_new[alias.name]))
    # all byte mutations are confined to the identified import specifiers
    result=source
    edits=[]
    for left,right,value,old,new in sorted(changes,reverse=True):
        edits.append({'old_target':old,'new_target':new,'position':left})
        result=result[:left]+value+result[right:]
    ast.parse(result,filename=new_path)
    if normalize_ast(source,old_path,{},old_names)!=normalize_ast(
           result,new_path,{v:k for k,v in old_to_new.items()},old_names):
        raise ProjectionError(f'AST changed outside logical imports: {old_path}')
    return result,edits



def main():
    data=json.load(sys.stdin)
    if set(data)!={'files','moves','internal'}:
        raise ProjectionError('unexpected input fields')
    moves=data['moves']
    old_to_new={}
    for old,new in moves.items():
        def name(p):
            return p.removesuffix('.py').replace('/','.')
        old_to_new[name(old)]=name(new)
    internal=set(data['internal'])
    output=[]
    for item in data['files']:
        old=item['path']
        payload=item['content'].encode('utf8')
        new=moves.get(old,old)
        generated,edits=patch_python(payload,old,new,old_to_new,internal)
        output.append({'old_path':old,'new_path':new,'content':generated.decode('utf8'),'edits':edits})
    sys.stdout.write(json.dumps(output,sort_keys=True,separators=(',',':')))

if __name__=='__main__':
    main()
