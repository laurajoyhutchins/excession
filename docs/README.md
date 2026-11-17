# Documentation

Excession has one Rust executable and several ways to consume its evidence.
Start with the task you want to perform. Documentation here distinguishes
**implemented commands** from **experimental designs** and
**historical proposals**.

## Start here

| Task | Read | Expected result |
| --- | --- | --- |
| Understand unfamiliar code from a question | [Codebase understanding](CODEBASE_UNDERSTANDING.md) | Source-first investigation and bounded reading plan |
| Install and inspect a repository | [Quick start](QUICKSTART.md) | A report pinned to Git source |
| Ask Excession from ChatGPT | [GitHub-native analysis](GITHUB_NATIVE_ANALYSIS.md) | Workflow artifact with original report and receipt |
| Ask about a private repository | [Private GitHub analysis](PRIVATE_GITHUB_ANALYSIS.md) | Advisory evidence retained only in the caller's private Actions run |
| Review a pull request | [GitHub PR evidence](GITHUB_EVIDENCE.md) | Base/head report, provenance, coverage, open obligations |
| Integrate a local agent | [Stdio MCP](MCP.md) | Tools executed inside the agent's trusted local checkout |
| Interpret results | [Evidence and limits](EVIDENCE_AND_LIMITS.md) | Clear boundaries between observations and conclusions |
| Develop Excession | [Development](DEVELOPMENT.md) and [CI policy](CI.md) | Pinned tools and independently checked changes |

## Use the CLI

| Question | Guide |
| --- | --- |
| What commands exist and what are their options? | [CLI reference](CLI.md) |
| Where is a behavior implemented? | [Codebase understanding](CODEBASE_UNDERSTANDING.md) and [agent locate/context/verify](AGENT_CLI.md) |
| What might this change affect? | [Source-impact witnesses](COMPACT_IMPACT.md) |
| How do I inspect and improve a refactoring? | [Refactoring](REFACTORING.md) |
| How are independent checks selected and executed? | [Native test selection](NATIVE_TEST_SELECTORS.md) |
| How can I inspect a smaller view of source? | [Repository views](REPOSITORY_VIEWS.md) and [composable views](COMPOSABLE_VIEWS.md) |
| How does the tool inspect its own Rust code? | [Self-analysis](SELF_ANALYSIS.md) |
| How does history planning avoid rewriting Git? | [Git reconstruction](GIT_RECONSTRUCTION.md) and [history safety](HISTORY_REORGANIZATION.md) |

## Understand the implementation

Read the [architecture](ARCHITECTURE.md) for the complete data flow,
authority boundaries, runtime surfaces, and supported versus unsupported
claims. The [evidence model](EVIDENCE_AND_LIMITS.md) explains commit/tree/blob
identity, producer receipts, partial coverage, replay, and semantic validity.

More specific implementation documentation is available for
[SCIP and observed relations](OBSERVED_RELATIONSHIP_ANALYSIS.md),
[configured build graphs](CONFIGURED_GRAPHS.md),
[compiler-declared inputs](COMPILER_ANALYSIS.md),
[Python import names](PYTHON_IMPORT_NAMES.md),
[syntax parsers](SYNTAX_PARSERS.md), and
[evidence querying](EVIDENCE_QUERY.md).
These producers do not share an invented semantic guarantee.

## Build, test, and release

[Development setup](DEVELOPMENT.md) defines the pinned toolchains and local
commands. [CI policy](CI.md) distinguishes fast checkpoint tests, required PR
checks, scoped native evidence, and deep acceptance. [Release procedure](RELEASE.md)
and [portability](PORTABILITY.md) describe independently tested distribution
artifacts. [Contribution guidelines](../CONTRIBUTING.md) describe change
acceptance; [documentation maintenance](DOCUMENTATION.md) covers the docs.

## Detailed reference and research

The [reference index](REFERENCE.md) groups the rest of the source-producer
documentation, domain-specific audits, experiments, and historical contract
plans. Those pages are retained for reproducibility but do not override the
current CLI, live tests, or versioned schemas.

An old PR plan saying “not implemented” describes the state **at the time
that plan was written**. Use current implementation and acceptance tests to
determine present support. Conversely, a schema or proof outline in a
research page is not execution evidence.

## How to verify a claim

Follow the source identity into the named native report. Read its coverage
and open obligations. For a behavior or performance claim, locate the
independent test or benchmark and pin the exact commit and configuration.
A successful GitHub Actions run is evidence that a workflow executed, not
that every property claimed by its inputs is true.
