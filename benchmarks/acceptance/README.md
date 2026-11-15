# Independent change-impact acceptance

This is a **reusable source-evidence test**, not a promise that static imports
represent every runtime dependency. It compares the installed Excession CLI
with a separately implemented Git + CPython AST oracle on at least two real
repositories, then checks that agent verification preserves its obligations.

## Run

Requires Python 3.11+, Git, network access to the pinned public GitHub
repositories and a previously built Excession executable.

    cargo +1.99.1 build --locked --release --manifest-path core/Cargo.toml --bin excession
    python3 -m unittest discover -s tests -p 'test_impact_acceptance_suite.py' -v
    python3 benchmarks/experiments/impact_acceptance_suite.py \
      --manifest benchmarks/acceptance/impact_cases.json \
      --bin core/target/release/excession \
      --out-dir /tmp/excession-impact-acceptance

The exit status is nonzero if **any** case fails. The harness retains
`summary.json` and one directory per case with `case-result.json`,
`independent-evaluation.json`, and `agent-verification.json` when the
respective stages ran. It retains failed-case diagnostics even when later
cases pass. CI uploads these records regardless of the result. The suite
summary includes the manifest digest, executable digest, exact source
commits/trees, measured costs and per-case evidence digests. Digests do not
authenticate who ran the tests.

The [multi-repository workflow](../../.github/workflows/impact-acceptance-suite.yml)
runs on relevant pull requests and by manual dispatch. Its generic job
builds Excession **once**, then reuses the same pinned executable and
assessment logic for each case.

## Add a repository

Add a case to [impact_cases.json](impact_cases.json):

| Field | Meaning |
| --- | --- |
| `id` | Unique lowercase, hyphenated case identifier |
| `repository` | GitHub `owner/repo`, not an arbitrary execution URL |
| `base`, `head` | Distinct **exact** 40-character Git commit IDs |
| `roots` | Python source root directories, for example `["src","testing"]` |
| `expected_changed_paths` | Exact list from the independently pinned Git diff |
| `must_include_head_paths` | Known candidate dependents expected under the independent AST import graph |
| `minimum_head_affected_paths` | Nonzero guard against a trivially empty oracle |

Only Python static imports are currently supported. New languages require
a genuinely independent language-specific oracle and an explicit manifest
schema revision, not substitution of Excession's own observations as gold.

The initial cases are the pinned pytest `CollectError` fix and the pinned
h11 Content-Length header fix. Both changes are authentic commits of
independently maintained projects. An additional case must keep the test
suite at two or more distinct repositories. Invalid/missing SHA pins,
ambiguous repository identifiers, path traversal, unsupported path kinds,
missing requested files and malformed evidence fail closed.

No source checkout is trusted as a mutable build: the harness obtains
Git objects for both exact revisions into a fresh, detached, clean
temporary directory. The oracle reads original Git blobs and constructs
literal Python import edges independently of Excession's parser, SQLite
graph and SCIP producer. Excession is run on the **same** pinned objects.

## What must pass

For **each** repository:

1. The exact changed-file set and requested source paths match Git and
   independently resolved AST imports.
2. Excession's distinct-source candidate set equals the independent
   reverse-reachable set **in both revisions**. Every source, hop, blob,
   tree, mode, rank-decreasing route and producer receipt is checked by
   the independent oracle. Missing and extra candidates both fail.
3. A capped `effects` list may omit alternate explanations **only** with
   an explicit `TRUNCATED` gap. It cannot excuse missing source witnesses,
   invented claims or false positives.
4. `excession agent verify` recomputes the candidate commit and tree,
   reports `OPEN_OBLIGATIONS`, denies `ready_to_merge`, denies permission
   to skip other tests and retains missing test/build evidence obligations.
5. Cost is reported rather than optimized away: separately measured
   Git/AST oracle time, Excession impact+index time, agent verification
   time, output bytes, unresolved import count, gap reasons, and complete
   source-candidate counts. Changed-file-only and one-hop AST candidates
   are descriptive baselines, not interchangeable execution workloads.

The original [pytest-specific experiment](../../.github/workflows/pytest-independent-impact.yml)
remains a separate **behavioral and native-tool sidecar**. It reproduces
the historical exception, checks the unchanged affected test, compares a
prewarmed pytest-testmon baseline and tests stale/mismatched genuine
execution receipts. Those checks should not be pretended to exist in
repositories that lack a valid behavioral probe.

## Interpretation

A passing case establishes equality of two implementations of a **bounded
static Python import relation** for that particular pinned source history.
It does **not** establish all imports or semantic dependencies are known,
that any changed source actually breaks a dependent, that tests can safely
be skipped, that a code change is correct, or that a caller-supplied test
receipt has an authenticated producer. Unresolved imports and coverage
gaps are reported even when the independently measurable static subset
matches. No statistical inference is made from two fixtures.
