# Semantic intelligence contracts (proposal)

Status: **design only**. These schemas describe future interfaces; no compiler, store, or query command is implemented by this PR.

## Authority and reuse

Git objects bind source identity. SCIP and language-native tools report their own observations. Existing `excession.semantic-observations.v1` remains static candidate-only; `excession.evidence-query.v1` retains separate static/compiler/runtime meanings. Rust/SQLite may materialize projections but cannot invent source facts.

## Normative identities

- `subject_id`: deterministic identity of one exact revision-scoped producer subject, including configuration where relevant. Preserve upstream SCIP symbol strings verbatim.
- `claim_id`: deterministic identity over predicate, operands, scope, inference rule and cited input evidence.
- `concept_id`: stable assigned identity for an architectural concept, *not* a digest of its current name or members.
- `revision_id`: digest of a canonical immutable concept definition, membership and review scope.
- `model_id`: digest of canonical model body excluding the ID field. An unkeyed digest is not a signature.

The schema checks syntactic shape. Rust must check Git bindings, canonical hashes, foreign references, supported predicate operand combinations, review authority and source replay.

## Semantic safety rules

1. A SCIP reference does not entail an import, call, configured-build relation or runtime dependency.
2. A syntactic match, name match, embedding or body fingerprint does not establish semantic identity.
3. `BEHAVIORALLY_EQUIVALENT_UNDER` requires an actual `verification_contract_id` and independent behavior evidence. A human review alone is insufficient.
4. `DECLARED_SCOPE_COMPLETE` means only that the producer's *declared scope* has been accounted for, never global runtime completeness.
5. Missing observations are unknown, not proof of absence.
6. Contradictory support is retained and visible.
7. Concept aliases and cross-revision continuity require explicit scoped justification.

## Deliverables in subsequent draft PRs

Observation normalization, immutable snapshot persistence, deterministic compilation, typed answerability, review event replay, downstream analysis and independent evaluation.

## Initial TDD contract

`python3 -m unittest tests.test_concepts_contracts -v` validates the draft schema and negative fixtures. Behavioral TDD cases in downstream PRs remain explicitly skipped until the corresponding Rust implementation exists. Do not count skipped tests as established functionality.

- [Temporal change-companion hypotheses](11-temporal-invariants.md): pinned historical associations, heldout predictions, independent popularity comparator (never verified behavior).
