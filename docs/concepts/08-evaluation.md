# PR 8: Independent semantic intelligence evaluation

Status: preregistered questions, measurable evaluation protocol and TDD report schema. **No measured results are claimed**. All zero-valued example metrics are clearly marked SYNTHETIC_SCHEMA_FIXTURE.

## Acceptance question

Does adding semantic compilation to existing Excession evidence make refactoring and impact conclusions more useful without unacceptable false merges, cost or evidence inflation?

## Baselines and independent oracles

- Existing `refactor suggest`, `refactor scip-suggest`, and evidence/impact queries serve as baseline. Do not modify them before evaluation.
- Freeze a held-out set of concept membership and relationship questions against real pinned repos *before looking at semantic predictions*.
- Cross-check source relationships against official SCIP/native parser/compiler outputs and independent Git source spans. SQL self-replay is not a semantic oracle.
- Include h11, Excession Rust, and one larger native-indexer-supported repository with explicit configuration. Never label unsupported languages as failures of verified semantics.
- Positive, negative and adversarial fixtures: same-name unrelated functions; cross-language names without bindings; duplicate bodies with differing effects; stale blobs; partial build configurations; intentionally forged candidate; changed review scope.
- Count unsupported and unresolved cases in reported denominators; do not discard them to inflate accuracy.

## Metrics

Precision/recall for independently reviewable in-scope claims, false concept merges, unresolved counts, usefulness of action recommendations, ingest/compile/query/audit wall time, peak RSS and artifact sizes. Record exact tool versions and machine configuration. A faster parser is not a proof of semantic correctness.

## CI split

Fast local tests validate schema shape, negative fixtures, canonical identity and refusal behavior. Actual upstream indexing and complete held-out evaluation are manual/scheduled deep tests, not a new per-commit heavy workflow.

## Release gate

Must show source audit, independently inspected claims, and a useful finding absent from the un-enriched baseline (or explicitly establish no such improvement). Do not mark the feature default solely because this report schema validates. Link to existing release acceptance issue #84.

## Executable evaluator

`tools/evaluate_concept_enrichment.py` performs read-only source-bound reconciliation of a baseline refactor report, concept snapshot/model, annotation report and a separately prepared oracle. Every baseline candidate must be labeled, including `UNJUDGED` and `UNSUPPORTED`, and every oracle source path/blob is checked against the pinned Git revision. It counts warranted links, false links, missing warranted links and unknowns. No source code or baseline reports are mutated.

**Important:** This first executable harness only checks internally consistent evidence and declared reviewer decisions. It cannot prove the reviewer was independent or froze decisions before predictions. Output remains `SYNTHETIC_SCHEMA_FIXTURE` regardless of whether it was run against a real checkout. The runtime/RSS fields measure evaluation overhead only, not end-to-end SCIP indexing, compilation or the full release acceptance benchmark. Independent real-world outcome validity and full cost measurement remain necessary for S8 completion.
