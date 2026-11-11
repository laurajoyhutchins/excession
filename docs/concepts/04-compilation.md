# PR 4: Deterministic semantic compilation

Status: specification, compilation receipt schema and TDD scaffold. No inference engine.

## Compilation function

`compile(source_commit, source_tree, observation_batch_ids, rule_set_id, prior_review_revisions?) -> semantic_model + compilation_receipt`

The compiler has no network/API/model dependency. It emits only typed claims permitted by the predicate registry. Use existing SCIP symbol occurrences, structural hypothesis reports and declared scopes. Text or name similarity can propose investigation candidates only, not claim semantic identity. Preserve counterevidence, unresolved cases and native evidence distinctions.

## Determinism and incremental invalidation

Sort inputs canonically; include compiler version, rule set version, producer receipts and reviewed model revisions in computation identity. Recomputing unchanged inputs yields an identical result. Changed source invalidates affected claims; no automatic carry of behavioral proof across revisions.

## Red TDD inventory for implementation

- Identical inputs produce exactly identical canonical bytes and content IDs.
- Different producer order produces identical results after canonical normalization.
- Same-name independent functions do not become merged concepts.
- Two different APIs with identical bodies are *not* automatically behaviorally equivalent.
- Partial source scope cannot yield an unconditional dependency-closure claim.
- Contradictory support remains accessible rather than overwritten.
- Missing receipt, invalid predicate, stale blob or oversized graph fails closed.
- Real h11/Excession SCIP runs are evaluated against independent native reference observations.

Only the schema shape tests are currently executable, not the compiler behavior.
