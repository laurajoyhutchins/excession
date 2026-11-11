"""Clang-backed C/C++ translation-unit observations from a compilation database.

This adapter REUSES clang's own JSON AST and dependency preprocessor. It does
not implement C grammar. The caller must explicitly consent to trusted-local
Clang execution. Nothing from compile_commands.json is executed via a shell.
"""
import bisect
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys

MAX_DB_BYTES = 32 * 1024 * 1024
MAX_AST_BYTES = 192 * 1024 * 1024
MAX_DEP_BYTES = 2 * 1024 * 1024
MAX_HEADER_BYTES = 32 * 1024 * 1024
MAX_HEADERS = 2048
MAX_OBSERVATIONS = 5000
TIMEOUT_SECONDS = 30
DECLS = {"FunctionDecl", "VarDecl", "TypedefDecl", "RecordDecl",
         "EnumDecl", "EnumConstantDecl", "NamespaceDecl", "CXXRecordDecl",
         "CXXMethodDecl", "CXXConstructorDecl", "CXXDestructorDecl",
         "CXXConversionDecl", "TypeAliasDecl", "ClassTemplateDecl",
         "FunctionTemplateDecl"}
FUNCTIONS = {"FunctionDecl", "CXXMethodDecl", "CXXConstructorDecl",
             "CXXDestructorDecl", "CXXConversionDecl"}
