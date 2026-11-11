# PR 3: Immutable semantic snapshot storage

Status: specification, schema, and TDD only. No SQLite migration in this PR.

## Storage contract

Reuse `core/src/sqlite.rs`, the repository/revision/tree-entry authority and the existing receipt store. Add additive semantic tables only when implemented: snapshot, subject, concept, concept_revision, membership, claim, support, coverage, review_event.

A snapshot is an immutable, canonical export of one model together with exact input receipt IDs, compiler/rule identity and source revision. File paths and timestamps must not alter canonical IDs; publication metadata belongs outside the hashed content. Readback must reconstruct the full model and all claim/support memberships, not just counts.

## TDD acceptance

- Atomic import and rollback for a failed cross-reference.
- Identical import twice has identical rows and snapshot identity.
- Replace/import cannot retain obsolete memberships or support links.
- Altered claim, forged extra SQL row, missing evidence, stale source revision, and duplicate IDs are independently refused.
- Closing and reopening SQLite preserves byte-identical canonical export.
- Legacy SQLite/evidence tables and previous reports are untouched.

No migration should be merged without exact-head Rust integration tests. See the NVIDIA auto-ontology replacement-import failure mode as a negative test design reference, not a dependency.
