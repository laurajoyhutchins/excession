#!/usr/bin/env python3
"""Bounded, source-authenticated Python slices and conservative evidence carry.

This is a trusted-local prototype. It is NOT an untrusted-code sandbox or a
proof that static imports exhaust Python's runtime dependencies.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
from typing import Any

SCHEMA = "excession.executable-slice.v1"
RECEIPT = "excession.slice-observation.v1"
MAX_FILES = 512
MAX_BYTES = 32 * 1024 * 1024
MAX_OUTPUT = 1024 * 1024


class Refusal(ValueError):
    pass


def git(repo: Path, *args: str, max_bytes: int = MAX_BYTES + 4096) -> bytes:
    p = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, check=False)
    if p.returncode or len(p.stdout) > max_bytes:
        raise Refusal(f"Git command refused: {' '.join(args[:2])}: {p.stderr[:240]!r}")
    return p.stdout


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def clean_revision(repo: Path, revision: str) -> tuple[str, str]:
    if not repo.is_dir() or git(repo, "status", "--porcelain", "--untracked-files=normal").strip():
        raise Refusal("source checkout must be clean")
    head = git(repo, "rev-parse", "--verify", "HEAD^{commit}").decode().strip()
    oid = git(repo, "rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip()
    if head != oid or len(oid) != 40:
        raise Refusal("revision must equal the clean checkout HEAD (SHA-1 Git only)")
    tree = git(repo, "rev-parse", f"{oid}^{{tree}}").decode().strip()
    return oid, tree


def inventory(repo: Path, oid: str) -> dict[str, tuple[str, str]]:
    data = git(repo, "ls-tree", "-rz", "--full-tree", oid)
    result = {}
    for row in data.split(b"\0"):
        if not row:
            continue
        meta, path = row.split(b"\t", 1)
        mode, kind, blob = meta.decode("ascii").split(" ")
        name = path.decode("utf-8", "strict")
        if name in result:
            raise Refusal("duplicate Git path")
        result[name] = (mode, blob)
    return result


def safe_path(name: str) -> bool:
    return bool(name) and "\\" not in name and not name.startswith("/") and all(
        p not in ("", ".", "..") for p in name.split("/"))


def blob_bytes(repo: Path, oid: str, path: str, expected: str) -> bytes:
    data = git(repo, "show", f"{oid}:{path}", max_bytes=MAX_BYTES + 1)
    # Compare the actual bytes to the content-addressed Git object, not the worktree.
    obj = git(repo, "rev-parse", f"{oid}:{path}").decode().strip()
    if obj != expected:
        raise Refusal(f"Git source object changed: {path}")
    return data


def module_path(name: str, files: dict[str, tuple[str, str]]) -> str | None:
    stem = name.replace(".", "/")
    if not stem or any(not s.isidentifier() for s in name.split(".")):
        return None
    for p in (stem + ".py", stem + "/__init__.py"):
        if p in files:
            return p
    return None


def parent_inits(path: str, files: dict[str, tuple[str, str]]) -> list[str]:
    parts = path.split("/")[:-1]
    acc: list[str] = []
    for i in range(1, len(parts) + 1):
        init = "/".join(parts[:i]) + "/__init__.py"
        if init not in files:
            raise Refusal(f"namespace package unsupported (missing {init})")
        acc.append(init)
    return acc


def exported(tree: ast.Module, member: str) -> bool:
    """Conservative top-level export check; no arbitrary execution."""
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) and node.name == member:
            return True
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            if any(isinstance(t, ast.Name) and t.id == member for t in targets):
                return True
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            if any((a.asname or a.name.split(".")[0]) == member for a in node.names):
                return True
    return False


def imports(path: str, data: bytes, files: dict[str, tuple[str, str]],
            repo: Path, oid: str) -> set[str]:
    try:
        tree = ast.parse(data.decode("utf-8-sig"), filename=path)
    except (SyntaxError, UnicodeError) as exc:
        raise Refusal(f"cannot parse Python source {path}: {exc}") from exc
    mod = path.removesuffix(".py").replace("/", ".")
    is_package = mod.endswith(".__init__")
    if is_package:
        mod = mod.removesuffix(".__init__")
    package = mod if is_package else mod.rpartition(".")[0]
    result: set[str] = set()

    def include(name: str, relative: bool = False) -> None:
        resolved = module_path(name, files)
        if resolved is not None:
            result.add(resolved)
        elif (relative or module_path(name.split(".")[0], files)
              or name.split(".")[0] not in sys.stdlib_module_names):
            raise Refusal(f"unresolved or external import {name!r} in {path}")

    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            fn = node.func
            dynamic = (isinstance(fn, ast.Name) and fn.id in {"__import__", "eval", "exec"}) or (
                isinstance(fn, ast.Attribute) and fn.attr in {"import_module", "spec_from_file_location", "run_path", "run_module"}
            )
            if dynamic:
                raise Refusal(f"dynamic code loading unsupported in {path}:{node.lineno}")
        if isinstance(node, ast.Import):
            for alias in node.names:
                include(alias.name)
        if isinstance(node, ast.ImportFrom):
            if node.level:
                groups = package.split(".") if package else []
                if node.level > len(groups):
                    raise Refusal(f"relative import escapes root in {path}")
                prefix = groups[:len(groups) - node.level + 1]
                base = ".".join(prefix + ([node.module] if node.module else []))
            else:
                base = node.module or ""
            if not base:
                raise Refusal(f"unresolved import in {path}")
            include(base, relative=bool(node.level))
            for alias in node.names:
                if alias.name == "*":
                    raise Refusal(f"star import unsupported in {path}")
                maybe = f"{base}.{alias.name}"
                if module_path(maybe, files):
                    include(maybe, relative=bool(node.level))
                elif module_path(base, files):
                    base_file = module_path(base, files)
                    assert base_file is not None
                    source = blob_bytes(repo, oid, base_file, files[base_file][1])
                    try:
                        exported_tree = ast.parse(source.decode("utf-8-sig"), filename=base_file)
                    except (UnicodeError, SyntaxError) as exc:
                        raise Refusal(f"invalid imported module: {base_file}") from exc
                    if not exported(exported_tree, alias.name):
                        raise Refusal(f"unresolved imported symbol {base}.{alias.name} in {path}")
    return result


def build_plan(repo: Path, oid: str, tree: str, entry: str,
               assets: list[str] | None = None) -> dict[str, Any]:
    files = inventory(repo, oid)
    if not safe_path(entry) or not entry.endswith(".py") or entry not in files:
        raise Refusal("entry must be a tracked repository-relative Python file")
    pending = {entry, *parent_inits(entry, files), *(assets or [])}
    done: set[str] = set()
    total = 0
    entries: list[dict[str, Any]] = []
    while pending:
        path = min(pending)
        pending.remove(path)
        if path in done:
            continue
        if not safe_path(path) or path not in files:
            raise Refusal(f"missing or unsafe tracked file: {path}")
        mode, blob = files[path]
        if mode != "100644":
            raise Refusal(f"only regular nonexecutable files supported: {path} ({mode})")
        data = blob_bytes(repo, oid, path, blob)
        total += len(data)
        if total > MAX_BYTES or len(done) >= MAX_FILES:
            raise Refusal("slice exceeds resource budget")
        done.add(path)
        entries.append({"path": path, "blob": blob, "bytes": len(data), "sha256": digest(data)})
        if path.endswith(".py"):
            for dependency in imports(path, data, files, repo, oid):
                pending.add(dependency)
                pending.update(parent_inits(dependency, files))
    body: dict[str, Any] = {"schema": SCHEMA, "commit": oid, "tree": tree, "entry": entry,
                            "assets": sorted(assets or []), "entries": sorted(entries, key=lambda e: e["path"]),
                            "scope": "STATIC_IMPORT_CLOSURE_ONLY", "semantic_equivalence": "UNVERIFIED"}
    body["view_id"] = digest(canonical(body))
    return body


def plan(repo: Path, entry: str, revision: str = "HEAD", assets: list[str] | None = None) -> dict[str, Any]:
    oid, tree = clean_revision(repo, revision)
    return build_plan(repo, oid, tree, entry, assets)


def validate_manifest(repo: Path, value: dict[str, Any]) -> None:
    if value.get("schema") != SCHEMA or not isinstance(value.get("view_id"), str):
        raise Refusal("invalid slice manifest schema")
    body = {k: v for k, v in value.items() if k != "view_id"}
    if digest(canonical(body)) != value["view_id"]:
        raise Refusal("slice manifest identity mismatch")
    regenerated = plan(repo, value["entry"], value["commit"], value["assets"])
    if regenerated != value:
        raise Refusal("slice differs from deterministic source-derived plan")


def materialize(repo: Path, manifest: dict[str, Any], destination: Path) -> None:
    validate_manifest(repo, manifest)
    destination = destination.absolute()
    if destination.exists() or destination.is_symlink() or not destination.parent.is_dir():
        raise Refusal("destination must not exist and parent must exist")
    if repo.resolve() == destination or repo.resolve() in destination.parents:
        raise Refusal("destination cannot be inside source checkout")
    temp = Path(tempfile.mkdtemp(prefix=".excession-", dir=destination.parent))
    try:
        for item in manifest["entries"]:
            path = temp / item["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            data = blob_bytes(repo, manifest["commit"], item["path"], item["blob"])
            path.write_bytes(data)
            path.chmod(0o444)
        os.replace(temp, destination)
    finally:
        if temp.exists():
            shutil.rmtree(temp)


def audit(repo: Path, manifest: dict[str, Any], directory: Path) -> None:
    validate_manifest(repo, manifest)
    expected = {x["path"]: x for x in manifest["entries"]}
    found: set[str] = set()
    if not directory.is_dir() or directory.is_symlink():
        raise Refusal("slice directory missing or linked")
    for base, dirs, names in os.walk(directory, followlinks=False):
        for d in dirs:
            if (Path(base) / d).is_symlink():
                raise Refusal("slice contains symlink directory")
        for name in names:
            p = Path(base) / name
            key = p.relative_to(directory).as_posix()
            if key not in expected or not stat.S_ISREG(p.lstat().st_mode):
                raise Refusal(f"unexpected or nonregular slice file: {key}")
            if p.stat().st_mode & 0o222:
                raise Refusal(f"writable slice file: {key}")
            data = p.read_bytes()
            if digest(data) != expected[key]["sha256"] or len(data) != expected[key]["bytes"]:
                raise Refusal(f"slice file differs from Git blob: {key}")
            found.add(key)
    if found != set(expected):
        raise Refusal(f"missing slice files: {sorted(set(expected) - found)}")


# Deliberately local and unsigned: namespace isolation is not a remote attestation.
# The host controls argv, mount list, source inputs, and namespace setup. The
# source program is never interpolated into shell code.
_LINUX_WORKER = r"""set -eu
jail="$1"
source_dir="$2"
entry="$3"
cpu="$4"
mkdir -p "$jail/usr" "$jail/work" "$jail/tmp" "$jail/dev"
ln -s usr/lib "$jail/lib"
ln -s usr/lib64 "$jail/lib64"
ln -s usr/bin "$jail/bin"
touch "$jail/dev/null"
mount --make-rprivate /
mount --bind /usr "$jail/usr"
mount -o remount,bind,ro "$jail/usr"
mount --bind "$source_dir" "$jail/work"
mount -o remount,bind,ro "$jail/work"
mount --bind /dev/null "$jail/dev/null"
mount -t tmpfs -o size=32m,nosuid,nodev,noexec tmpfs "$jail/tmp"
exec prlimit --cpu="$cpu" --as=1073741824 --nofile=128 -- \
  /usr/sbin/chroot "$jail" /usr/bin/setpriv --bounding-set=-all --no-new-privs \
  /usr/bin/python3 -I -B -c \
  "import os,runpy,sys;os.write(2,b'EXCESSION_WORKER_READY\\n');sys.path.insert(0,'/work');runpy.run_path('/work/'+sys.argv[1],run_name='__main__')" \
  "$entry"
