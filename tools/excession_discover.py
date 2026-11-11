#!/usr/bin/env python3
"""Conservative Git-native dependency declarations for Go, JS/TS and Rust.

This adapter never executes project code. It refuses dependency constructs that
cannot be resolved without toolchain semantics or external configuration.
Its output deliberately uses DECLARED_INPUTS_ONLY, not a completeness claim.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

class Unsupported(Exception):
    pass

MAX_FILES = 512
MAX_BYTES = 32 * 1024 * 1024

def git(repo, *args):
    p = subprocess.run(['git', '-C', str(repo), *args], capture_output=True)
    if p.returncode:
        raise Unsupported('Git refused '+ ' '.join(args[:2]) + ': '+p.stderr[:180].decode(errors='replace'))
    return p.stdout

class Tree:
    def __init__(self, repo, revision):
        self.repo = repo
        self.commit = git(repo, 'rev-parse', '--verify', f'{revision}^{{commit}}').decode().strip()
        if not re.fullmatch('[0-9a-f]{40}', self.commit):
            raise Unsupported('only SHA-1 Git repositories are supported')
        self.modes = {}
        for row in git(repo, 'ls-tree', '-rz', '--full-tree', self.commit).split(b'\0'):
            if not row:
                continue
            hdr, name = row.split(b'\t',1)
            mode, kind, _ = hdr.decode('ascii').split(' ')
            path = name.decode('utf-8')
            if path in self.modes:
                raise Unsupported('duplicate Git path')
            self.modes[path] = (mode,kind)
        self.cache = {}
    def read(self, path):
        if path not in self.modes or self.modes[path] not in [('100644','blob'),('100755','blob')]:
            raise Unsupported(f'unsupported missing or nonregular Git input: {path}')
        if path not in self.cache:
            data = git(self.repo,'show',f'{self.commit}:{path}')
            if len(data)> MAX_BYTES:
                raise Unsupported('source too large')
            try:
                self.cache[path] = data.decode('utf-8')
            except UnicodeDecodeError:
                raise Unsupported(f'non-UTF8 source: {path}')
        return self.cache[path]
    def exists(self, path):
        return self.modes.get(path) in [('100644','blob'),('100755','blob')]

def relative(path, spec):
    if not spec.startswith('.') or '\\' in spec or '\x00' in spec:
        raise Unsupported(f'nonrelative module import: {spec}')
    base = list(PurePosixPath(path).parts[:-1])
    for part in spec.split('/'):
        if part in ('','.'): continue
        if part=='..':
            if not base: raise Unsupported('module path escapes repository')
            base.pop()
        else: base.append(part)
    return '/'.join(base)

def js_imports(source, path):
    # Disallow dynamic loading and ambiguous runtime module resolution. Regex is
    # intentionally a bounded subset, not a JS/TS grammar completeness claim.
    if re.search(r'\b(?:import\s*\(|require\s*\(\s*[^\s\'\"]|eval\s*\(|module\.require\s*\()', source):
        raise Unsupported(f'dynamic import/eval/require in {path}')
    pats = [r'\bimport\s+(?:[\s\S]*?\s+from\s+)?[\'\"]([^\'\"]+)[\'\"]',
            r'\bexport\s+(?:\*|\{[^}]*\})\s+from\s+[\'\"]([^\'\"]+)[\'\"]',
            r'\brequire\s*\(\s*[\'\"]([^\'\"]+)[\'\"]\s*\)']
    found=set()
    for pat in pats:
        found.update(re.findall(pat,source))
    # Approximate coverage guard. All import/export/require tokens must be
    # covered as a parsed module reference, except type-only TS inline imports
    # which are refused rather than silently ignored.
    if re.search(r'\bimport\s*\(|\bimport\s+[\'\"]|\bexport\s+(?:\*|\{)|\brequire\s*\(',source) and not found:
        raise Unsupported(f'unresolved import syntax in {path}')
    return sorted(found)

def js_closure(tree, entry, lang):
    pending=[entry]
    visited=set()
    config=set()
    while pending:
        path=pending.pop()
        if path in visited:continue
        source=tree.read(path)
        visited.add(path)
        for spec in js_imports(source,path):
            if not spec.startswith('.'):
                # Builtins and package imports need separately verified runtime
                # resolution; no ungrounded dependency closure.
                raise Unsupported(f'external/bare JS module in {path}: {spec}')
            name=relative(path,spec)
            options=[name] if PurePosixPath(name).suffix else [name+x for x in ('.js','.mjs','.cjs','.ts','.tsx','.jsx','.json')]+[name+'/index'+x for x in ('.js','.ts','.tsx','.jsx')]
            candidates=[p for p in options if tree.exists(p)]
            if len(candidates)!=1:
                raise Unsupported(f'missing/ambiguous import {spec} from {path}')
            pending.append(candidates[0])
    # Config is always included when in the same package root; no guessing at
    # complex tsconfig aliases or npm package resolution.
    for p in ('package.json','package-lock.json','npm-shrinkwrap.json','yarn.lock','pnpm-lock.yaml', 'tsconfig.json','jsconfig.json'):
        if tree.exists(p): config.add(p)
    if lang=='typescript' and tree.exists('tsconfig.json'):
        conf=tree.read('tsconfig.json')
        if re.search(r'"(?:paths|baseUrl|extends|references)"\s*:',conf):
            raise Unsupported('TypeScript path alias, extends or project references require native resolver')
    return visited,config

def go_closure(tree, entry):
    if not entry.endswith('.go'): raise Unsupported('Go entry must be .go')
    if not tree.exists('go.mod'): raise Unsupported('Go module needs tracked go.mod')
    mod = tree.read('go.mod')
    m=re.search(r'(?m)^module\s+([^\s]+)',mod)
    if not m: raise Unsupported('go.mod missing module directive')
    module=m.group(1).strip('"')
    if re.search(r'(?m)^\s*(?:replace|exclude|toolchain)\b',mod):
        raise Unsupported('Go replace/exclude/toolchain semantics not resolved')
    pending=[str(PurePosixPath(entry).parent)]
    seen_dirs=set();selected=set()
    while pending:
        directory=pending.pop()
        if directory in seen_dirs:continue
        seen_dirs.add(directory)
        prefix='' if directory=='.' else directory+'/'
        sources=[p for p in tree.modes if p.startswith(prefix) and '/' not in p[len(prefix):] and p.endswith('.go') and tree.exists(p)]
        if not sources:raise Unsupported(f'no Go package sources at {directory}')
        for p in sources:
            s=tree.read(p)
            if re.search(r'(?m)^\s*//\s*(?:go:build|\+build)\b',s):
                raise Unsupported(f'build-constrained Go source: {p}')
            if re.search(r'\b(?:import\s*\(|import\s*\")',s):
                # import declarations are parsed but arbitrary comments or raw strings
                # are not accepted as proof of a complete Go AST.
                blocks=re.findall(r'(?m)^\s*import\s*\((.*?)\)',s,re.S)
                singles=re.findall(r'(?m)^\s*import\s+(?:[\w.]+\s+)?"([^"]+)"',s)
                imports=singles[:]
                for block in blocks: imports.extend(re.findall(r'"([^"]+)"',block))
                if not imports:raise Unsupported(f'unresolved import syntax in {p}')
                for imp in imports:
                    if imp == module or imp.startswith(module+'/'):
                        sub=imp[len(module):].lstrip('/')
                        pending.append(sub or '.')
                    elif '.' in imp.split('/')[0]:
                        raise Unsupported(f'external Go dependency {imp} in {p}')
            if re.search(r'\bimport\s+`',s):raise Unsupported(f'raw-string import in {p}')
            if re.search(r'(?m)^\s*//\s*go:(?:generate|embed|linkname|cgo_)',s):
                raise Unsupported(f'Go directive effects in {p}')
            selected.add(p)
    config={p for p in ('go.mod','go.sum','go.work','go.work.sum') if tree.exists(p)}
    if tree.exists('go.work'):raise Unsupported('multi-module Go workspace not resolved')
    return selected,config

def rust_closure(tree, entry):
    if not entry.endswith('.rs'):raise Unsupported('Rust entry must be .rs')
    if not tree.exists('Cargo.toml'):raise Unsupported('Rust crate needs tracked Cargo.toml')
    cargo=tree.read('Cargo.toml')
    if re.search(r'\[workspace\]|\bpath\s*=|\bbuild\s*=|\[target\.',cargo):
        raise Unsupported('Rust workspace/path/build/target configuration needs cargo metadata')
    if any(tree.exists(p) for p in ('build.rs','.cargo/config','.cargo/config.toml')):
        raise Unsupported('Rust build script or cargo config requires toolchain-aware resolver')
    # Conservative module discovery: include every tracked Rust source in this
    # crate to avoid underapproximation from mod/use, macros and cfg.
    files={p for p in tree.modes if p.endswith('.rs') and (p.startswith('src/') or p.startswith('tests/') or p.startswith('examples/') or p.startswith('benches/')) and tree.exists(p)}
    if entry not in files:raise Unsupported('Rust entry outside recognized crate source roots')
    if any(re.search(r'\binclude(?:_str|_bytes)?!\s*\(',tree.read(p)) for p in files):
        raise Unsupported('Rust include macro inputs require expansion')
    config={p for p in ('Cargo.toml','Cargo.lock','rust-toolchain','rust-toolchain.toml') if tree.exists(p)}
    return files,config

def discover(repo,rev,entry,lang):
    tree=Tree(repo,rev)
    if not tree.exists(entry):raise Unsupported('entry is not a regular tracked Git blob')
    if lang in ('javascript','typescript'):
        src,config=js_closure(tree,entry,lang)
    elif lang=='go':src,config=go_closure(tree,entry)
    elif lang=='rust':src,config=rust_closure(tree,entry)
    else:raise Unsupported('unsupported language adapter')
    inputs={p:'source' for p in src}
    for p in config:inputs[p]='configuration'
    if len(inputs)>MAX_FILES:raise Unsupported('too many discovered inputs')
    if sum(len(tree.read(p).encode('utf-8')) for p in inputs)>MAX_BYTES:raise Unsupported('discovered inputs exceed 32 MiB')
    return {'schema':'excession.input-declaration.v1','coverage':'DECLARED_INPUTS_ONLY',
        'language':lang,'subject':entry,
        'inputs':[{'path':p,'role':inputs[p]} for p in sorted(inputs)],
        'assumptions':{'dependency-discovery':'excession-static-conservative-v1'}}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('repo',type=Path);ap.add_argument('entry')
    ap.add_argument('--revision',default='HEAD')
    ap.add_argument('--language',choices=['go','javascript','typescript','rust'],required=True)
    args=ap.parse_args()
    try:
        print(json.dumps(discover(args.repo,args.revision,args.entry,args.language),sort_keys=True,separators=(',',':')))
    except (Unsupported,UnicodeError,ValueError) as e:
        print(f'UNSUPPORTED: {e}',file=sys.stderr);sys.exit(2)

if __name__=='__main__':main()
