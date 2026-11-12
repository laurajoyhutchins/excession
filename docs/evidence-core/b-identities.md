# Evidence identities and receipt integrity (PR #117)

**Status:** Rust identity primitives implemented and isolated local unit tests executed. Upstream SCIP normalization, end-to-end crate/CLI integration, and semantic verification remain **pending**. This PR remains draft.

## Current implementation

- `core/src/evidence/identity.rs`: typed `SourceRef`, `Subject`, `Span`, `ProducerReceipt` and provenance declarations.
- `core/src/git.rs::entry_at`: exact Git tree-entry lookup, avoiding a whole-repository inventory on each source check.
- `core/src/lib.rs`: exports `evidence::identity`.
- `contracts/evidence-core/identity-receipt.v1.schema.json`: discriminated subject types and provenance requirements.
- `tests/test_ec_b_identities.py`: structural source/producer wire contract tests. The redundant internal ODCS catalogue and its metadata-only tests have been removed; Rust validates the identity invariants.

## Authority boundaries

A source subject is Git-bound only after `SourceRef::verify_git` checks commit, tree, blob, mode and exact path. `SourceRef::validate` is shape validation only.

Artifact, execution and configuration subjects are independent of Git source. They must never receive fabricated `source` fields merely to satisfy a common data model.

SCIP symbol identities are preserved verbatim and remain tied to their declared source. Matching display names or symbol spellings is **not** cross-revision identity or behavioral equivalence.

`ProducerReceipt::verify_payload` checks only the original byte digest. `ExternalAttestationClaim` retains a claimed attestation digest but does **not** authenticate the attesting worker. Likewise, `SourceReplayed` does not imply independently verified analyzer semantics.

Content identities use SHA-256 with domain-separated, length-prefixed fields and distinct kind discriminators. They are stable for exact typed descriptions, not proof that two independently produced subjects are semantically identical.

## Tests

Structural contract tests exercise five subject variants, source path/mode validation, invalid symbols, missing/extra properties, and attestation-claim requirements. Rust unit tests exercise domain-separated identity, digest tampering and real temporary Git source replay (including forged tree/blob/mode).

Rust 1.99.1 was restored from the user's pinned verified Library archive to a local VM. An isolated Rust test harness exercised the identity implementation against the real Git binary: **5 passed**. The new contract's Python suite executed locally: **6 passed, 1 skipped** for upstream SCIP integration. This harness is not a full `cargo test` of Excession's head.

## Required before merging

1. Compile and run complete `cargo test`, `cargo fmt --check`, and Clippy against this exact PR head.
2. Execute the real maintained upstream SCIP fixture, independently replay and compare source identities and original producer receipts.
3. Check legacy `semantic-observations.v1` and `evidence-query.v1` byte/semantic compatibility.
4. Establish integration with existing concepts #88-#95 and product #101 without another evidence authority.

## Explicit non-goals

No SQLite persistence, evidence coverage completion, security validity promotion, REA decompilation, new CLI grammar or restoration of `excession research`.
