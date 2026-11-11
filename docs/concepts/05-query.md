# PR 5: Evidence-aware query and explanation

Status: JSON Schema and TDD specification only. No query engine or CLI behavior is implemented.

## Request protocol

A query identifies an immutable semantic snapshot, operation, selector, optional required evidence classes, and explicit depth/result limits. Parsing arbitrary natural-language text into SQL is not a core authority and is outside scope.

## Response protocol

The answer must echo the selected query/snapshot, pinned source commit, answerability status, supporting claim/evidence IDs, gaps and truncation status. Multi-hop traversals preserve predicate types and supporting evidence per hop. A digest is not an attestation. `ANSWERABLE` refers only to stated query scope and producer coverage.

Semantics of statuses:
- ANSWERABLE: requested evidence classes sufficiently observed within declared scope.
- PARTIAL: usable evidence with significant gaps.
- AMBIGUOUS: incompatible interpretations remain.
- UNKNOWN: insufficient evidence to decide.
- UNSUPPORTED: request requires semantics producers do not supply.

## TDD obligations

1. Positive exact symbol queries cite SCIP evidence, not a generic dependency edge.
2. Query with absent SCIP index returns UNKNOWN/UNSUPPORTED, never an empty complete answer.
3. Partial configuration yields PARTIAL, even with zero reported parser gaps.
4. Ambiguous concept names produce AMBIGUOUS with candidate identifiers.
5. Forged/missing evidence, stale snapshot, unsupported predicate and excessive traversal are refused.
6. Reports from the same snapshot/query are byte-identical and independently replayable.

Extend existing `excession explain` / issues #81 and #82 rather than introducing another SQL or parser authority.
