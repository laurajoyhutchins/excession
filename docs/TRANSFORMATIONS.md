# Transformations: investigation-driven refactoring

**Status:** canonical target workflow. Excession now also supports an
append-only `investigation plan` event linking a preservation obligation
to a source-bound claim. It already exposes bounded `refactor` and
`improve` commands, including source-bound candidate reports and isolated
native-edit replay, but **does not implement this complete end-to-end
transformation contract**. See [refactoring technical reference](REFACTORING.md)
and [improvement loop](IMPROVEMENT_LOOP.md) for exact shipped commands.
See [investigation ledger](INVESTIGATIONS.md) for persisting the obligation
and `investigation refresh` for conservative direct-source reassessment after
a candidate edit. An unchanged source blob does not discharge an obligation.

A transformation changes structure while preserving specified behavior or
explicitly documenting an intentional behavior change. It starts from a
source-grounded [investigation](INVESTIGATIONS.md).

## Transformation plan

A plan must specify:

1. **Problem and evidence:** what is bloated, duplicated, coupled or
   misleading, with exact source and behavioral observations.
2. **Desired boundary:** the new ownership and intended module/API shape;
   why it reduces cost or confusion.
3. **Preservation contract:** public behavior, error semantics, ordering,
   side effects, state transitions, compatibility constraints, performance
   bounds and data invariants that must remain true where applicable.
4. **Allowed differences:** what may intentionally change and who has
   authorized it.
5. **Edit plan:** affected source identities, native tool capabilities,
   proposed edits and unresolved source/configuration scope.
6. **Verification plan:** independent compile/build checks, tests, targeted
   negative cases, differential execution and post-change evidence refresh.
7. **Residual risk:** anything not exercised, unsupported or not proved.

A matching body fingerprint, source dependency cycle, or SCIP reciprocal
reference is only a candidate. Consolidation requires an actual comparison
of semantics, contracts, callers and error paths.

## Execution boundary

- Prefer a compiler, language server, native refactoring tool or
  language-aware edit producer for source changes.
- Excession validates identities and precise edits and can orchestrate
  bounded replay. It must not grow a second language implementation.
- Apply only with explicit authorization in an isolated worktree. The
  original checkout and dirty files are not mutation targets.
- Do not assume that byte-exact edit replay, syntax validity, a successful
  build, or the removal of a graph edge proves a safe refactoring.
- For cross-language transformations, record each provider's actual
  semantic scope and check interfaces across boundaries.

## Preservation evidence from exact source witnesses

`investigation witness` records line-precise control, data and state
**source candidates** against an existing claim. A later `investigation
plan` includes already recorded witness IDs for that claim. This makes
the preservation obligation reviewable at the exact base Git revision
without pretending the source classification is runtime behavior.

During review, compare those source-bound candidate operations with the
candidate revision, then require independent executions or proofs for
material preservation claims. Neither a matching witness text nor a
successful syntax check establishes behavioral equivalence.

## Observed behavior comparison

After recording source witnesses and a preservation plan,
`investigation execute` can run the same explicitly trusted local
command on detached worktrees for the base and candidate commits.
The comparison is tied to the plan and pre-existing witness identities.
Matching stdout, stderr and exit status is useful test evidence,
but **not** proof that the cited internal control/data/state operations
are preserved. Differences and inconclusive runs remain visible.
See [investigation execution](INVESTIGATIONS.md).

## Verification ladder

| Evidence | What it can establish | What remains open |
| --- | --- | --- |
| Exact diff and edit replay | Which bytes changed, against which Git revision | Behavior |
| Native compiler / type checks | Supported syntactic/type properties for a configuration | Runtime equivalence |
| Existing tests | Recorded properties for exercised conditions | Untested cases |
| Targeted property / differential tests | Agreement of specific observable behavior over checked inputs | Unexplored inputs and environments |
| Trace / integration checks | Observed state and effects under recorded conditions | Other configurations |
| Formal proof, when available | Precisely stated theorem under explicit assumptions | Unmodeled implementation boundaries |

A transformation can finish as `APPLIED_UNVERIFIED`, `PARTIALLY_VERIFIED`,
`VERIFIED_WITHIN_SCOPE`, or `REFUSED`. These names describe the proposed
product-level result taxonomy, **not currently emitted wire fields**.

## Refactoring investigation example

For three similar dependency-resolution implementations:

- Investigate which callers, configurations and errors distinguish them.
- Choose a canonical owner *only after* checking differences.
- Declare preserved API behavior, deterministic ordering and errors.
- Produce a native edit plan.
- Compile and run old/new behavioral comparison with negative cases.
- Report any untested caller or environment as unresolved.

## Reuse

An accepted refactoring generates a new exact source revision. Investigations
about its predecessor remain historical evidence, while affected claims are
re-established against the new source. See [evidence lifecycle](EVIDENCE_LIFECYCLE.md)
and [verification](VERIFICATION.md).
