"""Tiny, explicitly finite algebra of overlapping Python statement effects.

No arbitrary user-provided source or expression is injected. The exact AST and
exact bytes of a single write call must match a known state. The two effects
update different semantic attributes of that same write operation.

This establishes source-state confluence for the supported rewrite vocabulary,
NOT equivalence of the resulting program. Rust must still run fresh joint tests.
"""
import ast
import json
import sys

LENGTHS = {
    'raw': 'len(data)',
    'bytes_snapshot': 'len(bytes(data))',
    'drop_last': 'len(data[:-1])',
}
CODECS = {
    'percent': lambda length: f'write(b"%x\\r\\n" % {length})',
    'ascii_format': lambda length: f'write(format({length}, "x").encode("ascii") + b"\\r\\n")',
}
OPERATIONS = {
    'codec.ascii_format': ('codec', 'percent', 'ascii_format'),
    'length.bytes_snapshot': ('length', 'raw', 'bytes_snapshot'),
    'length.drop_last': ('length', 'raw', 'drop_last'),
}

def replacement(source, qualname, index, operation):
    if operation not in OPERATIONS:
        raise ValueError('unknown typed operation')
    if not isinstance(qualname, str) or len(qualname.split('.')) != 2:
        raise ValueError('expected Class.method')
    cls, method_name = qualname.split('.')
    if not cls.isidentifier() or not method_name.isidentifier() or type(index) is not int:
        raise ValueError('invalid target')
    tree = ast.parse(source)
    classes = [x for x in tree.body if isinstance(x, ast.ClassDef) and x.name == cls]
    if len(classes) != 1:
        raise ValueError('target class not unique')
    methods = [x for x in classes[0].body if isinstance(x, ast.FunctionDef) and x.name == method_name]
    if len(methods) != 1 or not 0 <= index < len(methods[0].body):
        raise ValueError('method or statement index invalid')
    stmt = methods[0].body[index]
    if not isinstance(stmt, ast.Expr) or stmt.lineno != stmt.end_lineno:
        raise ValueError('target must be one expression statement')
    line = ast.get_source_segment(source, stmt)
    if line is None:
        raise ValueError('could not resolve exact source bytes')
    candidates = {}
    for codec, writer in CODECS.items():
        for length, value in LENGTHS.items():
            canonical = writer(value)
            parsed = ast.parse(canonical).body[0]
            if ast.dump(stmt, include_attributes=False) == ast.dump(parsed, include_attributes=False):
                candidates[(codec, length)] = canonical
    if len(candidates) != 1:
        raise ValueError('unknown or ambiguous statement shape')
    (codec, length), canonical_old = next(iter(candidates.items()))
    if line != canonical_old:
        raise ValueError('source not canonical exact byte representation')
    dimension, required, effect = OPERATIONS[operation]
    value = codec if dimension == 'codec' else length
    if value != required:
        raise ValueError(f'operation precondition failed: {dimension} expected {required} got {value}')
    if dimension == 'codec':
        codec = effect
    else:
        length = effect
    canonical_new = CODECS[codec](LENGTHS[length])
    # Exact one-line UTF-8 source span replacement, not an AST unparse of file.
    data = source.encode('utf-8')
    lines = data.splitlines(keepends=True)
    start = sum(len(x) for x in lines[:stmt.lineno-1]) + stmt.col_offset
    end = sum(len(x) for x in lines[:stmt.end_lineno-1]) + stmt.end_col_offset
    if data[start:end] != canonical_old.encode('utf-8'):
        raise ValueError('target bytes differ from AST model')
    out = (data[:start]+canonical_new.encode('utf-8')+data[end:]).decode('utf-8')
    ast.parse(out)
    return out

if __name__ == '__main__':
    try:
        request=json.loads(sys.stdin.read())
        result=replacement(request['source'],request['qualname'],request['index'],request['operation'])
        print(json.dumps({'content':result},sort_keys=True))
    except (ValueError, TypeError, KeyError, UnicodeError, SyntaxError) as e:
        print('OPERATION_REFUSAL: '+str(e),file=sys.stderr)
        sys.exit(2)
