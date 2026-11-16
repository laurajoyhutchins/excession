# Quick start

This guide covers the native CLI. For analysis requested through a ChatGPT
conversation, use [GitHub-native analysis](GITHUB_NATIVE_ANALYSIS.md).
For other repositories' pull requests, use
[the reusable evidence workflow](GITHUB_EVIDENCE.md).

## 1. Install the executable and its parser

Download the correct platform archive and adjacent checksum from
[GitHub Releases](https://github.com/laurajoyhutchins/excession/releases).
Verify the checksum before extraction. Keep the entire extracted
`excession/` directory together: the binary uses its packaged
`lib/typescript.js` when JavaScript/TypeScript analysis is required.

Consult [release installation](RELEASE.md) for the supported platforms and
removal/upgrade procedure. A Cargo build from source is also possible;
[development setup](DEVELOPMENT.md) documents the pinned toolchain and
locked dependencies.

Git is required. Other language tools are optional or producer-specific.
Check what this particular environment supports:

```sh
excession doctor
excession doctor --json
```

A missing optional producer is a coverage gap, not permission to synthesize
its observations. Use `excession commands` to list all command families.

## 2. Inspect a committed repository

From the repository's root:

```sh
excession discover .
excession analyze . --json
```

`discover` reads the Git repository and reports the exact revision, source
roots, and available capabilities. `analyze` indexes supported static
Python and JavaScript/TypeScript candidates with their source identities.
The result does not establish runtime calls, behavioral correctness, or
complete cross-language dependencies.

To locate source from a task rather than a known file path:

```sh
excession agent locate --repo . \
  --task "Find the code that validates source commit identities" \
  --rev HEAD --limit 10 --json
```

This is lexical search over Git source. Its segmented cache can be partial.
A full refresh is explicit:

```sh
excession agent locate --repo . \
  --task "Find the code that validates source commit identities" \
  --index-segments 256 --limit 10 --json
```

See [agent-facing commands](AGENT_CLI.md) for bounded context, explicit
focus paths, and revision verification.

## 3. Inspect changed-source impact

Use two committed revisions. By default, `impact` compares
`HEAD~1..HEAD`; `--base` and `--head` override that range.

```sh
excession impact . --output /tmp/excession-impact.json
```

Read the resulting `excession.impact.v3` JSON. Its
`candidate_source_impacts` entries identify sources reachable through
observed relationships; an entry is not evidence that a change breaks
that source. Select a **real path from the report**:

```sh
excession explain . --report /tmp/excession-impact.json \
  --source path/from/report.py --json
excession explain . --report /tmp/excession-impact.json \
  --source path/from/report.py --audit
```

`--audit` checks what can be replayed against source and evidence. It
does not execute independent behavioral tests unless a separate, explicit
test workflow has done so.

[Source-impact witness semantics](COMPACT_IMPACT.md) documents the
bounded representation; [CLI reference](CLI.md) covers alternate options.

## 4. Read the evidence, not only the summary

For any report, check the exact source commit, tree, blob and mode where
present; the named producer and its original receipt; declared coverage,
truncation and unsupported cases; and unresolved obligations.

A deterministic replay of Git objects is not a semantic validation of
source behavior. A source link in SCIP is not automatically a runtime call.
A suggested test is not an independently verified complete test selection.
See [evidence and limits](EVIDENCE_AND_LIMITS.md).

## 5. What to do next

Use [GitHub-native analysis](GITHUB_NATIVE_ANALYSIS.md) for agent queries
without a local executable, [MCP](MCP.md) for a local coding agent,
[refactoring](REFACTORING.md) to investigate change candidates, or
[development](DEVELOPMENT.md) to work on Excession itself.

### Working-tree changes

The source-analysis commands above use committed Git objects, even if
uncommitted files are present in the working tree. They do **not** analyze
the uncommitted edits themselves. The separate
[native test workflow](NATIVE_TEST_SELECTORS.md) has explicit, sandboxed
handling for local uncommitted test execution. Do not treat a committed
source report as evidence about the dirty worktree.
