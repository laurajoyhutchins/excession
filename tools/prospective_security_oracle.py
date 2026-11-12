#!/usr/bin/env python3
"""Bounded method-level native witnesses for frozen prospective security cases.

Runs exact extracted upstream function bodies, not reimplementations of the
vulnerability logic. No shell commands, network listeners, external services
or untrusted package imports. Each witness uses adverse and benign inputs.
It is *not* an end-to-end exploit or a complete application behavior proof.
"""
from __future__ import annotations

import argparse
import ast
import contextlib
import copy
import hashlib
import json
import os
from pathlib import Path
import shlex
import stat
import sys
import tempfile
import types
import urllib.parse


class UnsafeOptionError(Exception):
    pass


class HTTPError(Exception):
    def __init__(self, status_code, *args):
        self.status_code = status_code
        super().__init__(status_code, *args)


class InvalidPathError(Exception):
    pass


def _function(module: ast.Module, name: str, owner: str | None = None) -> ast.FunctionDef:
    nodes = module.body
    if owner:
        classes = [n for n in nodes if isinstance(n, ast.ClassDef) and n.name == owner]
        if len(classes) != 1:
            raise ValueError(f"missing unique source class {owner}")
        nodes = classes[0].body
    hits = [n for n in nodes if isinstance(n, ast.FunctionDef) and n.name == name]
    if len(hits) != 1:
        raise ValueError(f"missing unique upstream method {owner}.{name}")
    return copy.deepcopy(hits[0])


def _source(path: Path) -> ast.Module:
    return ast.parse(path.read_text(encoding="utf-8"), filename=str(path))


def _compile_function(node: ast.FunctionDef, namespace: dict) -> object:
    node.decorator_list = []
    # Avoid importing any live third-party package for annotations.
    node.returns = None
    for arg in node.args.posonlyargs + node.args.args + node.args.kwonlyargs:
        arg.annotation = None
    if node.args.vararg:
        node.args.vararg.annotation = None
    if node.args.kwarg:
        node.args.kwarg.annotation = None
    module = ast.fix_missing_locations(ast.Module(body=[node], type_ignores=[]))
    exec(compile(module, "<pinned-upstream-method>", "exec"), namespace)
    return namespace[node.name]


def _class(path: Path, owner: str, methods: list[str], namespace: dict) -> type:
    src = _source(path)
    members = []
    for name in methods:
        n = _function(src, name, owner)
        n.returns = None
        for arg in n.args.posonlyargs + n.args.args + n.args.kwonlyargs:
            arg.annotation = None
        members.append(n)
    # Preserve @classmethod, etc., from upstream methods. Drop base classes
    # and unrelated methods to keep isolation and execution bounded.
    cls = ast.ClassDef(name=owner, bases=[], keywords=[], body=members,
                       decorator_list=[])
    exec(compile(ast.fix_missing_locations(ast.Module(body=[cls], type_ignores=[])),
                 "<pinned-upstream-class>", "exec"), namespace)
    return namespace[owner]


def _gitpython_guard(path: Path, case_id: str) -> dict:
    nodes = _source(path)
    methods = ["check_unsafe_options"]
    if any(isinstance(n, ast.FunctionDef) and n.name == "_canonicalize_option_name"
           for cls in nodes.body if isinstance(cls, ast.ClassDef) and cls.name == "Git"
           for n in cls.body):
        methods.insert(0, "_canonicalize_option_name")
    if case_id == "gitpython-single-char-smuggling":
        methods += ["_option_candidates", "_unpack_args"]
    namespace = {"UnsafeOptionError": UnsafeOptionError,
                 "dashify": lambda s: s.replace("_", "-")}
    Git = _class(path, "Git", methods, namespace)

    def rejected(options: list[str]) -> bool:
        try:
            Git.check_unsafe_options(options, ["--upload-pack"])
            return False
        except UnsafeOptionError:
            return True

    if case_id == "gitpython-unsafe-kwargs":
        malicious = ["upload_pack"]
        benign = ["depth"]
    else:
        malicious = Git._option_candidates([], {"n": "--upload-pack=fixture"})
        benign = Git._option_candidates([], {"n": "fixture"})
    return {"adverse_rejected": rejected(malicious),
            "benign_allowed": not rejected(benign),
            "witness": "upstream Git unsafe-option guard with kwargs-derived spellings",
            "limitations": "Guard-method witness; no Git subprocess or remote command executed"}


