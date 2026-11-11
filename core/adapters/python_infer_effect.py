"""CPython-native AST effect inference for one exact single-line expression or return statement.

Infers a left context rewrite which uniquely preserves a subtree and a right
subtree rewrite limited to that preserved subtree. No hand-authored operation
names or semantic classifications. The checked conclusion is byte-level source
confluence for these three exact sources, NOT behavioral equivalence.
"""
import ast
import copy
import hashlib
import json
import sys


def fingerprint(value):
    return hashlib.sha256(ast.dump(value, include_attributes=False).encode()).hexdigest()


def extract(source, qualname, index):
    if not isinstance(source, str) or not isinstance(qualname, str) or not 1 <= len(qualname.split('.')) <= 2:
        raise ValueError('invalid source/target')
    names = qualname.split('.')
    if any(not name.isidentifier() for name in names) or type(index) is not int:
        raise ValueError('invalid Python target')
    tree = ast.parse(source)
    if len(names) == 1:
        methods = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == names[0]]
    else:
        classes = [n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == names[0]]
        if len(classes) != 1:
            raise ValueError('ambiguous class')
        methods = [n for n in classes[0].body if isinstance(n, ast.FunctionDef) and n.name == names[1]]
    if len(methods) != 1 or not 0 <= index < len(methods[0].body):
        raise ValueError('ambiguous function or statement index')
    stmt = methods[0].body[index]
    if not isinstance(stmt, (ast.Expr, ast.Return)) or stmt.lineno != stmt.end_lineno:
        raise ValueError('only one-line expression/return statements supported')
    expr = stmt.value
    if not isinstance(expr, ast.expr):
        raise ValueError('target has no expression')
    if isinstance(stmt, ast.Expr) and (
        not isinstance(expr, ast.Call) or not isinstance(expr.func, ast.Name)
        or len(expr.args) != 1 or expr.keywords
    ):
        raise ValueError('expression statement must call one named function with one argument')
    raw = source.encode('utf-8')
    lines = raw.splitlines(keepends=True)
    offset = sum(len(line) for line in lines[:stmt.lineno - 1])
    start, end = offset + stmt.col_offset, offset + stmt.end_col_offset
    if start >= end or raw[start:end].decode('utf-8') != ast.get_source_segment(source, stmt):
        raise ValueError('statement source span not exact')
    return tree, stmt, expr, (start, end)


def subnodes(node, path=()):
    if isinstance(node, ast.expr):
        yield path, node
    for field, value in ast.iter_fields(node):
        if isinstance(value, ast.AST):
            yield from subnodes(value, path + ((field, None),))
        elif isinstance(value, list):
            for i, entry in enumerate(value):
                if isinstance(entry, ast.AST):
                    yield from subnodes(entry, path + ((field, i),))


def at(node, path):
    for field, i in path:
        node = getattr(node, field)
        if i is not None:
            node = node[i]
    return node


def replacement(root, path, value):
    root = copy.deepcopy(root)
    if not path:
        return copy.deepcopy(value)
    parent = at(root, path[:-1])
    field, i = path[-1]
    if i is None:
        setattr(parent, field, copy.deepcopy(value))
    else:
        getattr(parent, field)[i] = copy.deepcopy(value)
    return root


def span(source, node):
    if not hasattr(node, 'end_lineno') or node.lineno != node.end_lineno:
        raise ValueError('only one-line effect anchors supported')
    raw = source.encode('utf-8')
    lines = raw.splitlines(keepends=True)
    off = sum(len(line) for line in lines[:node.lineno - 1])
    start, end = off + node.col_offset, off + node.end_col_offset
    if start >= end:
        raise ValueError('invalid anchor span')
    return start, end


