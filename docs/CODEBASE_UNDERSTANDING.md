# Investigating an unfamiliar codebase today

For large repositories, use `--budget-file` or the `--max-file-bytes`,
`--max-segment-bytes`, and `--max-segment-files` overrides. Incomplete
segments resume at the next invocation rather than disappearing from the
index. The remaining non-streaming and non-paginated boundaries are listed in
[resource budgets](RESOURCE_BUDGETS.md).

The primary task is **answering a behavior question**. Source location and
static graph edges are supporting evidence, not a finished explanation.

**Source search uses pinned Git first.** `agent locate` now searches the
committed tree with literal Git grep terms, includes test suites with
non-language extensions such as SQLite's `.test` and `.tcl`, then fills
remaining results from its FTS5 index. The reading plan inspects likely
implementation and task-matching test source **before** graph neighbors.
Each Git excerpt is verified against its original Git blob identity, with
explicit truncation and unreadable-source gaps. Neither a matching test name
nor a graph reference proves the behavior is covered. This reuse of ordinary
Git search is intentional; Excession should not maintain a competing
language index just to identify source files.
This is the practical workflow using Excession's *implemented* interfaces.
The complete [investigation artifact contract](INVESTIGATIONS.md) is a
target, not currently emitted by one command.

## 1. Ask a concrete question

Example: *Where does a worker recover a lost lease, which state changes,
and what tests constrain that behavior?*

```sh
excession agent locate --repo /work/project --rev HEAD \
  --task "Recover a lost worker lease" --limit 12 --json
```

`agent locate` finds Git-authenticated source candidates. The response
has a bounded `reading_plan` of at most 12 unique files, evidence-typed
`relationship_candidates`, and `context`. Its `git_search` field distinguishes
committed-source coverage from FTS index coverage. Its original references separate
lexical matches, native-producer observations and test-name suggestions.
It does not inspect arbitrary dirty working-tree edits or certify behavior.

If an unrelated Python or Node static-language adapter refuses during the
optional context step, source location remains available: Excession returns
Git-authenticated lexical candidates and focus identities, but **no invented
static relationships**. It records `OPTIONAL_STATIC_CONTEXT_UNAVAILABLE`
with the adapter diagnosis under `context.coverage.gaps`. Authentication
failures, corrupt evidence, and other errors still refuse. This is not
full semantic understanding; inspect the named source and tests directly.

The segmented FTS5 index may be partial. Check the `index` status and
`SEGMENT_INDEX_INCOMPLETE` before interpreting no matches. See
[agent CLI](AGENT_CLI.md) for refresh controls and limits.

## 2. Inspect the governing implementation

Read the actual source windows from the named commit. For a known subset:

```sh
excession agent context --repo /work/project --rev HEAD \
  --focus src/worker.py,tests/test_worker.py --limit 32
```

Replace these illustrative paths with **tracked files that actually exist**.
Trace entry points, conditions, data passed, state mutations, external
effects, failure handling, and configuration. Distinguish compile-time or
static references from behavior under real inputs.

If a source reference is missing or unavailable, record the gap. Do not
create an invented edge to make the explanation complete.

## 3. Build a small behavioral model

For each material claim, write the source support, assumption and check:

| Claim | Source / observation | Verification | Unknown |
| --- | --- | --- | --- |
| Which path invokes recovery? | Read code and independently scoped native references | Relevant execution or tests if available | Dynamic dispatch and external callers |
| What state changes? | Inspected write paths and guards | Assertions / trace for the named conditions | Unexamined failure modes |
| Which test governs the behavior? | Read the test and its assertions | Identified actual test execution receipt | Untested configurations |

This table is a **human/agent investigation note**, not a currently
emitted Excession JSON schema. Label claims source-derived, test-supported,
observed, hypothetical or unresolved according to
[investigations](INVESTIGATIONS.md).

## 4. Consider a change

Inspect specific edit boundaries and preservation requirements. When
comparing two committed revisions, `impact` and `agent verify` can
identify candidate effects and obligations, but cannot prove semantic
equivalence. Use [transformation workflow](TRANSFORMATIONS.md) and
[verification](VERIFICATION.md) for the required reasoning.

## 5. Decide if the explanation is sufficient

An investigation is useful when a reviewer can answer **how, why, where,
under what conditions and with what evidence**, while seeing what is not
yet known. A short correct explanation with relevant tests is better than
a huge graph with unsupported conclusions.

Test that outcome on independent tasks and repositories:
[evaluation](EVALUATION.md). For current feature constraints, see
[evidence limits](EVIDENCE_AND_LIMITS.md) and [MCP](MCP.md).

## Why not invest primarily in more SCIP edges?

Existing SCIP and symbol-site experiments already captured much of the
conservatively labeled static call/type evidence. Further coverage is
worthwhile when an independently graded investigation reveals a specific
missing observation. Relationship recall and source graph size are
**provider diagnostics**, not code-understanding acceptance criteria.