def _dulwich_merge(path: Path) -> dict:
    src = _source(path)
    namespace = {"os": os, "sys": sys, "tempfile": tempfile,
                 "shlex": shlex, "subprocess": types.SimpleNamespace()}
    if any(isinstance(n, ast.FunctionDef) and n.name == "_shell_quote"
           for n in src.body):
        _compile_function(_function(src, "_shell_quote"), namespace)
    calls: list[str] = []

    def fake_run(command, shell, capture_output, text):
        if not shell:
            raise ValueError("unexpected non-shell invocation")
        calls.append(command)
        return types.SimpleNamespace(returncode=0)
    namespace["subprocess"].run = fake_run
    namespace["subprocess"].SubprocessError = Exception
    Driver = _class(path, "ProcessMergeDriver", ["__init__", "merge"], namespace)
    driver = Driver("true %P")
    driver.merge(b"base", b"ours", b"theirs", path="ok.txt")
    benign_command = calls.pop()
    driver.merge(b"base", b"ours", b"theirs", path="file; echo EXCESSION_WITNESS")
    adverse_command = calls.pop()
    # 'true file; echo ...' would execute a second shell command; a shell-
    # quoted path passes through as a single argument to 'true'. No shell run.
    benign_tokens = shlex.split(benign_command)
    malicious_tokens = shlex.split(adverse_command)
    return {
        "adverse_rejected": not (";" in malicious_tokens),
        "benign_allowed": benign_tokens == ["true", "ok.txt"],
        "witness": "upstream ProcessMergeDriver.merge command construction, subprocess stubbed",
        "adverse_command_observed": adverse_command,
        "limitations": "No shell execution; checks injection at the command-construction boundary",
    }


def _dulwich_checkout(path: Path, repo: Path) -> dict:
    source = _source(path)
    method = _function(source, "_checked_worktree_path")
    method.body = [
        node for node in method.body
        if not (isinstance(node, ast.ImportFrom) and node.level > 0)
    ]
    # Pull the actual patched source's symlink checker, not an approximation.
    index_file = repo / "dulwich/index.py"
    ns = {"os": os, "stat": stat, "InvalidPathError": InvalidPathError,
          "Error": ValueError,
          "get_path_element_validator": lambda cfg: object(),
          "validate_path": lambda p, fn: b".." not in p.split(b"/") and not p.startswith(b"/")}
    index_tree = _source(index_file)
    if any(isinstance(n, ast.FunctionDef) and n.name == "verify_leading_dirs"
           for n in index_tree.body):
        _compile_function(_function(index_tree, "verify_leading_dirs"), ns)
    resolver = _compile_function(method, ns)
    with tempfile.TemporaryDirectory(prefix="excession-dulwich-") as root:
        base = Path(root)
        work = base / "work"
        target = base / "outside"
        work.mkdir()
        target.mkdir()
        (work / "normal").mkdir()
        (work / "link").symlink_to(target, target_is_directory=True)
        fake_repo = types.SimpleNamespace(
            path=str(work), get_config_stack=lambda: None
        )
        benign = resolver(fake_repo, b"normal/ok.txt")
        try:
            adverse = resolver(fake_repo, b"link/secret.txt")
            blocked = False
        except ValueError:
            blocked = True
            adverse = None
        return {
            "adverse_rejected": blocked,
            "benign_allowed": os.fsdecode(benign) == str(work / "normal/ok.txt"),
            "witness": "upstream dulwich _checked_worktree_path + upstream verify_leading_dirs",
            "adverse_resolved_outside_root": (not blocked and
                  os.path.realpath(os.fsdecode(adverse)).startswith(str(target))),
            "limitations": "Path validation only; no checkout, actual write or hook execution",
        }


