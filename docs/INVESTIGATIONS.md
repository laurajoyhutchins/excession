# Investigations: the unit of understanding

**Status:** the first persistent Git-bound investigation ledger is implemented
(`excession investigation`). It retains claims, preservation obligations and
verification reports, but does **not** automatically derive an explanation,
infer behavior or validate refactoring equivalence. See [agent CLI](AGENT_CLI.md)
for evidence-gathering building blocks.

An investigation answers a **question at an exact repository revision**,
not a request for a complete graph. It ends with a reviewable explanation,
a compact implementation map, evidence, alternative hypotheses, and open
questions.

## Implemented investigation ledger (v1)

The CLI writes append-only canonical JSON events to an explicitly chosen
SQLite file **outside the source checkout**. Each event includes a Git
commit/tree identity, optimistic revision, event ID and predecessor digest.

```sh
# Pin source; returns investigation_id and event_id
excession investigation start \
  --repo /work/project --rev HEAD \
  --db /tmp/investigations.sqlite --question "How does recovery work?"

# Inspect the exact tracked source first, then report a source-grounded claim
excession investigation claim \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision 1 \
  --path src/worker.rs --statement "The retry path calls recovery" \
  --assumption "This is source review, not an execution observation" \
  --assessment SOURCE_REVIEW_REPORTED

# Bind a preservation obligation to that claim's returned event_id
excession investigation plan \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision 2 \
  --claim-event CLAIM_EVENT_ID \
  --obligation "Preserve retry error and ordering semantics"

# After a separately committed candidate, run existing agent verification
excession investigation verify \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision 3 \
  --plan-event PLAN_EVENT_ID --candidate CANDIDATE_COMMIT

# Audit the event chain and source/verification identities
excession investigation show \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID
```

`claim` accepts `HYPOTHESIS` or `SOURCE_REVIEW_REPORTED`, never a
free-form `VERIFIED` state. `plan` must reference an earlier claim and
`verify` must reference an earlier plan. Verification runs
`agent verify` and retains its `OPEN_OBLIGATIONS` or
`REPORTED_FAILURE` result, not a proof of behavior or merge approval.
The same expected revision cannot be written twice; event rows refuse SQL
update/delete, and `show` replays the chain and authenticates Git source.

The ledger does not yet select relevant source itself, infer call paths,
automatically expand open questions, invalidate claims across revisions,
or provide native source edits. Its event and report wire contracts are
[`investigation-event.v1`](../contracts/product/investigation-event.v1.schema.json)
and [`investigation.v1`](../contracts/product/investigation.v1.schema.json).

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

## Future automated agent experience (not a live API)

```text
investigate(question, repository, revision, scope?)
expand(investigation_id, question_or_unknown)
```

These names are proposed logical operations, separate from the implemented
`excession investigation` ledger commands. Current MCP tools are
`search_code`, `get_context`, `explain_impact`, `verify_change`,
`plan_tests` and `check_coverage`. Do not advertise the proposed API,
persisted IDs or claim states as available command syntax.

## Completion criteria

An investigator can use the report to locate the governing code, explain
why it behaves as claimed, identify relevant tests, and distinguish
established answers from guesses. The report remains falsifiable and
independently reviewable without Excession. Refer to
[evaluation](EVALUATION.md) and [evidence lifecycle](EVIDENCE_LIFECYCLE.md).
