# Public CLI and evidence boundaries

The Rust CLI emits versioned, operation-specific reports for discovery,
analysis, impact, explanation, audit, and agent work. The generic
`cli-result.v1` proposal was not an emitted response and is removed.
Preserve each existing implemented report's wire semantics until a deliberate
versioned migration is verified.

- Git commit/tree/path/mode/blob identify source. JSON shape is not a Git audit.
- Evidence retains its native producer and assessment. Static candidates,
  SCIP references, compiler-declared inputs and runtime witnesses are distinct.
- Coverage is scoped to a producer, configuration and finite universe.
  An omitted edge is unknown, not proof of absence.
- Semantic concepts are evidence-cited annotations, not dependency or
  behavioral-equivalence facts.
- Operational workflows compose existing Rust and SQLite functions; intermediate
  results do not each become independent authorities.

Human-readable output may be available alongside `--json`. Actual CLI grammar,
exit statuses and response formats must be tested against the built binary,
not invented in an unexecuted schema fixture.

For real Git-backed acceptance see
[the product implementation tests](08-acceptance-implementation.md),
[installed CLI smoke](09-installed-archive-implementation.md), and
[Excession dogfooding](../SELF_ANALYSIS.md). No new test-manifest schema is
needed for these tests.