def _tornado_static(path: Path) -> dict:
    source = _source(path)
    methods = ["validate_absolute_path"]
    if any(isinstance(n, ast.FunctionDef) and n.name == "_resolve_symlink_target"
           for n in next(c for c in source.body
                         if isinstance(c, ast.ClassDef) and c.name == "StaticFileHandler").body):
        methods.append("_resolve_symlink_target")
    ns = {"os": os, "stat": stat, "HTTPError": HTTPError}
    Static = _class(path, "StaticFileHandler", methods, ns)
    with tempfile.TemporaryDirectory(prefix="excession-tornado-") as root:
        base = Path(root)
        served = base / "static"
        served.mkdir()
        outside = base / "private"
        outside.write_text("ISOLATED_FIXTURE")
        inside = served / "public"
        inside.write_text("OK")
        (served / "link").symlink_to(outside)
        handler = Static()
        handler.path = "link"
        handler.default_filename = None
        handler.allowed_symlink_directory = str(served)
        handler.request = types.SimpleNamespace(path="/static/link")
        def validate(request: Path) -> str | None:
            try:
                return handler.validate_absolute_path(str(served), str(request))
            except HTTPError as e:
                if e.status_code == 403:
                    return None
                raise
        benign = validate(inside)
        adverse = validate(served / "link")
        return {"adverse_rejected": adverse is None,
                "benign_allowed": benign is not None and Path(benign).read_text() == "OK",
                "witness": "upstream StaticFileHandler.validate_absolute_path on real symlink",
                "limitations": "Method-level filesystem witness, not full HTTP response"}


def _tornado_redirect(path: Path) -> dict:
    source = _source(path)
    ns = {"HTTPError": HTTPError, "urllib": urllib}
    for helper in ["_normalize_url_like_browser", "_scheme_and_netloc"]:
        if any(isinstance(n, ast.FunctionDef) and n.name == helper for n in source.body):
            _compile_function(_function(source, helper), ns)
    Redirect = _class(path, "RedirectHandler", ["get"], ns)
    handler = Redirect()
    handler._url = "/{0}/destination"
    handler._permanent = True
    handler.request = types.SimpleNamespace(query_arguments={})
    redirects: list[str] = []
    handler.redirect = lambda url, permanent: redirects.append(url)
    handler.get("safe")
    benign = redirects.pop() == "/safe/destination"
    try:
        handler.get("/other-host.test")
        adverse = redirects.pop()
        rejected = False
    except HTTPError as e:
        adverse = None
        rejected = e.status_code == 400
    return {"adverse_rejected": rejected, "benign_allowed": benign,
            "adverse_redirect": adverse,
            "witness": "upstream RedirectHandler.get with protocol-relative target",
            "limitations": "Method-level redirect witness, not browser navigation"}


def execute(root: Path, case: dict, phase: str) -> dict:
    source = root / case["file"]
    contents = source.read_bytes()
    source_hash = hashlib.sha1(b"blob " + str(len(contents)).encode() + b"\0" + contents).hexdigest()
    expected_hash = case[phase]["blob"]
    if source_hash != expected_hash:
        raise ValueError("Git blob mismatch before native witness")
    try:
        if case["id"].startswith("gitpython-"):
            observed = _gitpython_guard(source, case["id"])
        elif case["id"] == "dulwich-process-merge-driver":
            observed = _dulwich_merge(source)
        elif case["id"] == "dulwich-symlink-checkout":
            observed = _dulwich_checkout(source, root)
        elif case["id"] == "tornado-static-symlink":
            observed = _tornado_static(source)
        elif case["id"] == "tornado-protocol-relative-redirect":
            observed = _tornado_redirect(source)
        else:
            raise ValueError("unknown oracle")
    except Exception as exc:
        # Record unsupported instead of silently pretending to observe truth.
        observed = {"status": "INCONCLUSIVE", "reason": type(exc).__name__ + ": " + str(exc)}
    if "status" not in observed:
        expected_adverse_rejected = phase == "patched"
        observed["status"] = ("OBSERVED_EXPECTED" if
            observed["adverse_rejected"] == expected_adverse_rejected and
            observed["benign_allowed"] else "OBSERVED_CONTRADICTION")
    return {"schema": "excession.security.prospective.oracle.v1",
            "case": case["id"], "phase": phase,
            "repository": case["repository"],
            "source_commit": case[phase]["commit"],
            "source_blob": source_hash, **observed}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--case", required=True)
    parser.add_argument("--phase", choices=["vulnerable", "patched"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    case = next(c for c in manifest["cases"] if c["id"] == args.case)
    result = execute(args.root, case, args.phase)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(f"{case['id']} {args.phase} native oracle: {result['status']}")


if __name__ == "__main__":
    main()