"""


def _isolated_process(root: Path, entry: str, timeout: int) -> subprocess.CompletedProcess[bytes]:
    if sys.platform != "linux":
        raise Refusal("Linux namespace worker unavailable on this platform")
    for program in ("unshare", "mount", "chroot", "setpriv", "prlimit"):
        if shutil.which(program) is None:
            raise Refusal(f"Linux namespace worker requires {program}; no unsandboxed fallback")
    if not Path("/usr/bin/python3").is_file() or not Path("/usr/sbin/chroot").is_file():
        raise Refusal("isolated worker requires /usr/bin/python3 and /usr/sbin/chroot")
    with tempfile.TemporaryDirectory(prefix="excession-namespace-") as directory:
        jail = Path(directory) / "root"
        jail.mkdir()
        cmd = ["unshare", "--user", "--map-root-user", "--mount", "--pid", "--fork",
               "--ipc", "--uts", "--net", "/bin/sh", "-c", _LINUX_WORKER,
               "excession-sandbox", str(jail), str(root), entry, str(timeout + 2)]
        # The ready marker comes from Python after entering the new namespaces.
        # A namespace/setup failure is a refusal, not a failed test receipt.
        result = subprocess.run(cmd, cwd=directory, capture_output=True,
                                timeout=timeout + 8,
                                env={"PATH": "/usr/sbin:/usr/bin:/bin", "LC_ALL": "C",
                                     "PYTHONDONTWRITEBYTECODE": "1", "PYTHONNOUSERSITE": "1"})
        marker = b"EXCESSION_WORKER_READY\n"
        if not result.stderr.startswith(marker):
            raise Refusal("isolated worker setup did not establish execution boundary")
        result.stderr = result.stderr[len(marker):]
        return result


def execute(repo: Path, manifest: dict[str, Any], directory: Path, timeout: int,
            isolated_linux: bool = False) -> dict[str, Any]:
    if not 1 <= timeout <= 60:
        raise Refusal("execution timeout must be 1..60 seconds")
    audit(repo, manifest, directory)
    root = directory.resolve()
    code = "import runpy,sys;sys.path.insert(0,sys.argv[1]);runpy.run_path(sys.argv[2],run_name='__main__')"
    cmd = [sys.executable, "-I", "-B", "-c", code, str(root), str(root / manifest["entry"])]
    # Native execution is only permitted by explicit --trusted-local; this
    # subprocess is not a security sandbox for arbitrary third-party code.
    try:
        if isolated_linux:
            p = _isolated_process(root, manifest["entry"], timeout)
        else:
            p = subprocess.run(cmd, cwd=root, capture_output=True, timeout=timeout, env={
                "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "PYTHONDONTWRITEBYTECODE": "1",
                "PYTHONNOUSERSITE": "1", "HOME": str(root), "LC_ALL": "C.UTF-8"})
        if len(p.stdout) > MAX_OUTPUT or len(p.stderr) > MAX_OUTPUT:
            raise Refusal("execution output exceeds 1 MiB")
        status, out, err = p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        status, out, err = 124, b"", b"timeout"
    # Native code may have created or modified files even when it exits zero.
    # Reject evidence if the reconstructed source is no longer exactly pinned.
    audit(repo, manifest, directory)
    result = {"schema": RECEIPT, "view_id": manifest["view_id"], "commit": manifest["commit"],
              "entry": manifest["entry"], "python": sys.version.split()[0],
              "exit_code": status, "stdout_sha256": digest(out), "stderr_sha256": digest(err),
              "status": "OBSERVED_PASS" if status == 0 else "OBSERVED_FAIL",
              "scope": "FINITE_TRUSTED_LOCAL_EXECUTION",
              "isolation": "LOCAL_LINUX_NAMESPACES_UNATTESTED" if isolated_linux else "NONE"}
    result["receipt_id"] = digest(canonical(result))
    return result


def carry(repo: Path, previous: dict[str, Any], receipt: dict[str, Any],
          revision: str = "HEAD") -> dict[str, Any]:
    if receipt.get("schema") != RECEIPT or receipt.get("view_id") != previous.get("view_id"):
        raise Refusal("observation does not match prior slice")
    if digest(canonical({k: v for k, v in receipt.items() if k != "receipt_id"})) != receipt.get("receipt_id"):
        raise Refusal("observation receipt identity mismatch")
    if receipt.get("status") != "OBSERVED_PASS":
        raise Refusal("only passing historical observations may be carried")
    oid, _ = clean_revision(repo, revision)
    # Replan on the target to detect new import-resolution paths and closure changes.
    # Recompute historical evidence from target's Git object store. Hashes in
    # caller-provided JSON are not sufficient authentication.
    old_oid = previous.get("commit")
    if not isinstance(old_oid, str) or len(old_oid) != 40:
        raise Refusal("invalid historical commit")
    old_tree = git(repo, "rev-parse", "--verify", f"{old_oid}^{{tree}}").decode().strip()
    if build_plan(repo, old_oid, old_tree, previous["entry"], previous["assets"]) != previous:
        raise Refusal("historical manifest does not match source Git objects")
    target_tree = git(repo, "rev-parse", "--verify", f"{oid}^{{tree}}").decode().strip()
    target = build_plan(repo, oid, target_tree, previous["entry"], previous["assets"])
    old = {x["path"]: x["blob"] for x in previous["entries"]}
    new = {x["path"]: x["blob"] for x in target["entries"]}
    differences = sorted(k for k in old.keys() | new.keys() if old.get(k) != new.get(k))
    unchanged = not differences
    return {"schema": "excession.evidence-carry.v1", "source_commit": previous["commit"],
            "target_commit": oid, "source_view_id": previous["view_id"],
            "target_view_id": target["view_id"], "previous_receipt_id": receipt["receipt_id"],
            "changed_closure_paths": differences,
            "decision": "SOURCE_MATCH_ONLY" if unchanged else "REVERIFY_REQUIRED",
            "target_behavior_verified": False,
            "reason": "Source closure unchanged; runtime/environment effects outside the slice remain unverified"
            if unchanged else "Source closure or import resolution changed; fresh target tests required"}


def read_json(path: str) -> dict[str, Any]:
    result = json.loads(Path(path).read_text())
    if not isinstance(result, dict):
        raise Refusal("JSON object required")
    return result


def cli() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("plan")
    p.add_argument("repo", type=Path)
    p.add_argument("entry")
    p.add_argument("--revision", default="HEAD")
    p.add_argument("--asset", action="append", default=[])
    p = sub.add_parser("materialize")
    p.add_argument("repo", type=Path)
    p.add_argument("manifest")
    p.add_argument("destination", type=Path)
    p = sub.add_parser("audit")
    p.add_argument("repo", type=Path)
    p.add_argument("manifest")
    p.add_argument("directory", type=Path)
    p = sub.add_parser("run")
    p.add_argument("repo", type=Path)
    p.add_argument("manifest")
    p.add_argument("directory", type=Path)
    p.add_argument("--timeout", type=int, default=10)
    p.add_argument("--trusted-local", action="store_true")
    p.add_argument("--isolated-linux", action="store_true",
                   help="use fail-closed unprivileged Linux namespaces")
    p = sub.add_parser("carry")
    p.add_argument("target_repo", type=Path)
    p.add_argument("source_manifest")
    p.add_argument("source_receipt")
    p.add_argument("--revision", default="HEAD")
    p.add_argument("--replay", action="store_true", help="rerun target slice as fresh evidence")
    p.add_argument("--trusted-local", action="store_true")
    p.add_argument("--isolated-linux", action="store_true")
    args = parser.parse_args()
    try:
        if args.action == "plan":
            result = plan(args.repo, args.entry, args.revision, args.asset)
        elif args.action == "materialize":
            materialize(args.repo, read_json(args.manifest), args.destination)
            result = {"status": "MATERIALIZED", "directory": str(args.destination)}
        elif args.action == "audit":
            audit(args.repo, read_json(args.manifest), args.directory)
            result = {"status": "AUDITED"}
        elif args.action == "run":
            if not args.trusted_local:
                raise Refusal("native execution requires explicit --trusted-local")
            result = execute(args.repo, read_json(args.manifest), args.directory, args.timeout,
                             args.isolated_linux)
        else:
            previous = read_json(args.source_manifest)
            result = carry(args.target_repo, previous,
                           read_json(args.source_receipt), args.revision)
            if args.replay:
                if not args.trusted_local:
                    raise Refusal("target execution requires explicit --trusted-local")
                fresh = plan(args.target_repo, previous["entry"], args.revision, previous["assets"])
                with tempfile.TemporaryDirectory(prefix="excession-replay-") as root:
                    directory = Path(root) / "slice"
                    materialize(args.target_repo, fresh, directory)
                    result["fresh_target_receipt"] = execute(args.target_repo, fresh, directory, 10,
                                                             args.isolated_linux)
        print(json.dumps(result, indent=2, sort_keys=True))
    except (Refusal, OSError, ValueError, KeyError, TypeError) as exc:
        parser.exit(2, f"excession slice refused: {exc}\n")


if __name__ == "__main__":
    cli()