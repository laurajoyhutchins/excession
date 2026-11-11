# PR 6: Semantic concept reconciliation and review events

Status: event protocol + TDD scaffold only.

## Review scope

A review event targets an exact immutable concept revision and source/model scope. It records reviewer identity as a claim; it does not itself attest external identity or prove behavior. Accepted definitions and aliases are *reviewed interpretations*, not verified program equivalence.

Operations: ACCEPT, REJECT, MERGE, SPLIT, ALIAS, SUPERSEDE. Merges and splits require referenced concepts and must retain previous versions. Untrusted model output can only propose decisions. Review history is append-only and independently replayable.

## TDD requirements

- Merge and alias operations must not infer equivalence from names alone.
- Rejected claims stay visible and cannot be reissued as accepted without a new event.
- Reviewed concepts at old Git revisions are not silently approved at future revisions.
- A changed source receipt marks dependent reviews stale for the changed scope.
- Conflicting merge/split chains report conflict; deterministic replay cannot silently choose one.
- No review event changes the behavioral verification status of a separate source claim.
- SQL readback must reconstruct the exact event history with no omitted or extra events.

The schema is a shape contract only; Rust must validate actor authorization, event chaining, source integrity, and event/canonical IDs before publication.
