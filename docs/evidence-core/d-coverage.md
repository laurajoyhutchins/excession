# Facet-scoped coverage and conservative negative-evidence handling

**Status:** Rust coverage assessment and regression tests are committed to draft [#119](https://github.com/laurajoyhutchins/excession/pull/119). Exact-head CI and a real native producer/inventory oracle remain required before full acceptance.

## Existing authority

`core/src/kernel_coverage.rs` remains the independently audited whole-Git-inventory implementation. It proves accounting for every eligible Git source within the partitioned scope, not configured-build, reference or runtime dependency closure. This PR does not replace or duplicate that source inventory walker.

`core/src/evidence_coverage.rs` defines the smaller, reusable **assessment layer** for all producers. Its purpose is to describe what a producer examined, not what Excession wishes it had examined.

## Rust behavior

A `Coverage` report binds one SHA-1 Git revision shape, one content-derived producer receipt identity, a typed observation facet, an explicit selection and configuration, and an optional declared finite universe.

It tracks examined, unsupported and excluded population counts separately from unresolved semantic observations, truncated execution, and qualitative limitations.

- `COMPLETE_DECLARED_SCOPE` requires a finite declared total equal to examined count, no unsupported/excluded/unresolved observations, no truncation and a resolved configuration.
- `PARTIAL` retains scoped observations with gaps, exclusions, unresolved configuration or truncation.
- `UNKNOWN` applies when no bounded population or observed facts are available.
- `UNSUPPORTED` is explicit for a producer facet that cannot be observed.
- An unknown/unbounded universe can never establish completeness.
- Count overflow or more accounted subjects than the declared total is refused.

**A complete declared inventory is not proof of absent relationships.** The `absence_is_proven` API intentionally remains false until E3 supplies a separate predicate-specific, independently audited closed-world oracle. A full Git file inventory does not prove that all calls or symbol references were found.

## Contracts and tests

- `contracts/evidence-core/coverage.v1.schema.json`: facet-scoped report format with finite/unbounded scope restrictions and negative-promotion refusal.
- `contracts/evidence-core/coverage.v1.odcs.json`: quality metadata only, not proof of coverage.
- `tests/test_ec_d_coverage.py`: positive and negative contract tests.
- `tests/test_ec_odcs_d_coverage.py`: dataset catalogue consistency.
- Rust inline tests: an empty/partial SCIP index cannot certify absence; unsupported, excluded, unresolved and truncated cases cannot become complete; invalid counts/configurations are refused; deterministic reports preserve facets and explicit limits.

## Next integration

1. Run exact-head Cargo + Python/ODCS verification.
2. Add a thin adapter from **successful** `kernel_coverage::audit` results to the source-inventory facet, without reimplementing Git partition audits.
3. Feed qualified coverage into the existing `excession explain` and `impact` answers; do not create a parallel query engine.
4. Compare against independent real SCIP/native producer data before allowing a negative claim about any semantic relation.

Preserve `semantic-observations.v1`, `evidence-query.v1`, the auto-ontology PRs #88-#95, product #101 and the retired `excession research` status.
