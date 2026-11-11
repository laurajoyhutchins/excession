"""Narrow language-native resolver behind Excession's Rust input authority.

Input: <staged-root> <entry> <language> <trusted TypeScript helper> <import-roots>.
Output: one JSON object with source/configuration paths and resolver assumptions.
Python also returns bounded static import-site observations.

This module does not read Git history, construct evidence contracts, validate
hashes, or stage files. Those responsibilities belong to the Rust core.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath


class Unsupported(ValueError):
    pass


def run(argv, cwd=None, env=None):
    try:
        p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=45)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Unsupported(f"native resolver unavailable or timed out: {exc}") from exc
    if len(p.stdout) > 16 * 1024 * 1024 or len(p.stderr) > 16 * 1024 * 1024:
        raise Unsupported("native resolver exceeded 16 MiB output limit")
    if p.returncode:
        detail = (p.stderr or p.stdout)[:700].decode(errors="replace").strip()
        raise Unsupported(f"native resolver exited {p.returncode}: {detail or 'no diagnostics'}")
    return p.stdout


def find(name):
    executable = shutil.which(name)
    if executable is None:
        raise Unsupported(f"native adapter requires {name}")
    # Keep invocation path and argv[0] intact: rustc is often a rustup shim.
    return Path(executable).absolute()


def relative(root, filename):
    try:
        rel = Path(filename).resolve(strict=True).relative_to(root.resolve()).as_posix()
    except (ValueError, OSError) as exc:
        raise Unsupported(f"dependency outside pinned Git snapshot: {filename}") from exc
    if rel == "." or any(x in ("", ".", "..") for x in rel.split("/")):
        raise Unsupported("invalid resolved dependency path")
    return rel


def go(root, entry):
    if not (root / "go.mod").is_file() or (root / "go.work").exists():
        raise Unsupported("Go requires tracked go.mod without go.work")
    dirname = PurePosixPath(entry).parent.as_posix()
    env = {"PATH": os.environ.get("PATH", "/usr/local/go/bin:/usr/bin:/bin"),
           "HOME": str(root.parent), "GOCACHE": str(root.parent / "go-cache"),
           "GOMODCACHE": str(root.parent / "go-mod-cache"),
           "GOPROXY": "off", "GOSUMDB": "off", "GOTOOLCHAIN": "local",
           "GOWORK": "off", "CGO_ENABLED": "0", "GOFLAGS": "-mod=readonly",
           "LC_ALL": "C", "TZ": "UTC"}
    output = run([str(find("go")), "list", "-deps", "-test", "-json",
                  "./" if dirname == "." else "./" + dirname], cwd=root, env=env)
    decoder = json.JSONDecoder()
    text = output.decode()
    offset = 0
    selected, package_dirs = set(), set()
    while offset < len(text):
        while offset < len(text) and text[offset].isspace():
            offset += 1
        if offset == len(text):
            break
        item, offset = decoder.raw_decode(text, offset)
        if item.get("Error") or item.get("DepsErrors"):
            raise Unsupported("Go native resolver reported dependency errors")
        if item.get("Standard") or str(item.get("ImportPath", "")).endswith(".test"):
            continue
        directory = item.get("Dir")
        if not directory:
            raise Unsupported("native Go package has no directory")
        package_dirs.add("." if Path(directory) == root else relative(root, directory))
        if any(item.get(key) for key in ("CgoFiles", "SwigFiles", "SwigCXXFiles")):
            raise Unsupported("Go cgo/SWIG needs separate toolchain effects")
        for key in ("GoFiles", "TestGoFiles", "XTestGoFiles", "SFiles",
                    "CFiles", "CXXFiles", "HFiles", "EmbedFiles",
                    "TestEmbedFiles", "XTestEmbedFiles", "SysoFiles"):
            for filename in item.get(key, []):
                selected.add(relative(root, Path(directory) / filename))
    # Include build-tag alternatives in any reached local package.
    for filename in root.rglob("*.go"):
        if filename.is_file() and filename.relative_to(root).parent.as_posix() in package_dirs:
            selected.add(filename.relative_to(root).as_posix())
    if entry not in selected:
        raise Unsupported("entry not reached by go list")
    return selected, {p for p in ("go.mod", "go.sum") if (root / p).is_file()}, {
        "go-flags": "CGO_ENABLED=0,GOWORK=off,GOPROXY=off"}


def typescript(root, entry, helper):
    # Resolve only for locating TypeScript's adjacent library, not execution.
    tsc = find("tsc").resolve()
    ts_api = tsc.parent.parent / "lib" / "typescript.js"
    if not ts_api.is_file():
        raise Unsupported("TypeScript compiler API missing beside tsc")
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(root.parent),
           "NODE_OPTIONS": "", "NODE_PATH": "", "LC_ALL": "C", "TZ": "UTC"}
    result = json.loads(run([str(find("node")), str(helper), str(ts_api),
                             str(root), entry], cwd=root, env=env))
    import hashlib
    sha = hashlib.sha256(ts_api.read_bytes()).hexdigest()
    return set(result["sources"]), set(result["configuration"]), {
        "typescript-api-sha256": sha,
        "typescript-options": result["options_sha256"]}


def rust(root, entry):
    config = root / "Cargo.toml"
    if not config.is_file():
        raise Unsupported("Rust requires tracked Cargo.toml")
    manifest = tomllib.loads(config.read_text())
    if "workspace" in manifest or any(manifest.get(key) for key in (
            "dependencies", "dev-dependencies", "build-dependencies", "patch", "replace")):
        raise Unsupported("Rust workspace/external dependencies need Cargo-aware adapter")
    package = manifest.get("package", {})
    if not isinstance(package, dict) or package.get("build") is not None or (root / "build.rs").exists():
        raise Unsupported("Rust build script is unsupported")
    if (root / ".cargo/config.toml").exists():
        raise Unsupported("Rust Cargo configuration needs additional interpretation")
    edition = package.get("edition", "2015")
    if edition not in ("2015", "2018", "2021", "2024") or entry not in ("src/lib.rs", "src/main.rs"):
        raise Unsupported("unsupported Rust crate root/edition")
    for filename in root.rglob("*.rs"):
        if re.search(r"\b(?:env|option_env|concat|include)!\s*\(", filename.read_text(errors="replace")):
            raise Unsupported("Rust compile-time environment or dynamic include unsupported")
    depfile = root.parent / "rust-dependencies.d"
    selected = os.environ.get("EXCESSION_NATIVE_RUSTC")
    if not selected:
        raise Unsupported("Rust authority did not pin a compiler executable")
    rustc = Path(selected)
    if not rustc.is_absolute() or not rustc.is_file():
        raise Unsupported("Rust authority supplied an invalid compiler path")
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(root.parent),
           "LC_ALL": "C", "TZ": "UTC", "CARGO_MANIFEST_DIR": str(root),
           "CARGO_PKG_NAME": str(package.get("name", "probe"))}
    run([str(rustc), "--crate-name", "excession_native_probe", "--crate-type",
         "lib" if entry == "src/lib.rs" else "bin", "--edition", edition,
         "--emit", f"dep-info={depfile}", str(root / entry)], cwd=root, env=env)
    first = depfile.read_text().splitlines()[0]
    if ": " not in first or any(x in first for x in ("\\ ", "\\#", "\\:")):
        raise Unsupported("unsupported rustc dep-info encoding")
    selected = {relative(root, p) for p in first.split(": ", 1)[1].split()}
    if entry not in selected:
        raise Unsupported("rustc dep-info omitted entry")
    return selected, {p for p in ("Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml")
                      if (root / p).is_file()}, {
        "rustc-edition": edition, "rustc-mode": "dep-info-no-externs"}


def python_roots(root, raw):
    """Interpret an explicit ordered import path, never ambient PYTHONPATH."""
    if not raw:
        raise Unsupported("Python requires explicit --import-roots")
    names = raw.split(",")
    if len(names) > 16 or len(set(names)) != len(names):
        raise Unsupported("Python import roots must be unique (up to 16)")
    paths = []
    for name in names:
        pieces = name.split("/")
        if name != "." and (name.startswith("/") or
                any(not piece or piece in (".", "..") for piece in pieces)):
            raise Unsupported("invalid Python import root")
        directory = root if name == "." else root.joinpath(*pieces)
        if not directory.is_dir():
            raise Unsupported(f"Python import root is not a tracked directory: {name}")
        paths.append(directory)
    for left in paths:
        for right in paths:
            if left != right and (left in right.parents or right in left.parents):
                raise Unsupported("overlapping Python import roots are ambiguous")
    return paths


class PythonFinder:
    """A bounded analogue of CPython's source-file and namespace lookup.

    Uses only the staged .py files. Never calls import_module, find_spec, a
    project loader, setup.py or a repository-defined import hook.
    """
    def __init__(self, root, search):
        self.root = root
        self.search = search

    def resolve(self, dotted):
        if not dotted or any(not p.isidentifier() for p in dotted.split(".")):
            return None
        search = self.search
        sources = []
        for index, part in enumerate(dotted.split(".")):
            namespaces = []
            chosen = None
            for directory in search:
                package = directory / part
                init = package / "__init__.py"
                module = directory / (part + ".py")
                if init.is_file():
                    chosen = (init, [package], False)
                    break
                if module.is_file():
                    chosen = (module, [], True)
                    break
                if package.is_dir():
                    namespaces.append(package)
            if chosen is not None:
                file, search, terminal = chosen
                sources.append(relative(self.root, file))
                if terminal and index < len(dotted.split(".")) - 1:
                    return None
            elif namespaces:
                search = namespaces
            else:
                return None
        return sources

    def name_for_entry(self, entry):
        path = self.root / entry
        candidates = []
        for directory in self.search:
            if path.is_relative_to(directory):
                relative_path = path.relative_to(directory)
                if relative_path.suffix != ".py":
                    continue
                pieces = list(relative_path.with_suffix("").parts)
                if pieces[-1] == "__init__":
                    pieces.pop()
                if pieces:
                    candidates.append(".".join(pieces))
        if len(candidates) != 1:
            raise Unsupported("entry does not have a unique configured import name")
        name = candidates[0]
        if self.resolve(name) is None or entry not in self.resolve(name):
            raise Unsupported("entry is shadowed or not importable under configured roots")
        return name


def python(root, entry, import_roots):
    """Build a conservative historical source-input declaration using AST.

    RESOLVED and CANDIDATE are static source observations, never assertions
    of unconditional runtime imports. Unknown environments are explicit.
    """
    import ast
    import importlib.util

    roots = python_roots(root, import_roots)
    finder = PythonFinder(root, roots)
    entry_name = finder.name_for_entry(entry)
    sources = set(finder.resolve(entry_name) or [])
    sources.add(entry)
    pending = [entry]
    seen = set()
    observations = []
    config = {p for p in ("pyproject.toml", "setup.cfg", "setup.py")
              if (root / p).is_file()}
    if "pyproject.toml" in config:
        try:
            project = tomllib.loads((root / "pyproject.toml").read_text())
            setuptools = project.get("tool", {}).get("setuptools", {})
            package_dir = setuptools.get("package-dir", {})
            if isinstance(package_dir, dict):
                configured = package_dir.get("")
                if configured is not None and configured not in import_roots.split(","):
                    raise Unsupported("explicit Python import roots conflict with setuptools package-dir")
            find_where = setuptools.get("packages", {})
            if isinstance(find_where, dict):
                find_where = find_where.get("find", {})
                if isinstance(find_where, dict):
                    for where in find_where.get("where", []):
                        if where not in import_roots.split(","):
                            raise Unsupported("explicit Python import roots conflict with setuptools find.where")
        except (tomllib.TOMLDecodeError, TypeError, AttributeError) as exc:
            raise Unsupported(f"cannot interpret tracked Python metadata: {exc}") from exc

    def observe(source, line, specifier, status, targets=(), reason=""):
        if len(observations) >= 2048:
            raise Unsupported("Python import site budget exceeded (2048)")
        observations.append({
            "source": source, "line": line, "specifier": specifier,
            "status": status, "targets": sorted(set(targets)),
            "reason": reason,
        })

    def target(source, line, specifier, status, reason=""):
        matches = finder.resolve(specifier)
        if matches is None:
            observe(source, line, specifier, "UNRESOLVED", reason="not in pinned import roots")
            return []
        observe(source, line, specifier, status, matches, reason)
        sources.update(matches)
        pending.extend(p for p in matches if p not in seen)
        return matches

    def relative_name(module, package_file, level, tail):
        parent = module if package_file else module.rpartition(".")[0]
        if not parent:
            return None
        try:
            return importlib.util.resolve_name("." * level + (tail or ""), parent)
        except ImportError:
            return None

    conditional_nodes = (
        ast.If, ast.For, ast.AsyncFor, ast.While, ast.Try,
        ast.With, ast.AsyncWith, ast.FunctionDef, ast.AsyncFunctionDef,
        ast.ClassDef, ast.Lambda, ast.Match,
    )

    while pending:
        path = pending.pop()
        if path in seen:
            continue
        seen.add(path)
        try:
            code = (root / path).read_text()
            tree = ast.parse(code, filename=path)
        except (UnicodeDecodeError, SyntaxError) as exc:
            observe(path, 1, "<parse>", "UNSUPPORTED", reason=str(exc)[:180])
            continue

        # A selected file may have several possible import names on sys.path.
        # Refuse instead of silently turning its relative imports into guesses.
        module = finder.name_for_entry(path)
        package_file = path.endswith("/__init__.py")

        def visit(node, guarded=False):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    target(path, node.lineno, alias.name,
                           "CANDIDATE" if guarded else "RESOLVED",
                           "conditional or deferred" if guarded else "")
                return
            if isinstance(node, ast.ImportFrom):
                base = (relative_name(module, package_file, node.level, node.module)
                        if node.level else node.module)
                if not base:
                    observe(path, node.lineno, "." * node.level + (node.module or ""),
                            "UNSUPPORTED", reason="invalid relative import context")
                    return
                main = finder.resolve(base)
                if main is None:
                    observe(path, node.lineno, base, "UNRESOLVED",
                            reason="base not in pinned import roots")
                    return
                sources.update(main)
                pending.extend(p for p in main if p not in seen)
                for alias in node.names:
                    if alias.name == "*":
                        observe(path, node.lineno, base + ".*", "UNSUPPORTED",
                                main, "wildcard export value cannot be known statically")
                        continue
                    child = finder.resolve(base + "." + alias.name)
                    paths = main + (child or [])
                    sources.update(paths)
                    pending.extend(p for p in paths if p not in seen)
                    observe(path, node.lineno, base + "." + alias.name,
                            "CANDIDATE", paths,
                            "from-import may refer to an attribute, not a submodule")
                return
            if isinstance(node, ast.Call):
                fn = node.func
                symbol = (fn.id if isinstance(fn, ast.Name) else
                          fn.attr if isinstance(fn, ast.Attribute) else "")
                if symbol in ("__import__", "import_module"):
                    arg = node.args[0] if node.args else None
                    if isinstance(arg, ast.Constant) and isinstance(arg.value, str):
                        name = arg.value
                        if name.startswith("."):
                            package = node.args[1] if len(node.args) > 1 else None
                            if (not isinstance(package, ast.Constant) or
                                    not isinstance(package.value, str)):
                                observe(path, node.lineno, name, "UNSUPPORTED",
                                        reason="relative dynamic import without literal package")
                                return
                            try:
                                name = importlib.util.resolve_name(name, package.value)
                            except ImportError:
                                observe(path, node.lineno, name, "UNSUPPORTED",
                                        reason="invalid dynamic relative import")
                                return
                        target(path, node.lineno, name, "CANDIDATE",
                               "literal dynamic import is not guaranteed to execute")
                    else:
                        observe(path, node.lineno, symbol, "UNSUPPORTED",
                                reason="computed dynamic module name")
                elif symbol in ("exec", "eval"):
                    observe(path, node.lineno, symbol, "UNSUPPORTED",
                            reason="dynamic code execution may change imports")
                if isinstance(fn, ast.Attribute):
                    chain = ast.unparse(fn)
                    if (chain.startswith(("sys.path.", "sys.meta_path.",
                                          "sys.path_hooks.", "sys.modules.")) or
                            chain.startswith("pkgutil.extend_path")):
                        observe(path, node.lineno, chain, "UNSUPPORTED",
                                reason="runtime import machinery modification")
            for child in ast.iter_child_nodes(node):
                visit(child, guarded or isinstance(node, conditional_nodes))

        visit(tree)
        if len(seen) > 512 or len(sources) > 512:
            raise Unsupported("Python native dependency closure exceeds 512 source files")

    assumptions = {
        "python-import-roots": import_roots,
        "python-import-policy": "AST-and-pinned-Python-source-only",
        "python-runtime-coverage": "NOT_ESTABLISHED",
        "python-observation-count": str(len(observations)),
        "python-unsupported-count": str(sum(x["status"] == "UNSUPPORTED" for x in observations)),
        "python-unresolved-count": str(sum(x["status"] == "UNRESOLVED" for x in observations)),
        "python-metadata-policy": "READ_NOT_EXECUTED",
    }
    return sources, config, assumptions, sorted(
        observations, key=lambda x: (x["source"], x["line"], x["specifier"], x["status"])
    )


def main():
    if len(sys.argv) != 6:
        raise Unsupported("expected staged root, entry, language, helper, import roots")
    root, entry, lang, helper, roots = (Path(sys.argv[1]).resolve(), sys.argv[2],
                                       sys.argv[3], Path(sys.argv[4]), sys.argv[5])
    if lang == "python":
        src, config, assumptions, observations = python(root, entry, roots)
    elif lang == "go":
        src, config, assumptions = go(root, entry)
    elif lang in ("javascript", "typescript"):
        src, config, assumptions = typescript(root, entry, helper)
    elif lang == "rust":
        src, config, assumptions = rust(root, entry)
    else:
        raise Unsupported("unsupported language")
    result = {"sources": sorted(src), "configuration": sorted(config),
              "assumptions": assumptions}
    if lang == "python":
        result["observations"] = observations
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    try:
        main()
    except (Unsupported, ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print("UNSUPPORTED: " + str(error), file=sys.stderr)
        sys.exit(2)
