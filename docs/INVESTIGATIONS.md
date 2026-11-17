# Investigations: the unit of understanding

**Status:** canonical product contract and target artifact. The CLI does
not yet emit a full persistent investigation report. The current supported
building blocks are `agent locate`, `agent context`, `impact`, `explain` and
`agent verify`; see [agent CLI](AGENT_CLI.md).

An investigation answers a **question at an exact repository revision**,
not a request for a complete graph. It ends with a reviewable explanation,
a compact implementation map, evidence, alternative hypotheses, and open
questions.

## Required content

| Field / concept | Meaning |
| --- | --- |
| Question and scope | User question, repository and Git commit, configuration and declared source roots |
| Implementation map | Small set of source paths and exact windows with their roles, not a complete symbol inventory |
| Behavioral claims | Explicit claims about entry points, data movement, state, errors and effects |
| Supports | Source/producer/test/trace IDs and the specific aspect each supports |
| Assumptions | Configuration, environment, external contracts, and boundaries of the claim |
| Alternatives | Competing interpretations, counterexamples and rejected hypotheses |
| Unknowns | Missing evidence, unsupported cases and next inspection questions |
| Completeness | Coverage *for this question*, not a repository-wide completeness percentage |
| Reproducibility | Original Git and tool revisions plus derivation, receipts and limits |

Every claim must keep a distinction between **observation** and **inference**.
Do not attach a single numeric confidence score to an entire investigation.

## Claim states

- **OBSERVED:** the specified event or value was observed in an independently
  identified execution or trace, within its recorded conditions.
- **TEST_SUPPORTED:** named tests ran and checked a specified property.
- **SOURCE_DERIVED:** follows from inspected source under listed assumptions;
  no execution is implied.
- **HYPOTHESIS:** plausible but not established.
- **UNRESOLVED:** relevant evidence is missing, conflicting or unsupported.
- **STALE:** a dependent source or configuration has changed since validation.

A claim may carry several evidence classes; a source-derived claim does not
silently become observed because a nearby test passed. Formal proofs, when
available, must record precisely the theorem and assumptions they establish.

## Demand-driven investigation procedure

1. State the question in behavioral terms and pin revision/configuration.
2. Locate candidate entry points and tests. Read original source; report
   source coverage and search limits.
3. Follow only relationships that help answer the question. Verify edges
   using the appropriate producer, source read or execution observation.
4. Construct an implementation map with control/data/state boundaries.
5. Seek relevant negative paths, error handling and counterexamples.
6. State claims with their evidence and unknowns; ask for only the next
   useful observation if the answer remains incomplete.
7. Reuse or refresh these claims during a follow-up or a transformation.

At each step, allow **no result**, **partial result**, or **refusal**.
A bounded reading plan is a useful intermediate artifact but not the final
behavioral explanation.

## Example investigation question

> In SQLite, which steps make a committed transaction durable, and which
> assumptions change between rollback-journal and WAL modes?

A valid report must establish the relevant paths and boundaries from pinned
SQLite source, name mode-specific conditions and tests, and explicitly
report missing evidence. This is a **benchmark question**, not a statement
that Excession has already completed the investigation.

## Proposed agent experience (not a live API)

```text
investigate(question, repository, revision, scope?)
expand(investigation_id, question_or_unknown)
```

These names are proposed logical operations. Current MCP tools are
`search_code`, `get_context`, `explain_impact`, `verify_change`,
`plan_tests` and `check_coverage`. Do not advertise the proposed API,
persisted IDs or claim states as available command syntax.

## Completion criteria

An investigator can use the report to locate the governing code, explain
why it behaves as claimed, identify relevant tests, and distinguish
established answers from guesses. The report remains falsifiable and
independently reviewable without Excession. Refer to
[evaluation](EVALUATION.md) and [evidence lifecycle](EVIDENCE_LIFECYCLE.md).
