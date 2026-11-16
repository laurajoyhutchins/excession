# Maintaining the documentation

The documentation is part of Excession's evidence boundary. A command
example or supported-feature statement must match a real executable
and be clear about where its guarantee ends.

## Where content belongs

| Content | Canonical location |
| --- | --- |
| High-level introduction and supported entry points | [Root README](../README.md) |
| Task-based navigation | [Documentation home](README.md) |
| Reproducible first analysis | [Quick start](QUICKSTART.md) |
| Exact CLI syntax and flags | [CLI reference](CLI.md) and [agent CLI](AGENT_CLI.md) |
| Source and producer authority | [Architecture](ARCHITECTURE.md), [evidence and limits](EVIDENCE_AND_LIMITS.md) |
| In-chat and CI execution | [GitHub-native request](GITHUB_NATIVE_ANALYSIS.md), [PR evidence](GITHUB_EVIDENCE.md) |
| Local agent protocol | [MCP](MCP.md) |
| Operations | [Development](DEVELOPMENT.md), [CI](CI.md), [release](RELEASE.md) |
| Detailed producer contracts and research | [Reference index](REFERENCE.md) |

Do not expand the README into a second CLI manual. Link to the canonical
guide rather than copying paragraphs of command flags into another page.

## Status and evidence

Every description should distinguish:

- **Implemented:** a command exists in the repository and its supported
  behavior has an acceptance test at a pinned revision.
- **Partial or conditional:** a producer exists but its language,
  configuration, resource bound, or dependencies limit coverage.
- **Experimental:** a reproducible experiment has a scoped result but
  does not establish a general property.
- **Proposed or historical:** a contract, PR plan, or design sketch.
  Its presence in `docs/` does not mean it shipped.

A dated benchmark should include its date, exact revision, hardware and
tool versions where relevant, fixtures, independent baseline and
unresolved cases. Link to a retained receipt when one exists; avoid
guessing results from test names or comments.

For a newly supported feature, update both the task guide and its
emitted schema or native acceptance reference. Do not move a proposal to
“implemented” without evidence.

## Writing

Use direct technical names. Explain what runs, what Git objects it uses,
what it produces, and what the caller must verify. Avoid slogans,
capability adjectives, speculative future tense presented as present
support, and repeated safety boilerplate.

Use normal Markdown headings, relative repository links, and fenced
shell/code examples. Use exact file paths and tested command syntax.
Prefer one complete example with its expected output contract over
multiple variations that do not specify inputs.

When a feature is superseded, preserve historical material only when it
has useful design or research provenance. Mark it explicitly as
historical and link to its current replacement. Do not delete a proof
obligation merely because newer functionality exists.

## Validation

```sh
python3 tools/check_docs.py
python3 -m unittest discover -s tests -p 'test_docs.py' -v
```

The local-link checker runs as part of `Verify`, before expensive
dependency installation. It checks relative Markdown and image targets,
including references into source, contracts, and workflows. It ignores
external URLs and fenced code examples. It cannot prove external pages
are live or that the linked source implements a claimed behavior.

Before merging, inspect README rendering, the [documentation home](README.md),
the affected source guides, and the GitHub Actions result at the PR's
actual head. A passing link check proves link targets exist, not that
technical descriptions are correct.

## What is not required

There is no extra docs server or generated knowledge base. GitHub renders
the checked-in Markdown. CI and agent workflows use the same repository
and versioned source, so links and snippets can be reviewed alongside
implementation changes. A future generated website should reuse this
content rather than creating another independent documentation authority.
