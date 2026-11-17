# Investigations: the unit of understanding

**Status:** a Git-bound investigation ledger now has demand-driven source
discovery, source-linked claim synthesis, recorded unresolved questions, and
direct-source refresh across revisions. `excession investigation explain`
builds a compact evidence-backed *projection*, not an independent proof of
behavior. See [agent CLI](AGENT_CLI.md) for the underlying discovery tools.

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

The ledger can now select candidate source and record direct-source refresh,
but cannot infer verified call paths, automatically resolve questions,
re-establish transitive behavior, or provide native source edits. Its event and report wire contracts are
[`investigation-event.v1`](../contracts/product/investigation-event.v1.schema.json)
and [`investigation.v1`](../contracts/product/investigation.v1.schema.json).

## Active investigation workflow

Existing investigations can select source evidence, retain questions, assemble
a cited explanatory draft and reassess direct source identities:

\`\`\`sh
# Persist the existing agent-locate candidate report and incomplete reading plan
excession investigation explore \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision 1 \
  --limit 12 --index-segments 8

# Record what still needs to be determined
excession investigation question \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision 2 \
  --question "What happens when recovery fails?" \
  --reason "No failure-path execution or test receipt has been inspected"

# Once source has actually been read, record a CLAIM and reference its event ID
excession investigation synthesize \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision REVISION \
  --narrative "The recovery path delegates to the retry handler under the inspected guard" \
  --claim-events CLAIM_EVENT_ID

# Compare direct claim-source identity against a separately committed revision
excession investigation refresh \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision REVISION \
  --candidate CANDIDATE_COMMIT

# Compose the source map, hypotheses, narrative, gaps and open obligations
excession investigation explain \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID
\`\`\`

Every mutating command requires the current \`--expected-revision\`. Replace
\`REVISION\` with the actual current ledger revision, which increments on
each successful append. \`explore\` reuses the existing \`agent locate\`
retrieval model and configurable index budgets. The same `--budget-file`,
`--max-file-bytes`, `--max-segment-bytes`, and `--max-segment-files`
options are available on both `agent locate` and `investigation explore`.
A source above an assigned budget is reported as incomplete rather than
silently excluded from the coverage claim. Its reading plan is **not** a program
trace; missing matches and incomplete index segments are reported as gaps.
The stored report omits volatile local cache paths but retains its stable
report digest and Git-bound candidate identities.

\`synthesize\` requires existing, individually source-bound \`CLAIM\` events.
The narrative is **author-provided**, with claim IDs and assumptions exposed;
the code does not manufacture behavioral conclusions. \`question\` keeps
unresolved questions open. \`explain\` returns source identities, claims,
narratives, retrieval gaps, preservation plans and independent verification
receipts in \`excession.investigation-explanation.v1\`.

\`refresh\` recomputes the exact Git change inventory and classifies each
claim's source path as \`DIRECT_SOURCE_UNCHANGED\`,
\`DIRECT_SOURCE_CHANGED\`, or \`DIRECT_SOURCE_UNAVAILABLE\`. **Every class
still has \`semantic_impact: UNRESOLVED\`**, including unchanged blobs:
callers, build flags and dependencies may change elsewhere. It retains
the complete count of changed paths and never claims transitive
behavioral equivalence. An absent source is not silently treated as an
unchanged implementation.

The current implementation is a **single-step exploration plus human/agent
source review loop**, not autonomous semantic analysis or a whole-program
correctness verifier. It does not execute repository code, invent call
relationships, or mark a change safe to merge.

The [explanation projection schema](../contracts/product/investigation-explanation.v1.schema.json)
and [event schema](../contracts/product/investigation-event.v1.schema.json)
describe the current wire representations.

## Source-level behavioral witnesses

An investigation can now attach **exact, independently replayed Git text** to
a source-grounded claim. It classifies the relevant line span as a control
condition, data transfer, or state mutation, while leaving its semantic
interpretation explicitly **unverified**.

```sh
# Record after a CLAIM has been accepted and note its returned event ID
excession investigation witness \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision REVISION \
  --claim-event CLAIM_EVENT_ID \
  --first-line 12 --last-line 15 --kind STATE_MUTATION \
  --interpretation "These lines appear to update the retry counter"
```

Allowed kinds: `CONTROL_CONDITION`, `DATA_TRANSFER`,
`STATE_MUTATION`. A witness must refer to an earlier claim whose exact
Git file and blob are authenticated. The Rust implementation streams and
SHA-1-verifies the **entire** Git blob using bounded memory, and captures
only the selected 1–32 source lines (at most 8,192 bytes). It computes
the source-text SHA-256 and independently replays the same source span
during every ledger audit. It refuses nonexistent lines, oversized spans,
unsupported source encodings and forged or mismatched claim references.

A witness proves **what text existed at that source revision**, not that a
particular branch was executed, data flowed at runtime, or state changed.
Its status remains `SOURCE_TEXT_VERIFIED_INTERPRETATION_UNVERIFIED`;
`runtime_verified` is always false. Claim interpretations remain
attributed to the author. No code is executed during witness capture.

`investigation explain` exposes these under `behavior_witnesses`.
When a subsequent `investigation plan` references that claim, the
plan includes the **witness event IDs already present at plan creation**
as a reviewable preservation checklist, not as evidence of successful
verification. The existing `investigation verify` retains independent
test/build obligations and does not upgrade witness status. Source changes
must still be re-examined at the candidate revision.

Actual control-flow reachability, runtime dataflow and verified state
transitions require independent compiler or execution evidence that
corresponds to these source spans. This remains the next acceptance
boundary, not a capability inferred from syntax.

## Trusted-local differential execution (observed outcomes)

The explicit opt-in `investigation execute` command runs the **same
host executable and arguments** on two independently created, detached,
Git-pinned worktrees for a witness-backed preservation plan. No shell
interpolation, checkout of the user's working tree, or implicit execution
occurs. The executable must be an absolute trusted host path outside
the mutable source checkout. This operation **does execute repository
code**, so it is not a sandbox and should only be used with reviewed
code in an isolated environment.

```sh
excession investigation execute \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision REVISION \
  --plan-event PLAN_EVENT_ID --candidate CANDIDATE_COMMIT \
  --program /usr/bin/python3 \
  --argv-json '["-B","src/counter.py"]' \
  --timeout-ms 30000 --max-output-bytes 1048576 \
  --trusted-local true
