# Documentation

**Start with the engineering question, not an index command.**

The [documentation cutover](CUTOVER.md),
[product contract](PRODUCT.md) and
[ADR 0001](DECISIONS/0001-investigation-first.md) define Excession:
an investigation-driven codebase understanding and transformation system.
The [architecture](ARCHITECTURE.md) separates current implementation
from future investigation artifacts. Historical proposals and specialist
tool documentation are not alternative product definitions.

## Main paths

| Goal | Canonical guide | Current executable entry |
| --- | --- | --- |
| Understand how a behavior works | [Investigation contract](INVESTIGATIONS.md) and [practical workflow](CODEBASE_UNDERSTANDING.md) | `agent locate`, `agent context`, source inspection |
| Refactor a repository | [Transformation contract](TRANSFORMATIONS.md) | `refactor` / `improve` specialists, native tools |
| Establish what a change preserves | [Verification](VERIFICATION.md) | Impact/evidence replay, independent tests and receipts |
| Follow provenance and invalidation | [Evidence lifecycle](EVIDENCE_LIFECYCLE.md) | Git identities, producers, SQLite and audits |
| Evaluate actual benefit | [Evaluation](EVALUATION.md) | Held-out study protocol; existing benchmarks are diagnostics |

The first column names **product workflows**. The last column
names the **presently implemented building blocks**. Do not invent an
`excession investigate` or `excession transform` command from the
product architecture.

## Work with Excession today

| Environment | Reference |
| --- | --- |
| Terminal, installation and first repository analysis | [Quick start](QUICKSTART.md) and [CLI](CLI.md) |
| Existing agent commands | [Agent CLI](AGENT_CLI.md) |
| Local coding agent | [Stdio MCP](MCP.md) |
| ChatGPT using GitHub | [GitHub-native analysis](GITHUB_NATIVE_ANALYSIS.md) |
| Private GitHub analysis | [Private workflow](PRIVATE_GITHUB_ANALYSIS.md) |
| Pull request evidence | [Reusable workflow](GITHUB_EVIDENCE.md) |
| Current source-bound refactoring operations | [Refactoring technical reference](REFACTORING.md) |
| Native test selection | [Test selectors](NATIVE_TEST_SELECTORS.md) |

## Architecture and source of truth

- [Current architecture and migration boundary](ARCHITECTURE.md)
- [Product contract](PRODUCT.md)
- [Evidence and limits](EVIDENCE_AND_LIMITS.md)
- [Evidence lifecycle](EVIDENCE_LIFECYCLE.md)
- [Investigations](INVESTIGATIONS.md)
- [Transformations](TRANSFORMATIONS.md)
- [Verification](VERIFICATION.md)
- [Accepted architecture decision](DECISIONS/0001-investigation-first.md)

## Develop and maintain

[Development](DEVELOPMENT.md) · [CI policy](CI.md) ·
[Release](RELEASE.md) · [Contributing](../CONTRIBUTING.md) ·
[Agent instructions](../AGENTS.md) ·
[Documentation maintenance](DOCUMENTATION.md) ·
[Benchmark program](../benchmarks/README.md).

For native producer details, graph queries, research experiments, historical
plans, and versioned evidence contracts use the
[reference index](REFERENCE.md). Its contents are retained for
reproducibility, not promoted to shipped product features. The deprecated
[architecture-module migration plan](TARGET_ARCHITECTURE.md) is historical
context, not the current product architecture.
