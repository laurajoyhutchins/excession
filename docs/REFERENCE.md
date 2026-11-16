# Reference and research index

Start with the [documentation home](README.md) if you are looking for an
ordinary user workflow. This index includes specialized implementation notes
and retained design records. **A proposed schema, algorithm, or PR plan is
not evidence that a feature is present in the current release.**

## Current user and agent interfaces

- [Quick start](QUICKSTART.md)
- [CLI options and command families](CLI.md)
- [Agent locate, context, and verify](AGENT_CLI.md)
- [GitHub-native analysis requests](GITHUB_NATIVE_ANALYSIS.md)
- [Reusable PR evidence](GITHUB_EVIDENCE.md)
- [Local stdio MCP](MCP.md)
- [Source-impact witness semantics](COMPACT_IMPACT.md)
- [Refactoring candidate analysis](REFACTORING.md)
- [Native test selection](NATIVE_TEST_SELECTORS.md)

## Architecture and evidence

- [Current architecture](ARCHITECTURE.md)
- [Evidence and limits](EVIDENCE_AND_LIMITS.md)
- [Source-producing tools](SOURCE_PRODUCERS.md)
- [Observed relationship analysis](OBSERVED_RELATIONSHIP_ANALYSIS.md)
- [Source coverage and incomplete dependencies](DEPENDENCY_COVERAGE.md)
- [Evidence querying](EVIDENCE_QUERY.md)
- [Configured graphs](CONFIGURED_GRAPHS.md)
- [Runtime observations](RUNTIME_OBSERVATIONS.md)
- [Semantic contracts](SEMANTIC_CONTRACTS.md)
- [Projections and source relationships](PROJECTION_MODEL_BOUNDARY.md)
- [Graph queries](GRAPH.md)

## Language-native observers

- [Syntax parsers](SYNTAX_PARSERS.md)
- [Native compiler producers](NATIVE_COMPILER_PRODUCERS.md)
- [Compiler analysis](COMPILER_ANALYSIS.md)
- [Compiler checks and receipts](COMPILER_CHECK_RECEIPTS.md)
- [Clang translation units](CLANG_TU.md)
- [JVM resolved inputs](JVM_RESOLVED_INPUTS.md)
- [Java-native observations](JAVA_NATIVE.md)
- [Python native resolution](PYTHON_NATIVE_RESOLUTION.md)
- [Python import names](PYTHON_IMPORT_NAMES.md)
- [Kernel parsers](KERNEL_PARSERS.md)
- [Kbuild graphs](KBUILD_GRAPHS.md)
- [Whole Linux source](LINUX_WHOLE_SOURCE.md)
- [Linux kernel experiment](LINUX_KERNEL_EXPERIMENT.md)

These documents discuss different producers and configurations. Read their
unsupported cases before interpreting an edge as a behavioral dependency.

## Source views, history and refactoring

- [Repository views](REPOSITORY_VIEWS.md)
- [Composable views](COMPOSABLE_VIEWS.md)
- [Git reconstruction](GIT_RECONSTRUCTION.md)
- [Reconstruction evidence](GIT_RECONSTRUCTION_EVIDENCE.md)
- [Reconstruction oracles](GIT_RECONSTRUCTION_ORACLES.md)
- [Reconstruction synthesis](GIT_RECONSTRUCTION_SYNTHESIS.md)
- [Reconstruction runtime boundary](RECONSTRUCTION_RUNTIME_BOUNDARY.md)
- [History reorganization safety](HISTORY_REORGANIZATION.md)
- [Target architecture proposal](TARGET_ARCHITECTURE.md)
- [Refactoring](REFACTORING.md)
- [Self-refactor audit](SELF_REFACTOR_AUDIT.md)
- [Self-analysis](SELF_ANALYSIS.md)
- [Improvement loop](IMPROVEMENT_LOOP.md)
- [Migration notes](MIGRATION.md)

## Security investigation

- [Security candidate discovery](SECURITY_CANDIDATE_DISCOVERY.md)
- [Scanner validity study](SECURITY_VALIDITY.md)
- [Held-out security results](SECURITY_HELDOUT_RESULTS.md)
- [Historical security-focused architecture](SECURITY_ARCHITECTURE_RESEARCH.md)

These are evidence studies and evaluation methods, not a claim of
autonomous vulnerability discovery or independent exploitability proof.

## Evidence-core development records

The [evidence-core directory](evidence-core/) contains bounded
implementation and contract work:

- [Source identities](evidence-core/b-identities.md)
- [Persistence](evidence-core/c-persistence.md)
- [Coverage](evidence-core/d-coverage.md)
- [Unknown-event history](evidence-core/unknown-history.md)
- [Comparison](evidence-core/comparison.md)
- [Source/artifact relations](evidence-core/artifact.md)
- [Build provenance](evidence-core/build-provenance.md)
- [Attestation](evidence-core/attestation.md)
- [REA observations](evidence-core/rea.md)
- [Predicate audit](evidence-core/predicate-audit.md)
- [Reconstruction](evidence-core/reconstruction.md)
- [SCIP vertical](evidence-core/SCIP_VERTICAL.md)
- [Ghidra corpus](evidence-core/ghidra-corpus.md)

Some records describe work on draft branches. Their dates and acceptance
notes matter; consult emitted code and independent test receipts for
current implementation status.

## Product design records

The [product-contracts index](product/README.md) links the staged plans,
implementation notes, contract changes and acceptance documents. The
original PR-numbered pages are retained as **historical plans**. In
particular, old “not implemented” statements do not override a later,
independently tested implementation. Current command behavior belongs
in [CLI reference](CLI.md).

## Developer operations

- [Development](DEVELOPMENT.md)
- [CI and execution policy](CI.md)
- [Verification profiles](VERIFICATION_PROFILE.md)
- [Real fixtures](REAL_FIXTURES.md)
- [Portability](PORTABILITY.md)
- [Release procedure](RELEASE.md)
- [Documentation maintenance](DOCUMENTATION.md)
- [Origins](ORIGINS.md)
- [Contributing](../CONTRIBUTING.md)

## Evidence hierarchy

For present-day claims, prefer an exact source revision and native code,
then a versioned schema emitted by that revision, an independently run test
or workflow artifact, and finally explanatory notes. A historical plan
is useful context but is not a substitute for those sources.
