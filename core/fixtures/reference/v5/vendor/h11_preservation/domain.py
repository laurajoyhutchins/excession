"""Deterministic finite corpus for an h11 chunked-writer contract experiment.

The finite input domain is an explicit boundary of the result, not a proxy
for all possible HTTP bodies, writer callbacks, or Python contexts.
"""
from __future__ import annotations

import hashlib
import json
import random


def corpus() -> list[dict]:
    sizes = (0, 1, 2, 3, 15, 16, 17, 31, 32, 63,
             64, 127, 128, 255, 256, 257, 4095, 4096)
    rng = random.Random(731237)
    cases: list[dict] = []

    def insert(parts: list[bytes], trailers: list[tuple[bytes, bytes]]) -> None:
        n = len(cases)
        cases.append({
            "id": f"trace-{n:03}",
            "payloads": [p.hex() for p in parts],
            "trailers": [[k.hex(), v.hex()] for k, v in trailers],
        })

    for size in sizes:
        payload = bytes((i * 37 + size * 11) % 256 for i in range(size))
        insert([payload], [])
        insert([b"", payload, b""], [(b"x-contract", b"yes")])
        mid = size // 2
        insert([payload[:mid], payload[mid:]],
               [(b"etag", b"verified"), (b"x-id", b"42")])
    for _ in range(54):
        lengths = [rng.randrange(0, 260) for _ in range(rng.randrange(0, 5))]
        chunks = [bytes(rng.randrange(256) for _ in range(n)) for n in lengths]
        insert(chunks, [(b"x-case", b"random")] if rng.randrange(2) else [])
    # These four cases are byte-for-byte the input event fixtures used by
    # research/ci_evidence/test_roundtrip.py in the previous in-toto run.
    # They form a bridge from earlier finite observations to the new writer.
    legacy = (
        ("legacy-empty", (), ()),
        ("legacy-single", (b"a",), ()),
        ("legacy-trailers", (b"abc", b"", b"defghi"), ((b"x-note", b"done"),)),
        ("legacy-binary", (b"\x00\r\n\xff", b"Z" * 100),
         ((b"etag", b"xyz"), (b"x-k", b"v"))),
    )
    for name, payloads, trailers in legacy:
        cases.append({
            "id": name,
            "payloads": [part.hex() for part in payloads],
            "trailers": [[key.hex(), value.hex()] for key, value in trailers],
        })
    assert len(cases) == 112
    return cases


def digest(rows: object) -> str:
    return hashlib.sha256(json.dumps(rows, sort_keys=True,
                                     separators=(",", ":")).encode()).hexdigest()
