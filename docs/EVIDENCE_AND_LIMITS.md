> This page explains the **implemented evidence guarantees**.
> The planned claim/investigation lineage is specified separately in
> [evidence lifecycle](EVIDENCE_LIFECYCLE.md) and
> [investigations](INVESTIGATIONS.md). No new verification state should
> be inferred from a static source relationship.

# Evidence and limits

Excession is a way to inspect evidence about source code. Its reports are
useful only when the **source identity, producer, scope, and unresolved
obligations** remain attached to the result.

## Four different questions

| Question | What can establish it | What does *not* establish it |
| --- | --- | --- |
| Which source bytes were examined? | Matching Git commit, tree, path, mode, and blob OIDs | A repository URL or filename alone |
| Which relationship was observed? | A named producer's original observation and receipt, bound to that source | A suggested edge without its provenance |
| Was the report faithfully reconstructed? | Replaying the exact source objects and the supported evidence transformation | A successful parse or command exit by itself |
| Does a program behave correctly? | Independent relevant checks, or a proof with explicit assumptions and scope | A Git hash, static dependency edge, or green evidence-only workflow |

These questions must not be compressed into a single “verified” label.

## Source identity

For tracked, regular source files, the relevant identity is:

```text
repository + commit OID + tree OID
           + path + entry mode + blob OID
```

Commit and tree OIDs identify immutable Git objects; the path and entry
mode identify *which* object is used. A rename, deletion, submodule,
symlink, or configuration change can have distinct semantics. Code that
cannot authenticate its claimed source identity must refuse or mark the
observation unsupported, not infer a replacement.

Git is authoritative for source. SQLite records derived relationships and
evidence and can be rebuilt; it is not a parallel authority for source
bytes. A local cache hit is not an independently checked receipt.

The implementation begins in [Git source access](../core/src/source/git.rs),
[source indexing](../core/src/index.rs), and
[evidence identity](../core/src/evidence/identity.rs).

## Observation producers

Excession reads multiple kinds of external observations.

| Producer / input | Supported kind of observation | What remains unproved |
| --- | --- | --- |
| Python and JavaScript/TypeScript static analysis | Candidate imports and related source references | Runtime dispatch, completeness, correctness |
| SCIP and rust-analyzer | Language-native indexed symbols and reference positions | That a call executes or a dependency is behaviorally relevant |
| Compiler and configured-build observations | Declared inputs and configuration-scoped relationships | Behavior for other build configurations |
| SARIF from CodeQL or Semgrep | Scanner findings with original rule and source references | Vulnerability truth or exploitability |
| Test and build receipts | Declared executed checks and outcomes, if independently supplied and validated | Coverage of every relevant behavior |

Each producer has its own validity domain. Excession cannot convert
a reported SCIP reference, a matching path, or a scanner claim into a
cross-language proof by relabeling it.

See [observed relationships](OBSERVED_RELATIONSHIP_ANALYSIS.md),
[configured graphs](CONFIGURED_GRAPHS.md),
[build provenance](evidence-core/build-provenance.md), and
[security finding validity](SECURITY_VALIDITY.md).

## Candidate analysis and coverage

`agent locate` ranks Git-bound lexical matches. `agent context`
returns bounded source context, relationship evidence and possible tests.
`agent verify` compares candidate and base revisions and preserves
unresolved obligations. `impact` emits a compact source-witness forest
rather than enumerating all possible paths.

A report may be **partial** because the indexed source scope is
limited, a producer is unavailable, a partition is not refreshed,
an external configuration was not examined, or a list was truncated.
Partial coverage is a property of the *specific analysis*, not a
general statement about the repository.

A returned source candidate is not an assurance of relevance.
An unlisted source is not guaranteed unaffected. A suggested test is
not a license to skip other checks. The existing GitHub evidence contract
requires `ready_to_merge: false` and
`safe_to_skip_unlisted_tests: false`.

See [agent commands](AGENT_CLI.md),
[source-impact algorithm](COMPACT_IMPACT.md), and
[versioned GitHub evidence schema](../contracts/agent/github-evidence.v1.schema.json).

## What GitHub actually verifies

Two GitHub-native routes are intentionally separate:

- [On-demand source queries](GITHUB_NATIVE_ANALYSIS.md) accept an
  owner-authorized, bounded request for an allowlisted public repository
  and exact Git commit. The runner executes the real Rust CLI and retains
  its original JSON report, report checksum, source identities, and
  refusal diagnostics.
- [PR evidence](GITHUB_EVIDENCE.md) retains a base/head envelope,
  original native verification and context reports, source witnesses,
  provenance, and unresolved obligations. It does **not** execute
  the analyzed repository's tests just because evidence was requested.

An Actions job marked successful establishes only that the stated
workflow completed. Verify its exact head SHA, tool source revision,
artifact name, report schema, Git identities, hash and uncertainty.
Artifacts have limited retention; an expired artifact or runner cache
is not durable evidence. SHA-256 detects content changes, not
publisher authenticity by itself.

## Refusals and unknowns

A sound response to missing evidence is one of:

1. **REFUSED**: necessary input, source identity, parser or native report
   did not validate. Do not publish success-shaped output.
2. **PARTIAL / UNSUPPORTED**: the observation is outside a supported
   scope or producer. Retain the scope and reason.
3. **OPEN_OBLIGATIONS**: supported evidence was recorded, but further
   independent checks or review are needed.

These have different meanings and must not be silently converted to
“passed”. The original operation-specific JSON schema remains the
interface contract.

## Claims requiring additional work

True semantic equivalence, complete dependency coverage, exploitability,
refactoring safety, a complete test-selection guarantee, and arbitrary
cross-language behavior all need independent acceptance or a scoped proof.
See [security validity](SECURITY_VALIDITY.md), [native test selectors](NATIVE_TEST_SELECTORS.md),
and the [research index](REFERENCE.md).

The product must make unsupported behavior visible. Adding another
analyzer, report wrapper, or optimization does not by itself settle a
missing proof obligation.
