"""Execute actual h11 writers in isolated Git checkouts.

Candidate mode deliberately does not call the reader. Downstream observations
are recorded only from the original checkout.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import h11
from h11 import Data, EndOfMessage
from h11._writers import ChunkedWriter

from domain import corpus, digest

SUFFIX = b"GET /subsequent HTTP/1.1\r\nHost: example.com\r\n\r\n"
SEGMENTS = (1, 7, 257, 8192)


def encode(case: dict) -> list[bytes]:
    writer = ChunkedWriter()
    emissions: list[bytes] = []
    for part in case["payloads"]:
        writer(Data(data=bytes.fromhex(part)), emissions.append)
    writer(EndOfMessage(headers=[(bytes.fromhex(k), bytes.fromhex(v))
                                  for k, v in case["trailers"]]), emissions.append)
    return emissions


def read_once(wire: bytes, step: int) -> dict:
    from h11._readers import ChunkedReader
    from h11._receivebuffer import ReceiveBuffer

    decoder = ChunkedReader()
    buffer = ReceiveBuffer()
    payload: list[bytes] = []
    for start in range(0, len(wire), step):
        buffer += wire[start:start + step]
        while True:
            event = decoder(buffer)
            if event is None:
                break
            if isinstance(event, Data):
                payload.append(bytes(event.data))
            elif isinstance(event, EndOfMessage):
                return {
                    "payload": b"".join(payload).hex(),
                    "trailers": [[k.hex(), v.hex()] for k, v in event.headers],
                    "unread_suffix": (bytes(buffer) + wire[start + step:]).hex(),
                }
            else:
                raise ValueError("unexpected HTTP event")
    raise AssertionError("unterminated message")


def observe(root: Path, role: str) -> dict:
    if Path(h11.__file__).resolve().parents[1] != root.resolve():
        raise ValueError("imported h11 is not from expected checkout")
    cases = corpus()
    rows = []
    for case in cases:
        emitted = encode(case)
        row = {"id": case["id"],
               "writes": [chunk.hex() for chunk in emitted]}
        if role == "baseline":
            raw = b"".join(emitted) + SUFFIX
            observations = [read_once(raw, segment) for segment in SEGMENTS]
            expected = {
                "payload": b"".join(bytes.fromhex(x) for x in case["payloads"]).hex(),
                "trailers": case["trailers"],
                "unread_suffix": SUFFIX.hex(),
            }
            if any(observation != expected for observation in observations):
                raise ValueError(f"baseline writer/reader failed corpus case {case['id']}")
            row["downstream"] = observations
        rows.append(row)
    return {
        "schema_version": 1,
        "role": role,
        "corpus_sha256": digest(cases),
        "segments": list(SEGMENTS) if role == "baseline" else [],
        "rows": rows,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--role", choices=("baseline", "candidate"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    a = parser.parse_args()
    a.output.write_text(json.dumps(observe(a.root, a.role),
                                   sort_keys=True, indent=2) + "\n")
    print("OBSERVED", a.role, len(corpus()), "traces", flush=True)


if __name__ == "__main__":
    main()
