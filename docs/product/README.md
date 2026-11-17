> **Historical product-contract plans.** Canonical product
> semantics now live in [product](../PRODUCT.md),
> [investigations](../INVESTIGATIONS.md),
> [transformations](../TRANSFORMATIONS.md) and
> [verification](../VERIFICATION.md). The pages here document prior
> design and scoped implementation work. Their names and schema drafts
> must not be presented as current or planned primary UI without
> checking the actual Rust CLI and accepted ADR.

# Product contracts and executable acceptance

> **Status:** These pages combine current contract notes with historical,
> PR-numbered implementation plans. Individual plans describe the scope and
> acceptance state at the time of writing, not current CLI availability.
> For supported commands see [the current CLI reference](../CLI.md),
> [quick start](../QUICKSTART.md), and [evidence limits](../EVIDENCE_AND_LIMITS.md).

Excession already implements Git-pinned discovery, analysis, revision changes,
impact, explanation, agent context/verification and evidence audit through one
Rust CLI. The source-of-truth remains Git objects plus the SQLite observation
catalog. Language-native outputs such as SCIP and SARIF remain producer inputs,
not new Excession authorities.

## What remains versioned

The retained `contracts/product/` JSON Schemas cover command results that are
emitted or intended as real interchange formats, plus shared evidence/coverage
references and release artifact digests. A schema does not establish behavioral
correctness, semantic completeness or producer authenticity. Do not promote a
draft schema to public API without comparing it against the real CLI output.

The prior `cli-result`, `acceptance-case`, `installed-smoke` and
`onboarding-example` schemas are removed: these were proposed test-only
representations rather than independent consumers of runtime JSON.
Three internal evidence-core ODCS catalogues and their skipped metadata-only
tests are also removed; the source identity, snapshot and coverage JSON Schemas,
Rust implementation and SQLite enforcement remain. The
associated Python tests verified only fabricated fixture shapes and left their
behavioral tests skipped.

## Verification instead of schema proliferation

- Unit and Rust integration tests establish implementation behavior.
- Independent Git and language-native oracles test actual source relationships.
- `tools/test_portable_archive.py` exercises installed CLI artifacts.
- `excession agent context` and `agent verify` run against this repository
  in `.github/workflows/verify.yml`; their partial coverage and unresolved
  obligations must remain visible.
- The advisory `self-analysis.yml` workflow performs deeper Excession-on-Excession
  rust-analyzer SCIP ingestion and audit on a pinned Git revision.

Never infer a safe edit, verified dependency, safe test selection or confirmed
vulnerability merely from a model annotation, source-bound reference or
successful replay.

## Remaining release gate

Run the installed binary on an independently selected, unfamiliar Git repository.
Establish the relevant positive and negative evidence using an independent
oracle, replay it, reject tampering, and measure time, peak RSS and artifacts.
Keep synthetic fixture acceptance distinct from held-out results.
