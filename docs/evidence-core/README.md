# Evidence-core implementation map

**Status:** draft documentation, contracts and structural TDD. Nothing in this series by itself establishes implemented behavior, source authenticity, or semantic/security validity.

## Authority boundaries
- Git objects are the source authority; the current implementation supports SHA-1 Git object repositories.
- Rust/SQLite owns relationships, evidence indexing and audit state. Reuse `core/src/configured_store.rs`, `core/src/kernel_coverage.rs`, `core/src/evidence_query.rs` and existing scanners/indexers.
- Native analyzers own their predicates. A SCIP reference, compiler declaration, runtime witness and CodeQL finding remain distinguishable.
- Concepts are derived/reviewable interpretations. `contracts/concepts/*` is not a second evidence or graph authority.
- Structural JSON Schema/ODCS validation never proves source replay, producer correctness, complete scope, or behavioral validity.
- Keep `excession research` retired.

## Independent draft PRs targeting main
| Cut | PR | Contract | Blocked behavioral goal |
| --- | --- | --- | --- |
| A | #116 | boundary.v1 | Reconcile with concept #88 and product #101 before promoting shared definitions |
| B | #117 | identity-receipt.v1 | Git/SCIP native identity, producer receipt replay |
| C | #118 | persistence.v1 | Idempotent transactions, audit of all SQL rows |
| D | #119 | coverage.v1 | Scoped inventory and sound negative-evidence rules |
| E | #120 | verification.v1 | Predicate-specific verification and prohibited promotions |
| F | #121 | unknown-event.v1 | Append-only unknown revisions, stale-write rejection |
| G | #122 | comparison.v1 | Correspondence and facet-qualified revision comparison |
| H | #123 | build-provenance.v1 | Observed build inputs and artifact SHA-256 identity |
| I | #124 | rea-import.v1 | Real REA Evidence export import, native IDs and raw bytes |
| J | #125 | investigation.v1 | Reuse existing inspect/explain/impact/query paths |
| K | #126 | acceptance.v1 | Independent real-source/binary/REA, correctness and cost results |

Each draft includes a closed Draft 2020-12 schema, ODCS 3.2 dataset-quality metadata, documentation, positive/negative structural tests, and an explicitly skipped *future* behavioral acceptance test. These are **not** eleven competing feature implementations or a new semantic ontology stack.

## Existing ownership to preserve
- #60 reconciles source/indexer/history/CLI branches and prevents duplicate authorities.
- #96 and drafts #88-#95 own semantic concepts, normalization, snapshots, deterministic compilation, queries, review, enrichment and independent evaluation.
- #79-#82 and product PR #101 own public analyze/impact/explain/CLI result behavior. Do not duplicate these command names.
- #84 is the existing independent release gate; #105 is semantic intelligence's independent evaluation. PR K should extend both, not create another CI harness.
- Security validity PR #87 owns independent behavioral ground truth; PR E must not relabel CodeQL/Semgrep ingest as a validity proof.

## Dependency order
A -> B -> C -> D -> E; F and G can be developed in parallel after E; H needs C/E; I needs D/H; J integrates the delivered primitives; K establishes independent acceptance. Drafts are based on the current main rather than inheriting the old #88-#95 or product branch ancestry. Rebase against freshly merged main as each cut lands.

## Merge criteria
No draft may be marked complete based solely on schema tests. Require unskipped exact-head Rust/native behavior, adversarial negative controls, independent Git/producer evidence, compatibility with legacy receipts and measured bounded cost. Preserve unsupported cases in the denominator; never use partial coverage to authorize test omission.
