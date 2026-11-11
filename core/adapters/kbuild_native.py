"""Trusted-local Kbuild export through Linux's own compile-command generator.
No Makefile/.cmd grammar is reimplemented; only the pinned upstream generator
parses those records. No compilation or shell execution is performed.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

MAX_MEMBERS = 10000
MAX_CMD = 2 * 1024 * 1024

def inside(root, path):
    path = Path(path).resolve()
    if not path.is_relative_to(root):
        raise ValueError("Kbuild object/record escapes build directory")
    return path

def digest(path):
    if path.stat().st_size > 256 * 1024 * 1024:
        raise ValueError("artifact exceeds bounded hashing limit")
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()

def export(repo, build, archive_relative, archiver):
    repo, build = Path(repo).resolve(), Path(build).resolve()
    if not archive_relative.endswith(".a") or Path(archive_relative).is_absolute():
        raise ValueError("archive must be a relative .a file")
    archive = inside(build, build / archive_relative)
    upstream_path = repo / "scripts/clang-tools/gen_compile_commands.py"
    if not upstream_path.is_file() or not archive.is_file():
        raise ValueError("missing Linux compile-command generator or archive")
    spec = importlib.util.spec_from_file_location("linux_compilation_db", upstream_path)
    if spec is None or spec.loader is None:
        raise ValueError("cannot load native compile-command generator")
    producer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(producer)
    output = subprocess.run([archiver, "t", str(archive)], cwd=build,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            check=True, timeout=30)
    if len(output.stdout) > 2 * 1024 * 1024:
        raise ValueError("archive listing exceeds byte budget")
    archive_members = output.stdout.decode("utf-8").splitlines()
    if not archive_members or len(archive_members) > MAX_MEMBERS:
        raise ValueError("archive member count unsupported")
    if len(set(archive_members)) != len(archive_members):
        raise ValueError("duplicate archive member identity")
    matcher = producer.re.compile(producer._LINE_PATTERN)
    members = []
    for name in archive_members:
        if not name.endswith(".o") or name.startswith("/") or "\\" in name:
            raise ValueError("unsupported archive member name")
        if any(part in ("", ".", "..") for part in name.split("/")):
            raise ValueError("archive member traversal")
        object_path = inside(build, build / name)
        cmd = inside(build, Path(producer.to_cmdfile(str(object_path))))
        member = {"object": name,
                  "object_sha256": digest(object_path) if object_path.is_file() else None,
                  "cmd": cmd.relative_to(build).as_posix(),
                  "cmd_sha256": None, "source": None, "command": None,
                  "status": "MISSING_KBUILD_COMMAND"}
        if cmd.is_file():
            if cmd.stat().st_size > MAX_CMD:
                member["status"] = "OVERSIZED_KBUILD_COMMAND"
            else:
                member["cmd_sha256"] = digest(cmd)
                with cmd.open("r", encoding="utf8", errors="replace") as handle:
                    first = handle.readline()
                match = matcher.match(first)
                if match is not None:
                    try:
                        parsed = producer.process_line(
                            str(build), match.group("command_prefix"), match.group("file_path")
                        )
                    except ValueError:
                        member["status"] = "SOURCE_UNAVAILABLE"
                    else:
                        member["source"] = parsed["file"]
                        member["command"] = parsed["command"]
                        member["status"] = "KBUILD_COMPILE_RECORD"
                else:
                    member["status"] = "UNSUPPORTED_KBUILD_COMMAND"
        members.append(member)
    data = {
        "schema": "excession.kbuild-native-export.v1",
        "archive": archive_relative,
        "archive_sha256": digest(archive),
        "producer_sha256": digest(upstream_path),
        "archive_listing_sha256": hashlib.sha256(output.stdout).hexdigest(),
        "archiver": archiver,
        "members": members,
    }
    encoded = json.dumps(data, sort_keys=True, separators=(",", ":")).encode()
    if len(encoded) > 24 * 1024 * 1024:
        raise ValueError("Kbuild export exceeds byte budget")
    return encoded

if __name__ == "__main__":
    if len(sys.argv) != 5:
        sys.exit("usage: kbuild_native.py REPO BUILD ARCHIVE ARCHIVER")
    try:
        sys.stdout.buffer.write(export(*sys.argv[1:]))
    except (OSError, ValueError, subprocess.SubprocessError) as err:
        sys.exit("Kbuild export refused: " + str(err))
