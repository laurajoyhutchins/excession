# SCIP fixture migration: evidence, not parity

The six existing real-repository fixtures and the sixteen pinned historical
revisions are valuable **source inputs**, not an expected semantic graph that
a new indexer must reproduce. Excession uses the official SCIP decoder and
language-native SCIP indexers without reimplementing symbol resolution.

Run on pinned versions of h11, ItsDangerous, Click, Requests, Axios, and the
mixed Python/JavaScript/TypeScript adversary. The optional historical sweep
includes four revisions each of h11, Click, Requests, and PostCSS from the
original Git bundles. Historical patches, behavior probes, and reference
receipts are neither rewritten nor deleted.

## Reproduce

Build the Rust binary, install the official SCIP v0.10.0 CLI and the native
`scip-python@0.6.6` and `scip-typescript@0.4.0` indexers in an isolated
trusted environment, then run:

```sh
python3 benchmarks/experiments/scip_fixture_migration.py \
  --bin core/target/release/excession \
  --out /tmp/scip-fixture-results.json \
  --evidence-dir /tmp/scip-fixture-evidence \
  --trusted-local

# Include all 16 pinned historical revisions, including PostCSS:
python3 benchmarks/experiments/scip_fixture_migration.py \
  --bin core/target/release/excession --historical \
  --out /tmp/scip-history.json \
  --evidence-dir /tmp/scip-history-evidence --trusted-local
```

The dedicated [SCIP fixture workflow](../../.github/workflows/scip-fixtures.yml)
runs the six-fixture suite on relevant PRs and supports the historical sweep
through manual dispatch. The official SCIP decoder release archive is verified
by SHA-256, and the tool versions are pinned in CI.

To replay previously generated decoded indexes without executing an indexer,
replace `--trusted-local` with `--decoded-dir PATH` containing
`<fixture>-python.json` or `<fixture>-typescript.json`.

## Observed six-fixture run

The pinned upstream SCIP tools and Git-bound replay passed on
[workflow run 43055514994](https://github.com/laurajoyhutchins/excession/actions/runs/43055514994).
That run produced 37,825 SCIP occurrences over six fixtures and preserved
decoded indexes, generated binary SCIP files, reports and audit receipts.

| Fixture | Scoped source coverage | Outcome |
| --- | ---: | --- |
| h11 | 12 / 12 | `SOURCE_BOUND` |
| ItsDangerous | 8 / 8 | `SOURCE_BOUND` |
| Click | 17 / 17 | `SOURCE_BOUND` |
| Requests | 19 / 19 | `SOURCE_BOUND` |
| Axios | 0 / 72 expected `lib` files | `PARTIAL` |
| Mixed import fixture | Python 4 / 4; JS/TS 1 / 4 | `PARTIAL` |

SCIP TypeScript indexed other source files in Axios but not the 72 scoped
`lib` files. That is a real configuration/coverage difference, not a
source-integrity failure. The fixture runner does **not** replace a
repository's existing TypeScript project configuration or synthesize
missing symbol relationships to produce apparent completeness.
Neither of the two partial fixtures is claimed to have complete coverage.

## Evidence contract

For each source language, the runner:

1. Checks out the exact Git revision specified by the original fixture.
2. Defines a bounded source universe from regular Git-tracked source files
   under the original roots. It **never** reads the legacy SQL module catalog.
3. Runs the upstream indexer on a disposable copy, not the clean source checkout.
4. Imports the upstream-decoded SCIP JSON through
   `refactor scip-suggest`, then replays using `refactor scip-audit`.
   Git-bound document paths, source blobs, spans, tree identity, raw input
   digest and report content must agree.
5. Reports all scoped missing and additional source documents, definition
   and reference counts, unresolved symbol identities and line-only bounds.

A source path appearing in the SCIP index is not enough to prove that every
symbol was discovered correctly. Even a replayed index is **not an independent
execution of the indexer** or proof of its semantic correctness.

Results are classified as:

| Result | Contract |
| --- | --- |
| `SOURCE_BOUND` | Git binding and replay succeed; all scoped tracked source files are represented and the index contains occurrences. Symbol correctness and completeness are **unproven**. |
| `PARTIAL` | The index is source-bound, but some scoped files are unindexed or the index has no occurrences. This is usable partial evidence, not a complete graph. |
| `UNSUPPORTED` | No applicable source scope or the indexer produces no source documents. |
| `INVALID` | Indexer execution, format, source binding, provenance, or audit fails. This **fails CI**, even when other fixture results are valid. |

A valid SCIP symbol reference is not automatically an import, call, build
dependency, runtime edge, or permission to skip tests. Unresolved/external
symbols are recorded without treating them as a negative observation.
Documents with an omitted optional `language` field are labeled
`unspecified`; Excession never constructs cross-language hypotheses unless
both languages were actually specified by the indexer.

### What happens to the old oracles?

The independent import oracle in `dependency_oracle.py` and the historical
behavior/projection fixtures are retained, but **not consulted as the acceptance
condition for SCIP**. They test different contracts. Existing import-specific
view behavior stays available until it has a suitable native import/build
authority. Do not silently substitute a symbol-reference graph for the
previous import dependency relation.

In particular, the unfinished SCIP-import-role-to-SQL proposal in PR #59
should not be merged merely to achieve parity. Indexer-specific `Import`
roles are not a universal complete import graph.
