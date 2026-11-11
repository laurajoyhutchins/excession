# Excession productization contract scaffold

**Status:** 11 open draft PRs containing specifications, JSON Schema and initial test-first fixtures. The new public commands and release are not yet implemented.

This work sits alongside, but does not merge with, semantic auto-ontology PRs #88–#95. The latter propose immutable concept/claim snapshots, review, query, annotation and evaluation. In particular:
- #88 sets up concept/model identities and semantics.
- #94 proposes an `analysis-link.v1` that can annotate an existing impact/refactor candidate.
- #95 specifies independent concept-enrichment evaluation.

**Contract boundary:** Existing `semantic-observations.v1`, `evidence-query.v1`, SCIP, Git/SQLite, configured graph, refactor and scanner evidence retain their original meaning and authority. A concept link is `ANNOTATION_ONLY`. A concept membership or reviewed alias does not become an import, compiler input, runtime reachability, scanner validity or proven safe edit. Reuse ontology snapshot/link IDs; do not create another ontology store.

## PR dependency graph

```text
#101 public CLI + source/evidence/coverage primitives (main)
  ├── #106 repository discovery ── #107 one-command analyze
  ├── #108 committed analysis with dirty worktrees
  ├── #109 Git change inventory ── #110 evidence-aware impact ── #111 explanation/audit
  ├── #112 independent acceptance
  ├── #113 installed archive acceptance
  ├── #114 contributor/security onboarding
  └── #115 release provenance
```

All ten downstream branches are **siblings based on #101**, not a deep stacked chain. They must be retargeted to `main` after #101 merges. The arrow above is a **functional implementation order**, not their current Git parent ancestry.

## Test strategy

Each PR adds:
1. A Draft 2020-12 JSON Schema contract in `contracts/product/`.
2. Design/acceptance documentation in `docs/product/`.
3. Active positive and invalid wire-shape tests under `tests/test_product_*.py`.
4. A **deliberately skipped** native/real-Git behavior placeholder.

Run:

```sh
python3 -m pip install -r requirements/research.txt
python3 -m unittest discover -s tests -p 'test_product_*.py' -v
```

A green schema test proves the fixture shape only. Skips do not prove runtime behavior, Git replay, source coverage, semantic accuracy, practical usefulness or platform portability. The existing independent oracle fixtures and real Git suites must provide those proofs when implementation lands.

## Contracts

- `common.v1`: Git SHA-1 source references, producer-scoped evidence refs, coverage and optional ontology annotation refs.
- `cli-result.v1`: basic user-facing command outcome.
- `discovery.v1`, `analysis.v1`, `working-tree.v1`: initial user experience.
- `change-inventory.v1`, `impact.v1`, `explanation.v1`, `audit-result.v1`: actionable, auditable change analysis.
- `acceptance-case.v1`, `installed-smoke.v1`, `onboarding-example.v1`, `release-manifest.v1`: independent release discipline.

**Do not** publish these draft schemas as supported public API until exact-head acceptance and compatibility review. Existing stored receipts must not be silently reinterpreted.

## Gate before product release

Run installed archive → inspect a genuinely unfamiliar pinned repository → produce source-grounded change impacts → explain one path → independently audit input/source and reported semantics → measure time/memory and report unsupported cases. CI must preserve the distinction between schema conformance, evidence replay, semantic truth and declared coverage.
