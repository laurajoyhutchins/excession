"""CPython-native static import semantics for Excession executable slices.

Input: JSON on stdin containing pinned Git source text and tracked paths.
Output: JSON with a sorted declared source closure. No Git, hashing, SQLite,
filesystem construction, execution, transport, or evidence decisions here.
The analysis remains bounded and *does not* prove runtime effect completeness.
"""
import ast
import json
import sys


class Unsupported(Exception):
    pass


def module_path(name, paths):
    stem = name.replace('.', '/')
    if not stem or any(not part.isidentifier() for part in name.split('.')):
        return None
    for path in (stem + '.py', stem + '/__init__.py'):
        if path in paths:
            return path
    return None


def parent_inits(path, paths):
    parts = path.split('/')[:-1]
    result = []
    for i in range(1, len(parts) + 1):
        init = '/'.join(parts[:i]) + '/__init__.py'
        if init not in paths:
            raise Unsupported(f'namespace package unsupported (missing {init})')
        result.append(init)
    return result


def parse(source, path):
    try:
        return ast.parse(source.removeprefix('\ufeff'), filename=path)
    except (SyntaxError, UnicodeError, ValueError) as error:
        raise Unsupported(f'cannot parse Python source {path}: {error}') from error


def exported(tree, member):
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) and node.name == member:
            return True
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            if any(isinstance(t, ast.Name) and t.id == member for t in targets):
                return True
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            if any((a.asname or a.name.split('.')[0]) == member for a in node.names):
                return True
    return False


def imports(path, sources, paths, parse_for):
    tree = parse_for(path)
    mod = path.removesuffix('.py').replace('/', '.')
    is_package = mod.endswith('.__init__')
    if is_package:
        mod = mod.removesuffix('.__init__')
    package = mod if is_package else mod.rpartition('.')[0]
    result = set()

    def include(name, relative=False):
        resolved = module_path(name, paths)
        if resolved is not None:
            result.add(resolved)
        elif (relative or module_path(name.split('.')[0], paths)
              or name.split('.')[0] not in sys.stdlib_module_names):
            raise Unsupported(f'unresolved or external import {name!r} in {path}')

    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            fn = node.func
            dynamic = (isinstance(fn, ast.Name) and fn.id in {'__import__', 'eval', 'exec'}) or (
                isinstance(fn, ast.Attribute) and fn.attr in {
                    'import_module', 'spec_from_file_location', 'run_path', 'run_module'
                })
            if dynamic:
                raise Unsupported(f'dynamic code loading unsupported in {path}:{node.lineno}')
        if isinstance(node, ast.Import):
            for alias in node.names:
                include(alias.name)
        if isinstance(node, ast.ImportFrom):
            if node.level:
                groups = package.split('.') if package else []
                if node.level > len(groups):
                    raise Unsupported(f'relative import escapes root in {path}')
                prefix = groups[:len(groups) - node.level + 1]
                base = '.'.join(prefix + ([node.module] if node.module else []))
            else:
                base = node.module or ''
            if not base:
                raise Unsupported(f'unresolved import in {path}')
            include(base, relative=bool(node.level))
            for alias in node.names:
                if alias.name == '*':
                    raise Unsupported(f'star import unsupported in {path}')
                maybe = f'{base}.{alias.name}'
                if module_path(maybe, paths):
                    include(maybe, relative=bool(node.level))
                elif module_path(base, paths):
                    base_file = module_path(base, paths)
                    if not exported(parse_for(base_file), alias.name):
                        raise Unsupported(f'unresolved imported symbol {base}.{alias.name} in {path}')
    return result


def plan(request):
    paths = set(request['paths'])
    sources = request['sources']
    entry = request['entry']
    assets = request['assets']
    if not entry.endswith('.py') or entry not in paths:
        raise Unsupported('entry must be a tracked Python source')
    pending = {entry, *assets, *parent_inits(entry, paths)}
    done = set()
    parsed = {}

    def parse_for(path):
        if path not in sources:
            raise Unsupported(f'unsupported source mode or encoding: {path}')
        if path not in parsed:
            parsed[path] = parse(sources[path], path)
        return parsed[path]

    while pending:
        path = min(pending)
        pending.remove(path)
        if path in done:
            continue
        if path not in paths:
            raise Unsupported(f'untracked dependency: {path}')
        if len(done) >= 512:
            raise Unsupported('slice exceeds file budget')
        done.add(path)
        if path.endswith('.py'):
            for dependency in imports(path, sources, paths, parse_for):
                pending.add(dependency)
                pending.update(parent_inits(dependency, paths))
    return {'paths': sorted(done)}


if __name__ == '__main__':
    try:
        incoming = json.load(sys.stdin)
        print(json.dumps(plan(incoming), sort_keys=True, separators=(',', ':')))
    except (Unsupported, ValueError, KeyError, TypeError) as error:
        print('UNSUPPORTED: ' + str(error), file=sys.stderr)
        sys.exit(2)