def infer(base, left, right, qualname, index):
    bt, bs, be, br = extract(base, qualname, index)
    lt, ls, le, lr = extract(left, qualname, index)
    rt, rs, re, rr = extract(right, qualname, index)
    if type(bs) is not type(ls) or type(bs) is not type(rs):
        raise ValueError('statement kind changed')
    if isinstance(bs, ast.Expr) and not (be.func.id == le.func.id == re.func.id):
        raise ValueError('call target changed')
    raw_base, raw_left, raw_right = [s.encode('utf-8') for s in (base, left, right)]
    if raw_base[:br[0]] != raw_left[:lr[0]] or raw_base[br[1]:] != raw_left[lr[1]:]:
        raise ValueError('left modifies undeclared source outside target statement')
    if raw_base[:br[0]] != raw_right[:rr[0]] or raw_base[br[1]:] != raw_right[rr[1]:]:
        raise ValueError('right modifies undeclared source outside target statement')
    if fingerprint(be) == fingerprint(le) or fingerprint(be) == fingerprint(re):
        raise ValueError('one branch has no AST effect')
    # Find the deepest unique anchor that the left edit preserves and the right
    # edit replaces. All other right-hand syntax must be EXACTLY unchanged.
    candidates = []
    left_nodes = list(subnodes(le))
    for path, old_anchor in subnodes(be):
        try:
            new_anchor = at(re, path)
        except (IndexError, AttributeError, TypeError):
            continue
        if not isinstance(new_anchor, ast.expr) or fingerprint(old_anchor) == fingerprint(new_anchor):
            continue
        if fingerprint(replacement(be, path, new_anchor)) != fingerprint(re):
            continue
        matches = [(p, n) for p, n in left_nodes if fingerprint(n) == fingerprint(old_anchor)]
        if len(matches) != 1:
            continue
        left_anchor = matches[0][1]
        try:
            old_span = span(base, old_anchor)
            new_span = span(right, new_anchor)
            left_span = span(left, left_anchor)
        except ValueError:
            continue
        old_bytes = raw_base[old_span[0]:old_span[1]]
        new_bytes = raw_right[new_span[0]:new_span[1]]
        left_bytes = raw_left[left_span[0]:left_span[1]]
        if old_bytes != left_bytes:
            continue
        # AST identity is insufficient; right must be an exact one-span source edit.
        predicted_right = raw_base[:old_span[0]] + new_bytes + raw_base[old_span[1]:]
        if predicted_right != raw_right:
            continue
        merged = raw_left[:left_span[0]] + new_bytes + raw_left[left_span[1]:]
        try:
            merged_text = merged.decode('utf-8')
            mt, ms, me, mr = extract(merged_text, qualname, index)
        except (UnicodeError, SyntaxError, ValueError):
            continue
        # Prove the merged statement is also exactly the left context with the
        # right new anchor. No source outside the target statement was touched.
        # Independently apply the left context rewrite to the right branch:
        # its outer statement must be the baseline context with only the hole changed.
        right_then_left = (raw_right[:rr[0]] + raw_left[lr[0]:left_span[0]]
                           + new_bytes + raw_left[left_span[1]:lr[1]] + raw_right[rr[1]:])
        if merged != right_then_left:
            continue
        candidates.append((len(path), path, old_anchor, new_anchor, merged_text))
    if not candidates:
        raise ValueError('no unique preserved subexpression / right-only edit')
    best_depth = max(item[0] for item in candidates)
    best = [x for x in candidates if x[0] == best_depth]
    if len(best) != 1:
        raise ValueError('multiple equally specific effect anchors; ambiguous composition')
    _, path, old, new, merged = best[0]
    # The effect calculus is a source-term statement: base context -> left
    # context, with exactly one shared hole; old hole -> right hole.
    return {
        'content': merged,
        'calculus': 'UNIQUE_SUBEXPRESSION_CONTEXT_V1',
        'statement_index': index,
        'anchor_before_sha256': fingerprint(old),
        'anchor_after_sha256': fingerprint(new),
        'base_statement_sha256': fingerprint(bs),
        'left_statement_sha256': fingerprint(ls),
        'right_statement_sha256': fingerprint(rs),
        'merged_statement_sha256': fingerprint(extract(merged, qualname, index)[1]),
        'anchor_path': [[f, i] for f, i in path],
        'source_confluence': True,
    }


if __name__ == '__main__':
    try:
        payload = json.loads(sys.stdin.read())
        outcome = infer(payload['base'], payload['left'], payload['right'], payload['qualname'], payload['index'])
        print(json.dumps(outcome, sort_keys=True, ensure_ascii=True))
    except (ValueError, SyntaxError, KeyError, TypeError, UnicodeError) as exc:
        print('INFER_EFFECT_REFUSAL: ' + str(exc), file=sys.stderr)
        sys.exit(2)
