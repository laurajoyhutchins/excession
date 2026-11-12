"""Independent observation oracle for real SCIP indexer outputs.

This reads official 'scip print --json' data directly. It does not use
Excession's SCIP decoder, normalizer, compiler, or semantic query code.
It checks that the projection preserves **unique-definition cross-file SCIP
references**, never converts them to runtime/import facts, and only proposes
concepts for observed reciprocal pairs. It cannot establish the correctness
or completeness of the upstream indexer.
"""
from __future__ import annotations

from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import subprocess
from typing import Any


def field(obj: dict, snake: str, camel: str | None = None, default: Any = None) -> Any:
    return obj.get(snake, obj.get(camel, default) if camel else default)


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=True, allow_nan=False).encode("utf-8")


def sha256(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def verified_content(value: dict, id_key: str) -> bool:
    return value.get(id_key) == sha256({key: item for key, item in value.items() if key != id_key})


def line_from_occurrence(occ: dict) -> int:
    raw = occ.get("range")
    if isinstance(raw, list):
        if len(raw) not in (3, 4) or not all(isinstance(x, int) and x >= 0 for x in raw):
            raise ValueError("invalid SCIP occurrence range")
        return raw[0] + 1
    typed = field(occ, "TypedRange", "typed_range", occ)
    single = field(typed, "SingleLineRange", "singleLineRange",
                   typed.get("single_line_range"))
    multi = field(typed, "MultiLineRange", "multiLineRange",
                  typed.get("multi_line_range"))
    if (single is None) == (multi is None):
        raise ValueError("unsupported or ambiguous typed SCIP range")
    value = field(single, "line") if single is not None else field(multi, "start_line", "startLine")
    if not isinstance(value, int) or value < 0:
        raise ValueError("invalid SCIP line")
    return value + 1


def native_references(
    decoded: dict, source_prefix: str | None = None,
) -> tuple[set[tuple[str, str, str, int]], dict[str, dict], dict]:
    """Reference oracle from raw native occurrences and **one** definition occurrence.

    Path-to-path relations here are *file association* only, never a call edge.
    Multiple definitions in the same file remain ambiguous.
    """
    definitions: dict[str, list[str]] = defaultdict(list)
    references: list[tuple[str, str, int]] = []
    docs: dict[str, dict] = {}
    occurrence_count = 0
    for doc in decoded.get("documents", []):
        relative = field(doc, "relative_path", "relativePath")
        if not isinstance(relative, str) or not relative:
            raise ValueError("invalid native SCIP source path")
        path = f"{source_prefix}/{relative}" if source_prefix else relative
        if path in docs:
            raise ValueError("duplicate/invalid SCIP document identity")
        language = doc.get("language") or "unspecified"
        docs[path] = {"path": path, "language": language}
        for occ in doc.get("occurrences", []):
            occurrence_count += 1
            symbol = occ.get("symbol", "")
            if not symbol:
                continue
            if not isinstance(symbol, str):
                raise ValueError("invalid SCIP symbol identity")
            role = field(occ, "symbol_roles", "symbolRoles", 0)
            if not isinstance(role, int) or role < 0:
                raise ValueError("invalid SCIP occurrence role")
            # Identical to the native role filter contract, but independently
            # parsed from the original decoded index instead of the Rust report.
            if symbol.startswith("local ") or role & 16:
                continue
            line = line_from_occurrence(occ)
            if role & 1:
                definitions[symbol].append(path)
            elif not role & 64:
                references.append((path, symbol, line))
    matched = {
        (source, definitions[symbol][0], symbol, line)
        for source, symbol, line in references
        if len(definitions.get(symbol, [])) == 1
        and source != definitions[symbol][0]
    }
    unresolved = sum(len(definitions.get(symbol, [])) != 1
                     for _, symbol, _ in references)
    return matched, docs, {"indexed_occurrences": occurrence_count,
                           "unresolved_references": unresolved}


def git_blobs(repo: Path, commit: str) -> dict[str, str]:
    result = subprocess.run(
        ["git", "-C", str(repo), "ls-tree", "-rz", commit],
        capture_output=True, check=True, timeout=30,
    ).stdout
    blobs = {}
    for part in result.split(b"\0"):
        if not part:
            continue
        header, name = part.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split()
        if kind == "blob" and mode in {"100644", "100755"}:
            blobs[name.decode("utf-8")] = oid
    return blobs


def reconcile(
    decoded: dict, batch: dict, model: dict, manifest: dict, audit: dict,
    repo: Path, commit: str, tree: str,
    source_prefix: str | None = None,
) -> dict:
    """Fail closed on disagreement between independent native and semantic views."""
    for item, key in [(batch, "batch_id"), (model, "model_id"),
                      (manifest, "snapshot_id")]:
        if not verified_content(item, key):
            raise ValueError(f"noncanonical semantic {key}")
    if (batch["source_commit"] != commit or batch["source_tree"] != tree or
            model["source_commit"] != commit or model["source_tree"] != tree or
            manifest["source_commit"] != commit or manifest["source_tree"] != tree or
            manifest["model_id"] != model["model_id"] or
            manifest["input_batch_ids"] != [batch["batch_id"]] or
            audit.get("snapshot_id") != manifest["snapshot_id"] or
            audit.get("status") != "SOURCE_BOUND_SCIP_INDEX_REPLAYED" or
            audit.get("indexer_independently_executed") is not False):
        raise ValueError("semantic snapshot or audit does not match pinned SCIP input")
    if (batch.get("completeness") != "PARTIAL_CANDIDATES_ONLY" or
            model["coverage"]["status"] != "PARTIAL" or
            model["coverage"]["producer_receipts"] !=
            [batch["inputs"][0]["receipt_id"]] or
            batch["inputs"][0]["kind"] != "SCIP"):
        raise ValueError("upstream native references incorrectly promoted to complete coverage")
    edges, docs, native_coverage = native_references(decoded, source_prefix)
    blobs = git_blobs(repo, commit)
    docs_expected = {
        f"{source_prefix}/{field(x, 'relative_path', 'relativePath')}"
        if source_prefix else field(x, "relative_path", "relativePath")
        for x in decoded["documents"]
    }
    if set(docs) != docs_expected:
        raise ValueError("SCIP document inventory changed during independent replay")
    for path in docs:
        if path not in blobs:
            raise ValueError(f"untracked SCIP document in independent oracle: {path}")
    subjects = {}
    for subject in batch["subjects"]:
        if not verified_content(subject, "subject_id"):
            raise ValueError("noncanonical semantic subject identity")
        key = subject["subject_id"]
        if key in subjects or subject["path"] not in blobs or subject["blob"] != blobs[subject["path"]]:
            raise ValueError("duplicate or stale semantic subject")
        if subject["source_commit"] != commit or subject["kind"] != "SOURCE_FILE":
            raise ValueError("semantic subject changed its native Git meaning")
        subjects[key] = subject

    actual_edges: set[tuple[str, str, str, int]] = set()
    observation_index: dict[str, tuple[str, str, str, int]] = {}
    for obs in batch["observations"]:
        if not verified_content(obs, "observation_id"):
            raise ValueError("noncanonical native observation identity")
        if (obs["kind"] != "SCIP_REFERENCE" or
                obs["native_relation"] != "SCIP_SYMBOL_REFERENCE" or
                obs["status"] != "OBSERVED" or
                obs["condition_evaluation"] != "NOT_EVALUATED" or
                obs["receipt_id"] != batch["inputs"][0]["receipt_id"]):
            raise ValueError("SCIP observation was promoted or misclassified")
        a, b = subjects[obs["source_subject_id"]], subjects[obs["target_subject_id"]]
        if a.get("span", {}).get("encoding") != "LINE_ONLY":
            raise ValueError("SCIP origin must retain line-only source span")
        edge = (a["path"], b["path"], obs["specifier"], a["span"]["start_line"])
        if obs["source_observation_id"] in observation_index:
            raise ValueError("repeated upstream native observation identity")
        observation_index[obs["source_observation_id"]] = edge
        actual_edges.add(edge)
    if actual_edges != edges or len(observation_index) != len(edges):
        missing = sorted(edges - actual_edges)[:5]
        invented = sorted(actual_edges - edges)[:5]
        raise ValueError(f"SCIP projection disagrees with native occurrences: missing={missing}; invented={invented}")

    oriented = {(source, destination) for source, destination, _, _ in edges}
    pairs = {
        tuple(sorted((source, destination)))
        for source, destination in oriented
        if (destination, source) in oriented
    }
    concept_pairs = set()
    claims_by_id = {claim["claim_id"]: claim for claim in model["claims"]}
    if len(claims_by_id) != len(model["claims"]):
        raise ValueError("duplicate semantic claims")
    for c in model["concepts"]:
        if c["review_status"] != "UNREVIEWED":
            raise ValueError("indexer source alone established review")
        member_ids = c["membership_claims"]
        if len(member_ids) != 2:
            raise ValueError("native proposal expected exactly two source memberships")
        paths = []
        for claim_id in member_ids:
            claim = claims_by_id[claim_id]
            if claim["predicate"] != "REALIZES_CONCEPT" or claim["concept_id"] != c["concept_id"]:
                raise ValueError("non-native concept membership predicate")
            paths.append(subjects[claim["subjects"][0]]["path"])
        pair = tuple(sorted(paths))
        if pair in concept_pairs:
            raise ValueError("duplicate native concept pair")
        concept_pairs.add(pair)
    if concept_pairs != pairs or len(model["claims"]) != 3 * len(pairs):
        raise ValueError("architectural hypothesis does not match reciprocal native references")
    for claim in model["claims"]:
        if claim["assessment"] != "PROPOSED" or claim["behavioral_verification"] != "UNTESTED":
            raise ValueError("SCIP candidate promoted beyond native evidence")
        for support in claim["support"]:
            if support["observation_kind"] != "SCIP_REFERENCE" or support["role"] != "SUPPORTS":
                raise ValueError("SCIP support was reclassified as runtime or verifier")
            if support["observation_id"] not in observation_index:
                raise ValueError("concept support invents native SCIP occurrence")
    declared = batch["gaps"]
    if not any(g["reason"] == "PARTIAL_CONFIGURATION" for g in declared):
        raise ValueError("SCIP source scope incorrectly described as complete")
    if native_coverage["unresolved_references"] and not any(
        g["reason"] == "UNRESOLVED_SYMBOL" for g in declared
    ):
        raise ValueError("missing unresolved native symbol coverage")
    if len(model["subjects"]) != len(batch["subjects"]):
        raise ValueError("compiler discarded source binding identities")

    return {
        "schema": "excession.concepts-native-scip-oracle.v1",
        "source_commit": commit,
        "source_tree": tree,
        "indexer_receipt": batch["inputs"][0]["receipt_id"],
        "source_bound_observation_count": len(actual_edges),
        "independent_native_reference_count": len(edges),
        "unresolved_native_references": native_coverage["unresolved_references"],
        "independently_reconstructed_reciprocal_file_pairs": len(pairs),
        "proposed_semantic_concept_count": len(model["concepts"]),
        "status": "SOURCE_BOUND_NATIVE_INTERPRETATION_AGREES",
        "indexer_semantic_correctness_independently_verified": False,
        "indexer_completeness_verified": False,
    }
