# Immutable evidence persistence and complete-row integrity audit

**Status:** Rust/SQLite implementation and adversarial unit tests committed to draft [#118](https://github.com/laurajoyhutchins/excession/pull/118). Exact-head verification is required before acceptance. Real upstream SCIP ingestion and Git source authentication are separate pending integration work.

## Scope

`core/src/evidence_store.rs` extends Excession's existing `sqlite::Db` with additive `evidence_snapshot_v1`, `evidence_receipt_v1` and `evidence_observation_v1` tables. It does **not** create a second graph/database authority or replace `configured_store.rs`.

A batch has one syntactically validated SHA-1 Git source revision, a set of original UTF-8 JSON producer receipts, and normalized observations referencing exact receipt identities. The native relation kind is preserved without promotion.

- All records and manifest identities are domain-separated SHA-256 hashes of canonical Rust JSON.
- Raw receipt strings retain their exact original JSON representation, including whitespace, and a separate raw-payload SHA-256.
- The persisted snapshot manifest enumerates the complete, sorted set of receipt and observation identifiers.
- Every normalized observation must reference a receipt in the same snapshot.
- Duplicate input rows and missing references are refused before publishing.
- Re-import is idempotent only when the existing snapshot independently passes a full readback audit. It must never silently repair missing or forged rows.
- SQLite import is transactional. An injected failure rolls back the entire attempted update.
- Audit checks snapshot/membership counts, every row's canonical content digest, exact receipt links, native kind and raw-payload integrity. It also runs SQLite foreign-key checks.
- Imports are bounded to 4,096 receipts, 20,000 observations, 4 MiB per original receipt and 512 KiB per normalized observation.

## Explicit limitations

**Integrity is not semantic validity.** The manifest explicitly sets `source_authentication = NOT_CHECKED`. Its SHA-1 revision is shape-validated but not independently replayed against Git. The raw receipt is producer-supplied, not an attested execution. A successful SQLite audit proves the retained rows and their provenance references are internally consistent and complete *within this snapshot*, not that a scanner finding or SCIP edge is true.

Binary/non-JSON evidence, independent artifact binding, predicate-specific verification, cross-snapshot correspondence and the public CLI query path are not implemented by this PR. Coordinate subsequent work with #117, native normalization #89, semantic snapshots #90 and #91, and public #81.

## Contracts

- `contracts/evidence-core/persistence.v1.schema.json`: JSON Schema Draft 2020-12 for the manifest actually emitted by the Rust implementation.
- `contracts/evidence-core/persistence.v1.odcs.json`: quality/limitations metadata; **not** an independent verification authority.
- `tests/test_ec_c_persistence.py`: positive/negative structural contract tests.
- `tests/test_ec_odcs_c_persistence.py`: pinned ODCS schema and dataset field alignment checks.
- `core/src/evidence_store_tests.rs`: Rust idempotency, reopen, rollback and adversarial SQL tampering tests.

## Completion requirements

1. Rust `cargo fmt --check`, unit and integration tests, and contract/ODCS suites pass against exact head.
2. Re-run a real native SCIP producer and independently authenticate the exact Git source and original producer receipts in #117/#89, then import that evidence using this store.
3. Exercise source-change invalidation, fail-closed query and independent semantic/behavioral oracles without implying completeness.
4. Preserve existing `semantic-observations.v1`, `evidence-query.v1` and concept/product report semantics. Keep the `excession research` entry point retired.

The remaining independent-oracle test is intentionally pending; do not treat a successful shape audit or database readback as native semantic verification.
