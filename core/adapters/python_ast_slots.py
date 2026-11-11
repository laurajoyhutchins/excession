"""One-statement CPython AST slot rewrite for RelGit.

Only a single top-level statement in a named class method may be replaced.
The new statement must remain a single `write(argument)` call. The resulting
source is patched in-place so surrounding whitespace/comments stay byte-equal.
This is a syntactic precondition checker, NOT a semantic equivalence theorem.
"""
import ast
import json
import sys


def rewrite(source: str, qualname: str, index: int, old: str, new: str) -> str:
    if not isinstance(qualname, str) or len(qualname.split('.')) != 2:
        raise ValueError('required exact Class.method name')
    cls, meth = qualname.split('.')
    tree = ast.parse(source)
    matching_classes = [n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == cls]
    if len(matching_classes) != 1:
        raise ValueError('class is absent or ambiguous')
    methods = [n for n in matching_classes[0].body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and n.name == meth]
    if len(methods) != 1:
        raise ValueError('method is absent or ambiguous')
    method = methods[0]
    if type(index) is not int or not 0 <= index < len(method.body):
        raise ValueError('statement index outside method body')
    actual = method.body[index]

    def parse_statement(text):
        if not isinstance(text, str) or '\n' in text or '\r' in text or not text.strip() or text != text.strip():
            raise ValueError('replacement must be one line without outer whitespace')
        parsed = ast.parse(text)
        if len(parsed.body) != 1 or not isinstance(parsed.body[0], ast.Expr):
            raise ValueError('only one expression statement is supported')
        expr = parsed.body[0].value
        if not isinstance(expr, ast.Call) or not isinstance(expr.func, ast.Name) or expr.func.id != 'write' or len(expr.args) != 1 or expr.keywords:
            raise ValueError('the effect must remain one write(argument) call')
        if isinstance(expr.args[0], ast.Starred):
            raise ValueError('starred arguments unsupported')
        return parsed.body[0]

    before = parse_statement(old)
    after = parse_statement(new)
    if ast.dump(before, include_attributes=False) != ast.dump(actual, include_attributes=False):
        raise ValueError('AST precondition mismatch at declared statement')
    if ast.dump(after, include_attributes=False) == ast.dump(before, include_attributes=False):
        raise ValueError('no-op AST edit')
    encoded = source.encode('utf-8')
    lines = encoded.splitlines(keepends=True)
    if actual.lineno != actual.end_lineno:
        raise ValueError('only single-line statement slots supported')
    start = sum(len(x) for x in lines[:actual.lineno - 1]) + actual.col_offset
    end = sum(len(x) for x in lines[:actual.end_lineno - 1]) + actual.end_col_offset
    if encoded[start:end] != old.encode('utf-8'):
        raise ValueError('exact original statement bytes mismatch')
    replacement = encoded[:start] + new.encode('utf-8') + encoded[end:]
    output = replacement.decode('utf-8')
    ast.parse(output)
    return output


def main():
    value = json.loads(sys.stdin.read())
    result = rewrite(value['source'], value['qualname'], value['index'], value['old'], value['new'])
    sys.stdout.write(json.dumps({'content': result}, ensure_ascii=True, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, SyntaxError, KeyError, TypeError, UnicodeError) as e:
        print(f'AST_SLOT_REFUSAL: {e}', file=sys.stderr)
        sys.exit(2)
