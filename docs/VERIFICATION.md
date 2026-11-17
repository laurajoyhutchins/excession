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

No identity, static index or generic green workflow proves behavioral
equivalence. No Excession report by itself authorizes merging, deleting code
or skipping tests.

## Negative evidence

Record counterexamples, contradictory traces, failing tests, unsupported
configurations and test-selection gaps alongside positive results. A missing
or unreachable relationship in an index is not proof that a behavior is
absent. Fail closed when a necessary source identity or receipt is invalid.

## Existing versus proposed

Today, the Rust CLI supports source-bound impact/explanation audits,
`agent verify`, some native receipt checks and bounded refactoring replay.
The GitHub-native evidence workflow deliberately sets
`ready_to_merge: false` and `safe_to_skip_unlisted_tests: false`.
See [CI](CI.md), [evidence and limits](EVIDENCE_AND_LIMITS.md),
[agent CLI](AGENT_CLI.md) and [refactoring](REFACTORING.md).

The target is a persistent, question-scoped verification record joined to
investigation claims and transformation obligations. It is **not** an
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