SCOPES = {"NamespaceDecl", "CXXRecordDecl", "RecordDecl", "ClassTemplateDecl"}
CALLS = {"CallExpr", "CXXMemberCallExpr", "CXXOperatorCallExpr"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def digest_file(path, limit):
    with open(path, "rb") as inp:
        data = inp.read(limit + 1)
    if len(data) > limit:
        raise ValueError("input exceeded byte budget: " + str(path))
    return digest(data), data


def invoke(argv, cwd, max_bytes, timeout=TIMEOUT_SECONDS):
    if not argv:
        raise ValueError("empty compiler invocation")
    try:
        proc = subprocess.run(argv, cwd=str(cwd), stdin=subprocess.DEVNULL,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              timeout=timeout, check=False)
    except subprocess.TimeoutExpired as error:
        raise ValueError("clang invocation timed out") from error
    if len(proc.stdout) > max_bytes or len(proc.stderr) > 65536:
        raise ValueError("clang output exceeded byte budget")
    if proc.returncode:
        raise ValueError("clang exited %d: %s" %
                         (proc.returncode, proc.stderr.decode("utf-8", "replace")[-5000:]))
    return proc.stdout, proc.stderr.decode("utf-8", "replace")[-8192:]


def load_command(path, source):
    db_digest, raw = digest_file(path, MAX_DB_BYTES)
    try:
        entries = json.loads(raw)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid compilation database") from error
    if not isinstance(entries, list) or len(entries) > 100000:
        raise ValueError("compilation database must be a bounded array")
    matches = []
    for item in entries:
        if not isinstance(item, dict) or not isinstance(item.get("directory"), str) or not isinstance(item.get("file"), str):
            raise ValueError("invalid compilation database entry")
        # Unrelated entries may have absent directories in sparse checkouts.
        directory = Path(item["directory"]).resolve()
        file_path = (directory / item["file"]).resolve()
        if file_path == source:
            if not directory.is_dir() or not source.is_file():
                raise ValueError("matching compilation directory or source is missing")
            matches.append((item, directory))
    if len(matches) != 1:
        raise ValueError("expected exactly one compile command for %s, found %d" %
                         (source, len(matches)))
    entry, directory = matches[0]
    if "arguments" in entry:
        argv = entry["arguments"]
        if not isinstance(argv, list) or not argv or not all(isinstance(v, str) for v in argv):
            raise ValueError("invalid compilation database arguments")
    elif isinstance(entry.get("command"), str):
        argv = shlex.split(entry["command"], posix=True)
    else:
        raise ValueError("entry needs arguments or command")
    if not argv or not argv[0] or len(argv) > 1024 or any(len(x) > 8192 for x in argv):
        raise ValueError("unbounded compilation command")
    # Original executable is provenance, not an executable. Cross-compilers
    # and arbitrary GCC target triples are supported as descriptive metadata.
    if any(chr(0) in arg for arg in argv):
        raise ValueError("NUL in compilation command")
    return db_digest, entry, directory, argv


def analysis_flags(argv, source, directory):
    """Keep compilation flags; remove only output-producing flags.

    Reject plugin flags, response files and multi-source commands. This is
    a scoped trusted-local interface, not an untrusted compiler sandbox.
    """
    result = []
    seen_source = 0
    i = 1
    paired_drop = {"-o", "-MF", "-MT", "-MQ", "-MJ", "--serialize-diagnostics"}
    single_drop = {"-c", "-MD", "-MMD", "-MP", "-M", "-MM", "-S", "-E"}
    forbidden = ("-fplugin", "-fmodules", "-save-temps", "-ftime-trace",
                 "-fprofile-generate", "-fprofile-instr-generate",
                 "-fprofile-use", "-fprofile-instr-use", "-gsplit-dwarf",
                 "-fsyntax-only", "-Xclang", "-load", "-wrapper",
                 "--serialize-diagnostics=", "-fmodules-cache-path=")
    while i < len(argv):
        arg = argv[i]
        if arg.startswith("@") or arg in (";", "|", "&&", "||", ">", "<"):
            raise ValueError("response files and shell syntax are unsupported")
        if arg.startswith(forbidden):
            raise ValueError("compiler option may execute extensions or write files: " + arg)
        if arg in paired_drop:
            if i + 1 >= len(argv):
                raise ValueError("missing compiler option operand: " + arg)
            i += 2
            continue
        if arg in single_drop or arg.startswith(("-o", "-MF", "-MT", "-MQ", "-MJ")):
            i += 1
            continue
        # Linux Kbuild often embeds depfile writes in driver-prefixed options.
        if arg.startswith(("-Wp,-MD,", "-Wp,-MMD,", "-Wp,-MF,",
                           "-Wp,-MT,", "-Wp,-MQ,")):
            i += 1
            continue
        if arg.startswith("-"):
            result.append(arg)
        else:
            candidate = (directory / arg).resolve()
            if candidate == source:
                seen_source += 1
                result.append(str(source))
            elif result and result[-1] in ("-I", "-isystem", "-iquote", "-include",
                                           "-imacros", "-isysroot", "-x", "-D", "-U",
                                           "-target", "--target", "-idirafter",
                                           "-iframework", "-Xpreprocessor"):
                result.append(arg)
            elif arg.endswith((".c", ".cc", ".cpp", ".S", ".s")):
                raise ValueError("multiple translation units unsupported: " + arg)
            else:
                result.append(arg)
        i += 1
    if seen_source != 1:
        raise ValueError("compile command must name the selected C file once")
    return result


def make_dependencies(text):
    # Clang's -M -MT excession emits make-style escaped dependency paths.
    text = re.sub(r"\\\r?\n", " ", text)
    if not text.startswith("excession:"):
        raise ValueError("invalid clang -M dependency output")
    names = shlex.split(text[len("excession:"):], posix=True)
    if not names or len(names) > MAX_HEADERS:
        raise ValueError("missing or too many clang dependencies")
    return sorted(set(names))


def header_observations(names, directory, repo):
    records = []
    total = 0
    for name in names:
        path = (directory / name).resolve(strict=True)
        if not path.is_file():
            raise ValueError("clang dependency is not a regular file: " + str(path))
        size = path.stat().st_size
        if size > 4 * 1024 * 1024:
            raise ValueError("oversized clang dependency: " + str(path))
        total += size
        if total > MAX_HEADER_BYTES:
            raise ValueError("clang dependency byte budget exceeded")
        sha, _ = digest_file(path, 4 * 1024 * 1024)
        relative = None
        try:
            relative = path.relative_to(repo).as_posix()
        except ValueError:
            pass
        records.append({"path": str(path), "repo_path": relative,
                        "sha256": sha, "bytes": size})
    return sorted(records, key=lambda r: r["path"])


def extract_symbols(ast, source, source_bytes):
    """Extract main-TU declarations and direct calls from Clang's parsed AST.

    Supports C and nested C++ namespaces/classes. Only path, name, type and
    location are retained; AST pointer IDs are unstable. Call-target names
    are not unique identities across overloads or translation units.
    """
    line_offsets = [i for i, byte in enumerate(source_bytes) if byte == 10]
    source_name = str(source)
    symbols, calls = [], []
    processed = 0

    def positions(node):
        loc = node.get("loc") or {}
        begin = (node.get("range") or {}).get("begin") or {}
        return [x for x in (loc, begin) if isinstance(x, dict)]

    def explicit_file(node):
        for pos in positions(node):
            for candidate in (pos, pos.get("expansionLoc"), pos.get("spellingLoc")):
                if isinstance(candidate, dict) and isinstance(candidate.get("file"), str):
                    return str(Path(candidate["file"]).resolve())
        return None

    def line_of(node):
        for pos in positions(node):
            for candidate in (pos, pos.get("expansionLoc"), pos.get("spellingLoc")):
                if not isinstance(candidate, dict):
                    continue
                line = candidate.get("line")
                if isinstance(line, int) and line > 0:
                    return line
                offset = candidate.get("offset")
                if isinstance(offset, int) and 0 <= offset <= len(source_bytes):
                    return bisect.bisect_left(line_offsets, offset) + 1
        return 0

    def callee_of(expr):
        pending = [expr]
        while pending:
            node = pending.pop()
            if not isinstance(node, dict):
                continue
            ref = node.get("referencedDecl") or {}
            if (isinstance(ref, dict) and ref.get("kind") in FUNCTIONS
                    and isinstance(ref.get("name"), str)):
                return ref["name"]
            if node.get("kind") == "MemberExpr" and isinstance(node.get("name"), str):
                return node["name"]
            pending.extend(reversed(node.get("inner") or []))
        return None

    # Clang omits explicit files for adjacent top-level source declarations,
    # including a C++ main() after a namespace. Carry the last explicit
    # top-level file across siblings, but do not inherit that file across an
    # unrelated subtree when Clang explicitly identifies a header.
    top = ast.get("inner") or []
    cursor = None
    top_frames = []
    for child in top:
        cursor = explicit_file(child) or cursor
        top_frames.append((child, cursor, (), None))
    stack = list(reversed(top_frames))
    while stack:
        node, inherited_file, scope, caller = stack.pop()
        if not isinstance(node, dict):
            continue
        processed += 1
        if processed > 800_000:
            raise ValueError("Clang AST node budget exceeded")
        here = explicit_file(node) or inherited_file
        kind = node.get("kind")
        name = node.get("name")
        eligible = here == source_name and not node.get("isImplicit")
        qualified = "::".join((*scope, name)) if isinstance(name, str) else ""
        is_function = kind in FUNCTIONS
        body = is_function and any(
            isinstance(x, dict) and x.get("kind") in ("CompoundStmt", "CXXTryStmt")
            for x in (node.get("inner") or []))
        if eligible and kind in DECLS and name:
            symbols.append({
                "name": name, "qualified_name": qualified, "kind": kind,
                "line": line_of(node),
                "type": (node.get("type") or {}).get("qualType", ""),
                "definition": bool(body or (
                    kind in ("CXXRecordDecl", "RecordDecl")
                    and node.get("completeDefinition"))),
            })
            if len(symbols) > MAX_OBSERVATIONS:
                raise ValueError("Clang declaration budget exceeded")
        if eligible and caller and kind in CALLS:
            children = node.get("inner") or []
            callee = callee_of(children[0]) if children else None
            if callee:
                calls.append({
                    "caller": caller, "callee": callee,
                    "line": line_of(node), "kind": kind,
                })
                if len(symbols) + len(calls) > MAX_OBSERVATIONS:
                    raise ValueError("Clang call budget exceeded")
        next_scope = scope
        if eligible and kind in SCOPES and isinstance(name, str):
            next_scope = (*scope, name)
        next_caller = qualified if eligible and body else caller
        for child in reversed(node.get("inner") or []):
            stack.append((child, here, next_scope, next_caller))
    symbols.sort(key=lambda x: json.dumps(x, sort_keys=True))
    calls.sort(key=lambda x: json.dumps(x, sort_keys=True))
    return symbols, calls


def analyze(item):
    if item.get("trusted_local") is not True:
        raise ValueError("Clang may run only with explicit trusted-local permission")
    repo = Path(item["repo"]).resolve(strict=True)
    source = (repo / item["source_path"]).resolve(strict=True)
    suffix = source.suffix
    if not source.is_relative_to(repo) or suffix not in (".c", ".cc", ".cpp", ".cxx", ".C"):
        raise ValueError("Clang source must be a tracked C/C++ translation unit")
    language = "c" if suffix == ".c" else "c++"
    expected = item["source_blob"]
    if not re.fullmatch(r"[0-9a-f]{40}", expected):
        raise ValueError("invalid Git source OID")
    _, source_bytes = digest_file(source, 1024 * 1024)
    real_oid = hashlib.sha1(b"blob " + str(len(source_bytes)).encode("ascii") +
                            b"\0" + source_bytes).hexdigest()
    if real_oid != expected:
        raise ValueError("working source differs from pinned Git blob")
    db_path = Path(item["compile_db"]).resolve(strict=True)
    db_digest, entry, directory, original = load_command(db_path, source)
    flags = analysis_flags(original, source, directory)
    config_path = item.get("config")
    config = None
    if config_path is not None:
        config_file = Path(config_path).resolve(strict=True)
        config_sha, _ = digest_file(config_file, 4 * 1024 * 1024)
        config = {"path": str(config_file), "sha256": config_sha}
    compiler_name = item.get("compiler") or ("clang" if language == "c" else "clang++")
    if not re.fullmatch(r"(?:clang(?:[+][+])?(?:-[0-9]+)?|/[A-Za-z0-9_./+-]*clang(?:[+][+])?(?:-[0-9]+)?)", compiler_name):
        raise ValueError("compiler must be an explicitly selected Clang C/C++ binary")
    compiler_location = shutil.which(compiler_name)
    if not compiler_location:
        raise ValueError("selected Clang compiler unavailable")
    compiler = Path(compiler_location).resolve(strict=True)
    compiler_digest, _ = digest_file(compiler, 256 * 1024 * 1024)
    version, _ = invoke([str(compiler), "--version"], directory, 8192, timeout=5)
    actual_args = [str(compiler)] + flags
    dep_output, dep_diagnostics = invoke(actual_args + ["-M", "-MT", "excession"],
                                         directory, MAX_DEP_BYTES)
    dependencies = header_observations(
        make_dependencies(dep_output.decode("utf-8")), directory, repo)
    ast_output, ast_diagnostics = invoke(
        actual_args + ["-Xclang", "-ast-dump=json", "-fsyntax-only"],
        directory, MAX_AST_BYTES)
    try:
        ast = json.loads(ast_output)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("Clang did not return a valid JSON AST") from error
    symbols, calls = extract_symbols(ast, source, source_bytes)
    # Ensure all identity-bearing data stayed fixed across both subprocesses.
    _, end_bytes = digest_file(source, 1024 * 1024)
    if end_bytes != source_bytes:
        raise ValueError("pinned Git source changed during Clang analysis")
    if digest_file(db_path, MAX_DB_BYTES)[0] != db_digest:
        raise ValueError("compilation database changed during Clang analysis")
    if header_observations(
            make_dependencies(dep_output.decode("utf-8")), directory, repo) != dependencies:
        raise ValueError("observed dependencies changed during Clang analysis")
    if config is not None and digest_file(config_file, 4 * 1024 * 1024)[0] != config["sha256"]:
        raise ValueError("configuration changed during Clang analysis")
    entry_digest = digest(json.dumps(entry, sort_keys=True,
                                     separators=(",", ":")).encode("utf-8"))
    return {
        "status": "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED",
        "source_path": item["source_path"],
        "source_blob": expected,
        "language": language,
        "compilation_database": {"path": str(db_path), "sha256": db_digest,
                                 "entry_sha256": entry_digest},
        "original_compiler": original[0],
        "effective_compiler": str(compiler),
        "compiler_sha256": compiler_digest,
        "compiler_version": version.decode("utf-8", "replace").splitlines()[0],
        "effective_args": actual_args,
        "config": config,
        "dependencies": dependencies,
        "symbols": symbols,
        "calls": calls,
        "diagnostics": [d for d in (dep_diagnostics, ast_diagnostics) if d],
        "limitations": [
            "Trusted-local Clang reparse, not an observed build",
            "Compiler flags originate in a chosen compilation database entry",
            "Generated or system headers are content-hashed but not Git-authenticated",
            "Clang AST covers one translation unit and one compiler configuration",
        ],
    }


def main():
    items = json.load(sys.stdin)
    if not isinstance(items, list) or len(items) != 1 or not isinstance(items[0], dict):
        raise ValueError("expected exactly one compiler analysis request")
    result = analyze(items[0])
    print(json.dumps([result], sort_keys=True, separators=(",", ":"), ensure_ascii=False))


if __name__ == "__main__":
    main()
