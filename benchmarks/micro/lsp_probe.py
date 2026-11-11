#!/usr/bin/env python3
"""Reproducible JSON-RPC language-server definition probe.

Measure real initialize/definition latency, transport bytes, and locations.
No agent behavior or subjective completion score is inferred.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import select
import subprocess
import time
from typing import Any


class ProtocolError(Exception):
    pass


class LspProcess:
    def __init__(self, command: list[str], cwd: Path, timeout: float) -> None:
        self.proc = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.timeout = timeout
        self.pending = b""
        self.bytes_received = 0
        self.bytes_sent = 0
        self.next_id = 0

    def write(self, message: dict[str, Any]) -> None:
        payload = json.dumps(message, separators=(",", ":")).encode()
        framed = b"Content-Length: " + str(len(payload)).encode() + b"\r\n\r\n" + payload
        assert self.proc.stdin is not None
        self.proc.stdin.write(framed)
        self.proc.stdin.flush()
        self.bytes_sent += len(framed)

    def _fill(self, deadline: float) -> None:
        assert self.proc.stdout is not None
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("LSP response deadline exceeded")
        ready, _, _ = select.select([self.proc.stdout.fileno()], [], [], remaining)
        if not ready:
            raise TimeoutError("LSP response deadline exceeded")
        chunk = os.read(self.proc.stdout.fileno(), 65536)
        if not chunk:
            raise ProtocolError("LSP server closed stdout")
        self.pending += chunk
        self.bytes_received += len(chunk)
        if len(self.pending) > 32 * 1024 * 1024:
            raise ProtocolError("LSP response exceeds 32 MiB")

    def read(self, deadline: float) -> dict[str, Any]:
        while b"\r\n\r\n" not in self.pending:
            self._fill(deadline)
        header, rest = self.pending.split(b"\r\n\r\n", 1)
        values = [line.split(b":", 1)[1].strip() for line in header.split(b"\r\n")
                  if line.lower().startswith(b"content-length:")]
        if len(values) != 1:
            raise ProtocolError("missing or duplicate Content-Length")
        try:
            length = int(values[0])
        except ValueError as exc:
            raise ProtocolError("invalid Content-Length") from exc
        if length < 0 or length > 16 * 1024 * 1024:
            raise ProtocolError("oversized LSP message")
        while len(rest) < length:
            self._fill(deadline)
            _, rest = self.pending.split(b"\r\n\r\n", 1)
        payload, self.pending = rest[:length], rest[length:]
        message = json.loads(payload)
        if not isinstance(message, dict):
            raise ProtocolError("non-object LSP message")
        return message

    def request(self, method: str, params: dict[str, Any]) -> Any:
        self.next_id += 1
        wanted = self.next_id
        self.write({"jsonrpc": "2.0", "id": wanted, "method": method, "params": params})
        deadline = time.monotonic() + self.timeout
        while True:
            incoming = self.read(deadline)
            if incoming.get("id") == wanted and "method" not in incoming:
                if "error" in incoming:
                    raise ProtocolError(f"{method}: {incoming['error']}")
                return incoming.get("result")
            if "method" in incoming and "id" in incoming:
                # Reply to unsolicited server requests such as workspace/configuration.
                self.write({"jsonrpc": "2.0", "id": incoming["id"], "result": None})

    def close(self) -> None:
        if self.proc.poll() is None:
            try:
                self.request("shutdown", {})
                self.write({"jsonrpc": "2.0", "method": "exit"})
                self.proc.wait(timeout=2)
            except (ProtocolError, TimeoutError, BrokenPipeError, subprocess.TimeoutExpired):
                self.proc.kill()
                self.proc.wait()
        if self.proc.stdin:
            self.proc.stdin.close()
        if self.proc.stdout:
            self.proc.stdout.close()


def probe(command: list[str], repo: Path, relative: str, line: int, character: int,
          language: str, timeout: float) -> dict[str, Any]:
    path = (repo / relative).resolve(strict=True)
    if repo not in path.parents:
        raise ValueError("file outside pinned repository")
    data = path.read_bytes()
    source = data.decode("utf-8")
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter_ns()
    server = LspProcess(command, repo, timeout)
    receipt: dict[str, Any] = {
        "status": "error",
        "document_sha256": hashlib.sha256(data).hexdigest(),
    }
    try:
        init = server.request("initialize", {
            "processId": None, "rootUri": repo.as_uri(),
            "capabilities": {"textDocument": {"definition": {"linkSupport": False}}},
            "workspaceFolders": [{"uri": repo.as_uri(), "name": repo.name}],
        })
        receipt["initialize_wall_ms"] = round(
            (time.perf_counter_ns() - start) / 1_000_000, 3
        )
        receipt["server_capabilities"] = sorted((init or {}).get("capabilities", {}).keys())
        server.write({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        server.write({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": {
                "uri": path.as_uri(), "languageId": language,
                "version": 1, "text": source,
            }
        }})
        request_start = time.perf_counter_ns()
        result = server.request("textDocument/definition", {
            "textDocument": {"uri": path.as_uri()},
            "position": {"line": line, "character": character},
        })
        receipt["definition_wall_ms"] = round(
            (time.perf_counter_ns() - request_start) / 1_000_000, 3
        )
        locations = result if isinstance(result, list) else ([result] if result else [])
        receipt["locations"] = locations
        receipt["location_count"] = len(locations)
        receipt["status"] = "ok"
    except (ProtocolError, TimeoutError, OSError, ValueError, json.JSONDecodeError) as exc:
        receipt["error"] = str(exc)
    finally:
        # Wall time ends with the definition response, before server shutdown.
        receipt["wall_ms"] = round((time.perf_counter_ns() - start) / 1_000_000, 3)
        receipt["wire_bytes_sent"] = server.bytes_sent
        receipt["wire_bytes_received"] = server.bytes_received
        server.close()
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        receipt["child_cpu_including_shutdown_ms"] = round(
            1000 * ((after.ru_utime + after.ru_stime) -
                    (before.ru_utime + before.ru_stime)), 3
        )
    return receipt


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--repo", required=True)
    p.add_argument("--file", required=True)
    p.add_argument("--line", required=True, type=int, help="zero-based")
    p.add_argument("--character", required=True, type=int, help="zero-based UTF-16 offset")
    p.add_argument("--language", default="c")
    p.add_argument("--timeout", type=float, default=15)
    p.add_argument("--out", required=True)
    p.add_argument("command", nargs="+", help="server and arguments; use -- before flags")
    a = p.parse_args()
    repo = Path(a.repo).resolve(strict=True)
    commit = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
    ).strip()
    if subprocess.check_output(
        ["git", "-C", str(repo), "status", "--porcelain"]
    ).strip():
        p.error("LSP probes require a clean pinned checkout")
    receipt = probe(
        a.command, repo, a.file, a.line, a.character, a.language, a.timeout
    )
    receipt["source_commit"] = commit
    receipt["server_command"] = a.command
    dest = Path(a.out)
    if dest.exists():
        p.error("output already exists")
    dest.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(json.dumps(receipt, indent=2, sort_keys=True))
    if receipt["status"] != "ok":
        raise SystemExit(2)


if __name__ == "__main__":
    main()
