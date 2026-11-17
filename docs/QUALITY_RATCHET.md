# Cumulative quality ratchet

**Implemented scope:** a fast, standard-library-only Git diff gate plus a weekly,
main-branch-only native Rust autofix PR. It is not an autonomous semantic
refactoring engine and does not replace the broad [Verify](CI.md) workflow.

## The ratchet model

Accepted rules live in [`quality/ratchet.json`](../quality/ratchet.json).
Each immutable rule has a stable ID, file globs, a line-level Python regular
expression, a rationale, and an evidence reference. The gate uses **exact
Git commits**, never uncommitted source or a mutable debt counter.

For each changed tracked path, compare the multiset of matching source lines
at the base and candidate revision. Existing violations are grandfathered,
but newly introduced matching occurrences fail. Unchanged files need not be
rescanned. A content-preserving rename carries its baseline. Rule removal or
modification fails; additive rules can be introduced without immediately
penalizing untouched historical code.

Run locally:

```sh
python3 -m unittest discover -s tests -p test_quality_ratchet.py -v
python3 tools/ci/quality_ratchet.py check --base HEAD^ --candidate HEAD
```

The report includes the exact base/candidate commits, reviewed file count,
rule count, new violations and removed/altered rule IDs. Missing source blobs,
bad expressions, and malformed policy refuse rather than passing.

The main Verify workflow checks both the checker at the **previous accepted
revision**, when one exists, and the checker at the candidate revision. This
makes silently weakening the Python implementation less effective. Changes to
the workflow itself still depend on repository branch protection, code review,
and mandatory checks; self-testing YAML is not a security boundary.

### What this establishes and does not establish

- **Established by the checker:** no increase in configured line-pattern
  occurrences *in changed tracked paths* against the specified Git base;
  no deletion or modification of previously registered rules.
- **Not established:** that old code is good, that the regex detects every
  semantic violation, that the tree was fully analyzed, that passing tests
  guarantee behavioral equivalence, or that a malicious workflow change
  could not bypass a repository without branch protection.
- **Scope limits:** regex patterns inspect physical lines, not ASTs; multiline
  constructions, semantic aliases and changes outside rule globs are unknown.
  Rules should be narrow, reproducibly tested and backed by an independent
  oracle where semantics matter.

This is a **negative regression gate**, not a numerical quality score.
Increasing the rule count is not itself progress. A rule earns admission by
detecting a concrete class of regression without blocking valid programs.

## Turning a lesson into a rule

1. Record the observed defect, the exact Git revision, and a counterexample.
   Reuse a relevant [technical-notebook](https://github.com/laurajoyhutchins/technical-notebook)
   explanation where appropriate, but never treat the note as an oracle.
2. Create a negative test that fails for the original defect and a positive
   control that accepts the intended behavior.
3. Prefer the language's native compiler, test runner or analyzer for semantic
   assertions. Add a narrow regex rule only for an independently inspectable
   textual invariant.
4. Append a stable ID and its rationale/evidence reference to the catalog.
   Existing rule objects remain immutable. If a rule becomes obsolete,
   replace/supersede it through an explicitly reviewed change to the checker
   and protection policy; never quietly delete it.
5. Run fast fixtures, the ratchet and the complete native CI at the exact
   candidate commit. Keep unsupported, failing and partial evidence visible.
6. After merge, the new test or rule becomes a permanent obligation of future
   revisions. Review signal quality and remove obsolete duplication through
   explicit approval, not automatic policy erosion.

## Automatic changes and authority

The [quality improvement workflow](../.github/workflows/quality-improvements.yml)
runs weekly or on owner dispatch **from main only**. It executes pinned
`rustfmt` and Clippy's machine-applicable `--fix` using the repository's
locked Rust configuration. It refuses edits outside tracked Rust source,
checks formatting, Clippy, library tests, and the fast ratchet suite, commits
a candidate to a new bot branch and opens a **draft PR**. It explicitly
uses GitHub `workflow_dispatch` to run Verify on the candidate head,
because bot-token-created PR events may not automatically trigger checks.
Full existing PR admission and human review remain necessary. It never auto-merges.

The workflow requires GitHub Actions permission to create PRs and available
runner minutes. If that setting or Actions billing prevents execution, no
unattended improvement can be claimed. The implementation is deliberately
limited to native, deterministic edits because Excession's own
[transformation contract](TRANSFORMATIONS.md) does not yet establish
automatic equivalence or merge authority for larger changes.

The existing [self-analysis](CI.md) workflow supplies advisory structural
candidates; it does not authorize automatic deletion, extraction, dependency
rewrites or refactoring. Those candidates must first acquire source-bound
preservation obligations and independent verification receipts. Only then
could additional allowlisted edit producers be promoted into the ratchet.

## Cost policy

The fast line gate and fixture tests run before dependency installation on
every PR. Native autofix is weekly/on-demand, **not per commit**. Deep
self-analysis and large source sweeps remain advisory. Do not convert
timeouts, missing tools, or partial evidence into passing reports.

## Automatic integration of bounded native improvements

The [versioned improvement recipes](../quality/improvement-recipes.json)
and [autonomous improvement loop](AUTONOMOUS_IMPROVEMENTS.md) turn
explicitly documented lessons into native edit generation. Unlike merely
rejecting regressions, the loop can propose or integrate concrete source
changes. Auto-integration is available only to hard-allowlisted, narrowly
checked Rustfmt transformations and graph-preserving Cargo patch updates.
Every candidate still needs independent exact-head Verify and GitHub source
evidence, and GitHub's ordinary merge restrictions remain authoritative.
