"""Constrained CPython AST adapter for metadata rebinding. Rust owns all Git and SQL operations."""
from __future__ import annotations
import ast
import json
import sys
class Refuse(ValueError): pass
def declared_symbols(source: bytes) -> list[str]:
    """Only direct module declarations and undecorated class methods.

    Explicitly refuses descriptors, dynamic attribute generation and class
    decorators. Those require language-adapter semantics not established here.
    """
    tree=ast.parse(source)
    found:dict[str,ast.stmt]={}
    for node in tree.body:
        if isinstance(node,(ast.ClassDef,ast.FunctionDef,ast.AsyncFunctionDef)):
            found[node.name]=node
        elif isinstance(node,(ast.Assign,ast.AnnAssign)):
            targets = node.targets if isinstance(node,ast.Assign) else [node.target]
            if any(isinstance(t,ast.Name) and t.id in ('__getattr__','__dir__') for t in targets):
                raise Refuse('dynamic module attribute customization unsupported')
    result=[]
    for name,node in sorted(found.items()):
        if not name.isidentifier():raise Refuse('invalid declaration identifier')
        if isinstance(node,ast.ClassDef):
            if node.decorator_list:raise Refuse('class decorator requires separate preservation proof')
            result.append(name)
            methods:dict[str,ast.stmt]={}
            for child in node.body:
                if isinstance(child,(ast.FunctionDef,ast.AsyncFunctionDef)):
                    methods[child.name]=child
                elif isinstance(child,ast.ClassDef):
                    raise Refuse('nested class requires separate preservation proof')
            for method,definition in sorted(methods.items()):
                if definition.decorator_list:
                    raise Refuse('method descriptor/decorator requires separate preservation proof')
                result.append(name+'.'+method)
        else:
            if node.decorator_list:
                raise Refuse('runtime-decorated function requires separate preservation proof')
            result.append(name)
    if not result:raise Refuse('no safe direct authored declarations')
    return result


def epilogue(raw:bytes,logical:str,physical:str,original:bytes)->tuple[bytes,list[str],bytes]:
    if logical==physical or not logical or not physical or not logical.startswith('h11.'):
        raise Refuse('invalid logical-to-physical identity mapping')
    symbols=declared_symbols(raw)
    if symbols!=declared_symbols(original):
        raise Refuse('declaration set changed during relocation')
    if b'\n# RelGit generated logical module identities:' in raw:
        raise Refuse('identity epilogue already present')
    # Rebinding __module__ is a predictable, reviewable metadata projection.
    # No import statement, function body or class definition is edited.
    lines=[f'# RelGit generated logical module identities: {logical} -> {physical}',
           '# This does not change __file__, __spec__, __package__, or runtime path resolution.']
    lines += [f'{name}.__module__ = {logical!r}' for name in symbols]
    generated=('\n' if not raw.endswith(b'\n') else '')+'\n'+'\n'.join(lines)+'\n'
    payload=generated.encode('utf-8')
    transformed=raw+payload
    ast.parse(transformed)
    if transformed[:-len(payload)]!=raw:raise Refuse('source prefix changed')
    return transformed,symbols,payload


def main():
    data=json.load(sys.stdin)
    if set(data)!={'original','projected','logical','physical'}: raise Refuse('unexpected keys')
    new,names,suffix=epilogue(data['projected'].encode('utf8'),data['logical'],data['physical'],data['original'].encode('utf8'))
    sys.stdout.write(json.dumps({'content':new.decode('utf8'),'symbols':names,'suffix_hex':suffix.hex()},sort_keys=True,separators=(',',':')))
if __name__=='__main__': main()
