# Evaluate understanding and refactoring, not index size

**Primary question:** Does Excession help an unfamiliar agent correctly
understand and improve real software, with transparent uncertainty, at a
lower *total* cost than ordinary repository tools?

## Study design

Use unfamiliar, externally maintained repositories at exact public commits.
Pre-register questions, inclusion criteria, source revisions, available
tools, cost limits, and independent answer keys before running Excession.
Create two agent arms: the same agent/tool budget with ordinary Git search
and language tooling, and with Excession's available evidence interface.
Randomize order and prevent answer-key leakage into either arm.

Questions must cover governing implementations, cross-component behavior,
control/data flow, failure paths, test ownership, conflicting hypotheses,
change impact and refactoring-preservation obligations.

| Dimension | Measure |
| --- | --- |
| Understanding | Reviewer-scored behavioral correctness against an independent source/execution answer |
| Discovery | Whether the actual governing implementation and critical tests were found |
| Evidence discipline | Accurate source citations, supported claims, calibrated unknowns, counterexample handling |
| Transformation | Independently judged structural improvement and preserved specified behavior |
| Failure containment | False certainty, incorrect rewrite, skipped relevant tests and unnoticed regressions |
| Total cost | Indexed/preprocessing cost, source bytes inspected, tool calls, wall time, peak memory, test/build cost |

Never reward a confident incorrect answer. A smaller correct model with
explicit gaps should outperform an exhaustive but misleading graph.

## First pilot: SQLite

Choose a pinned SQLite commit and independently reviewed questions about
commit/durability paths, mode-dependent behavior, error/recovery boundaries,
and tests. Also choose one scoped structural refactoring task with a
reviewed preservation contract. The answers and pass/fail gates must be
established *before* Excession runs; do not derive the oracle from its output.

Record all assumptions and negative results, including cases where direct
source inspection beats Excession. Current SQLite study notes or a high SCIP
recall result do **not** establish completed behavioral understanding.

## Separate diagnostic benchmarks

Retain existing relation-recall, parser, SCIP and speed experiments as
**evidence-provider diagnostics**, not primary product success metrics.
Existing benchmark command names, JSON schemas and dated records remain
valid within their stated scope. Do not delete them or reinterpret their
old scores as task-completion results.

See [benchmark inventory](../benchmarks/README.md),
[protocol](../benchmarks/protocol.md),
[codebase understanding](CODEBASE_UNDERSTANDING.md), and
[product contract](PRODUCT.md).

## Reproducibility

Publish exact Git and tool identities, index/configuration scope, the
agent's context and accessible tools, all independent receipts,
raw evaluations, adverse outcomes and resource costs. Choose quantitative
acceptance thresholds before evaluating and report uncertainty intervals
when multiple cases are aggregated. One successful demonstration is not
proof of general productivity improvement.
