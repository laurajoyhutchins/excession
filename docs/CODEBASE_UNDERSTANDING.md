# Understanding a codebase with Excession
For large repositories, use `--budget-file` or the `--max-file-bytes`,
`--max-segment-bytes`, and `--max-segment-files` overrides. Incomplete
segments resume at the next invocation rather than disappearing from the
index. The remaining non-streaming and non-paginated boundaries are listed in
[resource budgets](RESOURCE_BUDGETS.md).


Excession should help an engineer answer questions about unfamiliar software.
A graph of symbol references is an input to that work, not its result.

## Start with a question

For example: *Where does a worker recover a lost lease, what state does it
change, and what tests describe the expected behavior?*

```sh
excession agent locate --repo /work/project --rev HEAD \
  --task "Recover a lost worker lease" --limit 12 --json
```

The result includes `candidates`, `relationship_candidates`, `context`
and a **`reading_plan`**. The reading plan is an ordered set of Git-pinned
source files to inspect. Its entries distinguish task-text matches from
source-bound producer observations, static candidates and filename-based test
suggestions. Every step references an existing element of the original
report; it does not create new dependency edges or certify behavior.

Read the suggested source, then check the hypotheses it raises:
what invokes this code, which inputs and configuration matter, what state or
external systems it touches, and what the tests really assert. A candidate
test is not necessarily an executing or relevant test.

`reading_plan` is a **navigation aid**, not a generated behavioral
explanation. It is deliberately small (at most 12 unique files), never reads
the working tree, and does not turn SCIP symbol references into runtime calls.
If no match is found or the index is incomplete, the gaps are retained.
The user should be able to ask a follow-up question without being handed
a second graph dump.

## The product questions

A codebase-understanding investigation should eventually answer:

| Question | Useful answer | Evidence |
| --- | --- | --- |
| Where should I start? | Few likely implementation files and source windows | Git-bound lexical search, source |
| How does this behavior work? | Verified sequence of entry points, data transformations, state changes and effects | Read code, compiler symbols, tests, configuration |
| What depends on it? | Relevant callers, consumers, code paths and uncertainty | Native observations, build configuration, execution receipts |
| What assumptions govern it? | Preconditions, invariants, contracts and failure cases | Assertions, tests, type definitions, documented contracts |
| What changes if I edit this? | Source-level impact candidates, verification work and unknowns | Git diff, dependency evidence, test/build receipts |

The first row has a partly implemented assistant workflow now. The remaining
rows require evidence-guided reasoning over **actual source**, not an
ever-larger SCIP or lexical graph. Results should not be labeled
`verified` until the particular claim has independent support.

## Architecture boundary

```text
User task / question
     |
     v
Find relevant implementation and test source
     |
     v
Plan a bounded reading order and identify open questions
     |
     v
Inspect source, configuration, data movement, failure paths and invariants
     |
     v
Explain behavior with exact source citations and explicit unknowns
     |
     v
Investigate effects of a proposed change and choose verification
```

Git authenticates source identity. SCIP, language servers, compilers, test
runners and other tools **supply observations when useful**. Excession owns
selection, contextualization, citations, uncertainty and the resulting
investigation workflow. It must not build a second SCIP implementation or
pretend that a list of observed cross-file references is a code explanation.

The initial `reading_plan` composes existing `agent locate` and
`agent context` reports. It neither adds a new database nor a language
resolver. The richer behavioral explanation remains future work, and must
be tested against original code and independent answer keys before claiming
accuracy.

## How to measure improvement

**Primary metric:** Whether an agent unfamiliar with the repository answers
a concrete codebase question correctly, with fewer inspected bytes,
fewer tool calls and less elapsed work than the same agent using ordinary Git
search and a language server.

Use preselected, independently reviewed tasks on *other people's
repositories*, covering entry points, dataflow, error handling, invariants,
test ownership and change impact. Compare answer correctness, citation
accuracy, unresolved uncertainty and reviewer effort. Preserve negative
results and false explanations.

**Diagnostics**, not product success metrics: number of SCIP edges, matched
cross-file symbol occurrences, observed pair retention, and size of the
source graph. The [import and symbol-site experiments](../benchmarks/acceptance/DEPENDENCY_RECALL.md)
measure only part of the evidence layer. Their high full-index coverage
does not imply that Excession makes unfamiliar software understandable.

## Next technical priorities

1. Evaluate task-to-source reading plans, specifically whether the first
   few files actually contain the implementation, tests and key boundaries.
2. Build a *trace question* query that narrows source observations to the
   user's question, exposing relevant source windows rather than
   indiscriminately paging SCIP occurrences.
3. Compose an auditable explanation of behavior with source anchors and
   marked hypotheses. Do not have the model infer executable control flow
   solely from static cross-file symbols.
4. Add an investigation history that records what source was inspected,
   what was established, what remains unknown and which question to ask
   next. Reuse the existing Git/evidence model.

SCIP improvements should be justified only when they unlock a measured
code-understanding task. A perfectly reproduced language index is not an
end goal.
