# Excession on Excession: consolidation audit

## Source and scope

This refactor was guided by the archived output of Excession's own
[successful advisory self-analysis run](https://github.com/laurajoyhutchins/excession/actions/runs/43169892817)
on the Git commit `d105606625ea3c7c85e01ce224ce873399cbcb70`
(November 12, 2026). Its artifact is `excession-self-analysis`, including
`summary.json`, `scip-report.json`, `scip-audit.json`, and independently
replayed syntax and dependency observations.

The pinned report observed 194 source files, 49 conservative dependency
candidates, and 11,854 Git-bound cross-file Rust SCIP references across
87 indexed documents. It proposed 60 advisory refactoring hypotheses,
of which 54 were triaged as likely crate-root/module structure links.
**None of these numbers measures complete dependencies, build equivalence,
or authorization to skip tests.**

## What the six review candidates actually contain

All six originally unclassified reciprocal-reference candidates pair
a Rust module file with code belonging to the same module:

| Module | Child file | Candidate observation |
| --- | --- | --- |
| `transactions.rs` | `transactions/execution.rs` | Module declaration and calls; child references shared `Spec` |
| `transactions.rs` | `transactions/verification.rs` | Module declaration and calls; child references shared `Spec` |
| `projection.rs` | `projection/composition.rs` | Module declaration and calls; child references parent `Plan`/`Entry` |
| `projection.rs` | `projection/workspace.rs` | Module declaration and calls; child references parent `Plan`/`Entry` |
| `bin/excession.rs` | `bin/excession/graph.rs` | Real parent response/presentation dependency; removed below |
| `evidence_store.rs` | `evidence_store_tests.rs` | Explicit test submodule and references to fixtures/core types |

Excession now recognizes narrow *parent/child filesystem shapes* **only if**
rust-analyzer emits a corresponding parent-to-child module-symbol reference.
These pairs are retained with every original identity and reference, but
prioritized as structural/module integration rather than automatically
presented as a real dependency cycle or refactoring recommendation.
Removing a module declaration from a fixture, changing the indexer, or
using an unrelated path prevents this classification.

## Refactor driven by an actual SCIP reverse edge

The archived SCIP candidate for `core/src/bin/excession.rs` versus
`core/src/bin/excession/graph.rs` contained eight child-to-parent symbol
references: six to the parent's `json()` adapter and two to `Response`.
Unlike a source-bound module declaration, these are avoidable
presentation-layer dependencies.

The graph router now returns `Result<json::Value>`, importing its CLI
options explicitly from its sibling `cli` module. The top-level CLI
alone converts the domain JSON to `Response::Json`. This removes the
child's references to parent presentation symbols without changing the
graph CLI's output or graph persistence semantics. A CLI regression test
initializes a SQLite schema and verifies that `graph ls` still returns the
expected canonical JSON response.

The historical six-review-candidate count is a baseline, **not** the
expected post-refactor count. A fresh SCIP analysis should no longer show
the `excession.rs ↔ graph.rs` reciprocal parent-response pair; newly
indexed source may produce other hypotheses independently.

## Consolidation

A separate source inspection identified repeated Rust implementations of
`sha256(canonical_unsigned_json)` followed by insertion of
`report_id`, `candidate_id`, `claim_id`, or `change_id`.

One authoritative `json::Value::stamp_id` now performs that operation,
and `content_id_matches` validates already-stamped objects. Call sites
in agent context, change inventory, impact, explanations, native test
workflow, discovery, kernel, refactoring, and graph receipt ingestion use
the shared operations. Existing field names, canonical serializer,
SHA-256 implementation, coverage labels, and producer-trust claims are
unchanged.

## Fresh PR self-analysis resource boundary

The [November 14 exact-head attempt](https://github.com/laurajoyhutchins/excession/actions/runs/43638679564)
successfully produced the rust-analyzer SCIP index, but Excession refused its
decoded input because it exceeded the old 16 MiB ingestion cap. The retained
artifact contains **20,950,375 bytes of decoded SCIP JSON**, **121 indexed
Rust documents**, and **113,823 occurrences**. Independent inspection of the
decoded index found 80 reciprocal source pairs before advisory triage.

To process this known bounded input without disabling limits, the SCIP JSON
ingestion cap is now 32 MiB and the occurrence cap 150,000. The 16 MiB
*emitted report* cap remains unchanged, as do maximum source bytes, document
count, and candidate count. Excession must still refuse over-budget input;
higher capacity does not justify stronger semantic claims. The fresh
run must be repeated at the final PR commit before merging.

## Acceptance

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked --all-targets`
- Exact-head portable CLI, native selectors, impact and source-evidence tests
- Graph CLI JSON regression; no parent `Response` or `json()` use in child router
- Advisory self-analysis of the PR's merged Git tree, with replay identity
  checks, Git-bound SCIP index, and explicit unresolved relationship counts
- The JSON tests compare new stamped wire bytes with the pre-refactor
  canonical result, refuse double stamping, and reject mutated observations
- The SCIP tests distinguish parent/child module integration from reciprocal
  references without an actual module declaration

**Limits:** The historical SCIP evidence is an observation from a specific
older commit. Its structural candidates are not proof that modules are
well designed. Forcing a new triage classification does not prove runtime
equivalence. Full regression tests and source/receipt replay remain necessary.

Existing open PR #66 proposes a mechanical Rust module-layout move.
This consolidation changes behavior-preserving primitives and advisory
triage, not physical module placement. Rebase or restack as needed rather
than silently treating the two proposals as independent completed merges.
