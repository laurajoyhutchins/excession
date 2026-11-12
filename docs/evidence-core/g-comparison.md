# contracts: coverage-qualified revision and artifact evidence comparison

**Status:** initial contract, design notes, and structural negative-first tests only. No feature implementation or verification claim.

**Tracking:** #80, #96, #102; align concepts query PR #92. **Target base:** `main`.

## Purpose
Comparison separates subject correspondence, each observed facet, and the coverage needed to justify unchanged or changed statements.

## Normative invariants
- Absence of evidence cannot justify UNCHANGED_OBSERVED without appropriate comparison scope.
- One known difference stays CHANGED despite uncertainty in unrelated facets.
- Name/address/order-only correspondence is insufficient; preserve native symbol and artifact identities.

## Public wire contract
`contracts/evidence-core/comparison.v1.schema.json` uses JSON Schema Draft 2020-12. ODCS metadata will be consolidated with the shared evidence dataset contract during implementation; avoid creating another ODCS/validation authority.

## Test-driven implementation
`python3 -m unittest discover -s tests -p 'test_ec_g_comparison.py' -v`

Structural positive/negative tests execute when the repo's existing `jsonschema` dependency is installed. The actual Rust behavior is intentionally marked skipped, and therefore **not acceptance**.

**Completion obligation:** Implement deterministic Git revision comparison with coverage-aware one-sided absence and known-difference negative fixtures.

## Integration boundary
Extend the existing Rust CLI and SQLite/Git authority. Preserve `semantic-observations.v1`, `evidence-query.v1` and existing concept/product drafts. Do not revive the retired `excession research` entry point.
