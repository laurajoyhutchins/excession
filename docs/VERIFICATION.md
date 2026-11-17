# Verification as a separate product outcome

An investigation describes software behavior. A transformation proposes
changes. **Verification** asks what, specifically, was established about
the explanation or change, and with what independent evidence.

## Claim-by-claim obligations

A useful verification record identifies the claim or preservation obligation,
base and candidate revision, check, input domain, relevant configuration,
receipt, outcome, and unresolved cases. It must preserve the raw check
result and distinguish:

- **Identity and replay:** exact Git source, producer bytes, output digest
  and deterministic transformation agreement.
- **Static validity:** what compilers/indexers actually observed.
- **Executable behavior:** what tests and traces exercised.
- **Semantic equivalence:** only the equivalence domain established by a
  differential oracle or formal proof.
- **Safety of application:** whether the change was actually applied to a
  trusted isolated destination, and whether independent gate(s) passed.

Source-line witnesses may authenticate the exact control/data/state text
cited by a claim, but do not prove runtime behavior. Those witnesses are
linked to preservation plans and remain `runtime_verified: false` until a
separate independently scoped verification workflow is established.

No identity, static index or generic green workflow proves behavioral
equivalence. No Excession report by itself authorizes merging, deleting code
or skipping tests.

## Directly observed differential executions

`excession investigation execute` is an explicit trusted-local runner
for the same host command against temporary base and candidate Git
worktrees. It records actual exit codes and output digests for a
witness-backed preservation plan. `AGREEMENT_OBSERVED` means only that
these observable results matched for **one configured invocation**.
`DIFFERENCE_OBSERVED` is concrete counterevidence for that invocation;
`INCONCLUSIVE` preserves timeout, output-budget or abnormal-termination
uncertainty.

The command has **no sandbox**, records the executable identity but not
a signed independent attestation, and cannot prove internal control/data
flow or all-input equivalence. It therefore never clears a plan's
preservation obligations or grants merge approval. See the
[execution contract and example](INVESTIGATIONS.md).

## Negative evidence

Record counterexamples, contradictory traces, failing tests, unsupported
configurations and test-selection gaps alongside positive results. A missing
or unreachable relationship in an index is not proof that a behavior is
absent. Fail closed when a necessary source identity or receipt is invalid.

## Existing versus proposed

Today, the Rust CLI supports source-bound impact/explanation audits,
`agent verify`, some native receipt checks and bounded refactoring replay.
`excession investigation verify` now executes `agent verify` and links
the report to an earlier preservation plan within an append-only ledger.
The GitHub-native evidence workflow deliberately sets
`ready_to_merge: false` and `safe_to_skip_unlisted_tests: false`.
See [CI](CI.md), [evidence and limits](EVIDENCE_AND_LIMITS.md),
[agent CLI](AGENT_CLI.md) and [refactoring](REFACTORING.md).

The implemented ledger persists a question-scoped `agent verify` report and
its open obligations. `investigation refresh` independently checks direct
Git source differences, always leaving transitive semantic impact unresolved. The **remaining target** is independent, claim-by-claim
behavioral verification joined to transformation obligations. It is **not** an
already-shipped `excession verify-investigation` command or schema.

## Acceptance rules

- Each reported pass points to an independently inspectable receipt.
- The executed check addresses the named claim, not merely adjacent code.
- Base and candidate source/configuration identities match the receipts.
- Failure, partial coverage and unsupported semantics remain explicit.
- A report never silently upgrades a claim based on available but
  unexecuted tests or proposed check names.
- The final claim is no stronger than the strongest applicable evidence.

For product evaluation, use independent held-out answer keys and adversarial
changes rather than counting only successful command invocations.
