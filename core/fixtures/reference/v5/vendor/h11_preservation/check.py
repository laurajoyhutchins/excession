"""Check a bounded behavioral refinement witness and carry a downstream claim.

If every write callback invocation is identical on the explicitly enumerated
input corpus, the pinned deterministic reader receives identical bytes. The
reader is not rerun against the modified writer. This is a finite-domain
composition rule, not a general proof of Python observational equivalence.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
from pathlib import Path
import subprocess
from typing import Any

from domain import corpus, digest
from probe import SEGMENTS, SUFFIX


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args],
                                   text=True).strip()


def source(repo: Path, ref: str, path: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), "show", f"{ref}:{path}"]).decode("utf-8")


def method_only_change(baseline: str, candidate: str) -> bool:
    def erase_method(code: str) -> tuple[str, str]:
        tree = ast.parse(code)
        matches = [
            member for cls in tree.body if isinstance(cls, ast.ClassDef)
            and cls.name == "ChunkedWriter"
            for member in cls.body if isinstance(member, ast.FunctionDef)
            and member.name == "send_data"
        ]
        if len(matches) != 1:
            raise ValueError("expected exactly one ChunkedWriter.send_data")
        method = matches[0]
        raw_method = ast.dump(method, include_attributes=False)
        method.body = [ast.Pass()]
        return ast.dump(tree, include_attributes=False), raw_method
    old_elsewhere, old_method = erase_method(baseline)
    new_elsewhere, new_method = erase_method(candidate)
    return old_elsewhere == new_elsewhere and old_method != new_method


def source_obligations(repo: Path, original: str, changed: str) -> dict:
    if git(repo, "rev-parse", changed + "^") != original:
        raise ValueError("candidate must be a direct Git descendant of original")
    files = git(repo, "diff", "--name-only", original, changed).splitlines()
    if files != ["h11/_writers.py"]:
        raise ValueError("unexpected changes outside ChunkedWriter source: " + str(files))
    old = source(repo, original, "h11/_writers.py")
    new = source(repo, changed, "h11/_writers.py")
    if not method_only_change(old, new):
        raise ValueError("change is not isolated to ChunkedWriter.send_data")
    baseline_tree = git(repo, "rev-parse", original + "^{tree}")
    candidate_tree = git(repo, "rev-parse", changed + "^{tree}")
    return {
        "baseline_commit": original,
        "candidate_commit": changed,
        "baseline_tree": baseline_tree,
        "candidate_tree": candidate_tree,
        "changed_paths": files,
        "writer_body_changed": True,
        "reader_and_its_module_initializers_unchanged": True,
        "source_scope": "Git-tracked files only; only send_data AST differs",
    }


def valid_hex(value: Any) -> bool:
    if not isinstance(value, str) or len(value) % 2:
        return False
    try:
        bytes.fromhex(value)
    except ValueError:
        return False
    return True


def verify_rows(old: dict, new: dict, cases: list[dict]) -> list[dict]:
    expected_ids = [c["id"] for c in cases]
    if (old.get("schema_version") != 1 or new.get("schema_version") != 1
        or old.get("role") != "baseline" or new.get("role") != "candidate"
        or old.get("corpus_sha256") != digest(cases)
        or new.get("corpus_sha256") != digest(cases)
        or old.get("segments") != list(SEGMENTS)
        or new.get("segments") != []):
        raise ValueError("unrecognized or mismatched finite-domain observation metadata")
    baseline = old.get("rows")
    candidate = new.get("rows")
    if (not isinstance(baseline, list) or not isinstance(candidate, list)
        or [r.get("id") for r in baseline] != expected_ids
        or [r.get("id") for r in candidate] != expected_ids):
        raise ValueError("missing, duplicate, or reordered observation cases")
    checked: list[dict] = []
    for case, old_row, new_row in zip(cases, baseline, candidate):
        if (set(old_row) != {"id", "writes", "downstream"}
            or set(new_row) != {"id", "writes"}
            or not isinstance(old_row["writes"], list)
            or not isinstance(new_row["writes"], list)
            or not all(valid_hex(v) for v in old_row["writes"] + new_row["writes"])
            or old_row["writes"] != new_row["writes"]):
            raise ValueError("writer callback trace differs or is malformed: " + case["id"])
        expected = {
            "payload": "".join(case["payloads"]),
            "trailers": case["trailers"],
            "unread_suffix": SUFFIX.hex(),
        }
        if (not isinstance(old_row["downstream"], list)
            or len(old_row["downstream"]) != len(SEGMENTS)
            or any(observation != expected for observation in old_row["downstream"])):
            raise ValueError("baseline downstream contract fails: " + case["id"])
        checked.append({
            "case": case["id"], "writer_trace_sha256": digest(old_row["writes"]),
            "baseline_reader_satisfied": True,
            "candidate_writer_equal": True,
            "composition_retained_by_congruence": True,
        })
    return checked


def make_certificate(baseline: Path, candidate: Path,
                     old: dict, new: dict) -> dict:
    cases = corpus()
    orig_commit = git(baseline, "rev-parse", "HEAD")
    candidate_commit = git(candidate, "rev-parse", "HEAD")
    if (git(baseline, "status", "--porcelain") or
        git(candidate, "status", "--porcelain")):
        raise ValueError("checkout is not clean; cannot bind observed code to Git tree")
    source_evidence = source_obligations(candidate, orig_commit, candidate_commit)
    checked = verify_rows(old, new, cases)
    return {
        "schema_version": 1,
        "kind": "bounded-contract-preservation",
        "scope": {
            "corpus_sha256": digest(cases), "case_count": len(cases),
            "reader_segment_sizes": list(SEGMENTS),
            "callback": "nonthrowing append to an ordered list of bytes",
            "inputs": "valid Data sequences followed by EndOfMessage; finite corpus",
            "output_equality": "exact ordered write callback trace, not only concatenated bytes",
            "excluded": [
                "arbitrary lengths and bytes beyond the finite corpus",
                "side-effecting or failing write callbacks",
                "general Python contextual or source-level equivalence",
                "untracked runtime influences and different interpreter environments",
            ],
        },
        "source": source_evidence,
        "observations": {
            "baseline_sha256": digest(old),
            "candidate_sha256": digest(new),
        },
        "claims": [
            {
                "id": "chunked.writer.bounded",
                "status": "FRESHLY_VERIFIED_WITHIN_CORPUS",
                "rule": "all candidate write callback traces equal baseline for each listed input",
            },
            {
                "id": "chunked.reader.bounded",
                "status": "CARRIED_BY_SOURCE_IDENTITY_AND_RESTRICTED_AST_CHANGE",
                "rule": "reader implementation and initialization code are unchanged; no candidate reader run",
            },
            {
                "id": "chunked.composition.bounded",
                "status": "CARRIED_BY_FINITE_TRACE_CONGRUENCE",
                "premises": [
                    "chunked.writer.bounded",
                    "chunked.reader.bounded",
                    "baseline reader met its contract on every same input",
                ],
                "rule": "equal writer output trace yields equal byte stream to pinned deterministic reader",
            },
        ],
        "cases": checked,
        "trust_boundary": (
            "Machine-checked finite execution observations with source identity "
            "and a syntactic-change guard. Reader determinism/context isolation "
            "and Python execution semantics are assumptions, not formal proofs."
        ),
    }


def strict_json(path: Path) -> dict:
    def unique(pairs: list[tuple[str, Any]]) -> dict:
        d = {}
        for name, value in pairs:
            if name in d:
                raise ValueError("duplicate JSON key")
            d[name] = value
        return d
    data = json.loads(path.read_text(), object_pairs_hook=unique)
    if not isinstance(data, dict):
        raise ValueError("evidence root must be an object")
    return data


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--baseline", type=Path, required=True)
    p.add_argument("--candidate", type=Path, required=True)
    p.add_argument("--baseline-observation", type=Path, required=True)
    p.add_argument("--candidate-observation", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    certificate = make_certificate(
        a.baseline, a.candidate, strict_json(a.baseline_observation),
        strict_json(a.candidate_observation))
    a.output.write_text(json.dumps(certificate, sort_keys=True, indent=2) + "\n")
    print("PRESERVED", certificate["scope"]["case_count"],
          "writer traces; 2 downstream claims carried without candidate reader tests")


if __name__ == "__main__":
    main()