```

The `PLAN_EVENT_ID` must reference a plan with at least one prior
`investigation witness` for its claimed source. The output records:
- Authenticated base/candidate commit and tree identities, and the relevant
  witness event IDs that existed when the plan was made.
- The local host executable SHA-256, arguments, fixed minimal process
  environment policy, and configured time/output budgets.
- Exit status and exit code, stdout/stderr byte counts and SHA-256 digests.
  Raw output is **not** persisted in the investigation ledger.
- A comparison: `AGREEMENT_OBSERVED`, `DIFFERENCE_OBSERVED`, or
  `INCONCLUSIVE` if either run times out, exceeds output budgets, or
  does not exit normally.

Both sides run with the same arguments and a minimal locale/PATH environment.
Runtime OS state, dynamic libraries, clock, network, environment-dependent
behavior, and external effects are **not** attested or made deterministic.
Child process trees are not sandboxed. Excession only captures a local
observation; it does not claim a signed independent execution oracle.
A matching result for a single invocation does **not** establish all-input
behavioral equivalence, preservation of a particular internal state mutation,
or that a refactoring is ready to merge. The source witness remains
`runtime_verified: false`; the plan remains `OPEN`.

The full report is retained in an append-only `EXECUTION` event.
`investigation explain` exposes its original report under
`execution_comparisons`, joined to the preservation plan and source
witnesses. Replaying the ledger revalidates Git identities, referenced
event history, content hashes, and comparison consistency **without
re-executing source code**.

The [execution report schema](../contracts/product/investigation-execution.v1.schema.json)
defines the wire contract. Independent CI attestations, real runtime
instrumentation, repeated input corpora and proof obligations remain
future extensions, not implied by the observed outcome comparison.

## CPython runtime event observations

The optional `investigation trace` command reuses the **existing**
trusted-local detached-worktree execution runner and records events from
a reviewed Python tracing adapter. This is the first instrumented runtime
slice; the evidence protocol is versioned, but **other languages are not yet
instrumented**.

```sh
excession investigation trace \
  --repo /work/project --db /tmp/investigations.sqlite \
  --id INVESTIGATION_ID --expected-revision REVISION \
  --plan-event PLAN_EVENT_ID --candidate CANDIDATE_COMMIT \
  --program /usr/bin/python3 \
  --driver /trusted/excession/tools/python_trace_driver.py \
  --script src/counter.py --argv-json '[]' \
  --timeout-ms 30000 --max-output-bytes 1048576 \
  --max-trace-events 2000 --max-trace-bytes 1048576 \
  --trusted-local true
```

The script must be tracked in both revisions. The tracing adapter is a
trusted host script **outside the target checkout**. Source worktrees are
detached and temporary; observation files are stored outside the checkout,
and source cleanliness is checked after execution. The operator must
explicitly opt into running the repository's code, which remains unsandboxed
and may have arbitrary external effects.

Each side retains CPython-reported `CALL`, `LINE`, `RETURN` and
`EXCEPTION` events, Git-authenticated file identities, line numbers,
function names and changes in sampled local values. Primitives such as
bounded integers and booleans can be represented directly; strings and
byte strings carry length and SHA-256 rather than raw values. Objects and
mutable containers are **opaque**, so in-place mutation is not independently
observed. A local-variable change is associated with the last traced line
before the next trace callback, not an abstract semantic assignment edge.

Base witness source-line hits are counted but **do not prove** a branch's
condition evaluated true or that a claimed mutation had the expected effect.
The candidate's source lines may shift, so it is not automatically paired
to the old witness by line number. `MATCHING_REPORTED_EVENTS` means the
recorded event sequences matched for the selected invocation;
`DIFFERING_REPORTED_EVENTS` means they differed, potentially due to a
line-number shift or a true behavioral change; `INCONCLUSIVE` covers
missing, interrupted or truncated trace streams. Output comparison remains
a **separate** field, so identical stdout cannot conceal a difference in
the observed internal event sequence.

Tracing is best effort: target code can disable the tracer, values may be
opaque, C extensions and subprocesses are outside the CPython frame stream,
and no independent completeness attestation exists. A reported complete
trace is not a proof of complete coverage. Both source witnesses and
preservation plans remain unresolved; no outcome authorizes merging.

All events and source identities are retained in an append-only `TRACE`
record. `investigation show` replays their provenance and conservative
comparison, while `investigation explain` serves them under
`runtime_traces`. Neither audit nor explanation executes repository code.
See the [runtime trace contract](../contracts/product/investigation-runtime-trace.v1.schema.json).

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
