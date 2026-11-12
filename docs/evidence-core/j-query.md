# contracts: evidence-qualified inspect, explain, impact and view integration

**Status:** schema, documentation, negative-first structural tests only. The behavior remains **not implemented**.

**Issue coordination:** #80, #81, #82, #102 and #104; align drafts #92 and #94. **Base:** current `main`; do not depend on unmerged concept branches.

## User-facing requirement
One bounded typed read-only query/answer contract for all producer facts and derived explanations.

## Normative rules
- Extend existing inspect/explain/impact/view commands and product result schema rather than introducing a parallel query engine.
- Every reported path retains its producer native predicate, source identity and per-hop evidence, and every gap remains visible.
- No query result may authorize test skipping, refactoring safety or CVE validity based on partial evidence.

## Schema and TDD
- `contracts/evidence-core/investigation.v1.schema.json`: Draft 2020-12 initial wire shape.
- `python3 -m unittest discover -s tests -p 'test_ec_j_query.py' -v`: positive/negative structural tests using existing `jsonschema` dependency.
- Behavior test intentionally skipped; a passing structural suite is **not** feature acceptance.
- Completion: Implement and replay a multi-hop evidence explanation in h11 plus an unknown coverage result, using existing Rust CLI grammar.

## Existing interfaces
Reuse `configured_store.rs`, `semantic-observations.v1`, `evidence-query.v1`, existing SCIP/native/scanner adapters, and the single Rust CLI. The semantic concept PRs #88–#95 and product #101 remain separate derived/presentation layers. Do not restore `excession research`.

No CI workflow, additional database, new parser, decompiler, remote service or security-verdict engine is introduced in this draft.
