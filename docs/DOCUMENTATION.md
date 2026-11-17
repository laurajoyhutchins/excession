# Maintaining documentation

Documentation must tell agents **what to do today**, **what Excession can
actually establish**, and **what is only proposed**. It is part of the
source/evidence contract.

## Canonical ownership

| Topic | Canonical location |
| --- | --- |
| Product and scope | [Product](PRODUCT.md) and [ADR 0001](DECISIONS/0001-investigation-first.md) |
| Task navigation | [Documentation home](README.md) |
| Current system and target boundaries | [Architecture](ARCHITECTURE.md) |
| Evidence flow and invalidation | [Evidence lifecycle](EVIDENCE_LIFECYCLE.md), [limits](EVIDENCE_AND_LIMITS.md) |
| Question-to-explanation | [Investigations](INVESTIGATIONS.md) and [practical workflow](CODEBASE_UNDERSTANDING.md) |
| Refactoring and change preservation | [Transformations](TRANSFORMATIONS.md), [native commands](REFACTORING.md) |
| Check interpretation | [Verification](VERIFICATION.md) |
| End-to-end efficacy | [Evaluation](EVALUATION.md) |
| Exact command flags | [CLI](CLI.md), [agent CLI](AGENT_CLI.md), [MCP](MCP.md) |
| Developer operations | [Development](DEVELOPMENT.md), [CI](CI.md), [release](RELEASE.md) |
| Specialist/historical studies | [Reference](REFERENCE.md) |

The README is a concise entry point, not a second command reference.
Specialist pages may describe SCIP/graphs/ontologies, but must not
reintroduce indexing as Excession's product goal.

## Implementation honesty

- **Implemented:** real code and acceptance at a named revision.
- **Partial:** supported behavior under stated language, size,
  configuration, tooling or source limits.
- **Experimental:** scoped results from a reproducible study.
- **Proposed:** a contract/ADR/desired interface with no shipped command.
- **Historical:** notes from a previous branch or research design.

Never write a proposed command as a runnable example. Do not promote a
claimed verified behavior based only on static edges, source hashes,
passing syntax checks, or a successful workflow.

## Evidence-aware writing

Anchor source-level statements in current implementation and exact paths.
For performance or behavioral assertions, cite independent receipts,
fixtures, revisions and conditions. A meaningful refactoring example
names what must remain unchanged and what verification actually exercised.

State unknowns and negative outcomes explicitly. Avoid repeated generic
boilerplate and ungrounded adjectives. Keep one source of truth for each
contract and link to it rather than copy evolving command flags.

## Validation

```sh
python3 tools/check_docs.py
python3 -m unittest discover -s tests -p 'test_docs.py' -v
```

The link checker tests local link targets, not the correctness of source
claims or external URLs. Before merging, inspect rendered navigation,
relative links, examples against actual `--help`, and the **exact PR head**
checks in GitHub Actions. There is no required docs server: GitHub
renders checked-in Markdown and Git history preserves revisioned changes.
