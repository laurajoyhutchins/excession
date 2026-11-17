# Technical reference and historical research

**This is not the product roadmap.** The canonical direction is
[investigation-first understanding and transformation](PRODUCT.md),
with [architecture](ARCHITECTURE.md),
[evidence lifecycle](EVIDENCE_LIFECYCLE.md) and
[ADR 0001](DECISIONS/0001-investigation-first.md), and
[documentation cutover](CUTOVER.md).
The documents below remain valuable producer details, command references,
specialized evidence studies and historical experiments. Their existence
does not imply a shipped capability or change Excession's primary unit
of work from an investigation to a graph query.

## Present-day technical interfaces

- [Quick start](QUICKSTART.md)
- [CLI](CLI.md)
- [Agent CLI](AGENT_CLI.md)
- [Local MCP](MCP.md)
- [GitHub-native analysis](GITHUB_NATIVE_ANALYSIS.md)
- [Reusable PR evidence](GITHUB_EVIDENCE.md)
- [Refactoring commands](REFACTORING.md)
- [Improvement loop](IMPROVEMENT_LOOP.md)
- [Source-impact witnesses](COMPACT_IMPACT.md)
- [Native tests](NATIVE_TEST_SELECTORS.md)

## Provider and query internals

- [Observed relationships / SCIP](OBSERVED_RELATIONSHIP_ANALYSIS.md)
- [Graph queries](GRAPH.md)
- [Evidence queries](EVIDENCE_QUERY.md)
- [Configured builds](CONFIGURED_GRAPHS.md)
- [Source producers](SOURCE_PRODUCERS.md)
- [Language-native producers](NATIVE_COMPILER_PRODUCERS.md)
- [Compiler analysis](COMPILER_ANALYSIS.md)
- [Runtime observations](RUNTIME_OBSERVATIONS.md)
- [Dependency coverage](DEPENDENCY_COVERAGE.md)
- [Semantic contracts](SEMANTIC_CONTRACTS.md)
- [Composable views](COMPOSABLE_VIEWS.md)
- [Repository views](REPOSITORY_VIEWS.md)

## Historical and specialist work

- [Module consolidation target (historical)](TARGET_ARCHITECTURE.md)
- [Git reconstruction](GIT_RECONSTRUCTION.md)
- [History reorganization](HISTORY_REORGANIZATION.md)
- [Security research](SECURITY_ARCHITECTURE_RESEARCH.md)
- [Security validity](SECURITY_VALIDITY.md)
- [Self-refactoring audit](SELF_REFACTOR_AUDIT.md)
- [Legacy migration](MIGRATION.md)
- [Product contract plans](product/README.md)
- [Evidence-core research records](evidence-core/)

Additional files in this directory remain findable through GitHub's file
tree and history; they do not constitute a second canonical documentation
hierarchy. A proposal or old PR saying "not implemented" describes its
original scope, not necessarily today's implementation. Conversely, a
schema or planned benchmark does not demonstrate completed functionality.

For current claims: inspect pinned source, emitted versioned schemas and
independent run receipts. See [evidence limits](EVIDENCE_AND_LIMITS.md).
