# contracts: reconcile evidence-core boundaries with concepts and product scaffolds

**Status:** contract/specification + structural TDD scaffold only. No functional implementation is claimed.

**Tracking:** #96, #97, #82 and #60; reconcile PRs #88 and #101. **Design reference:** REA's evidence, coverage and comparison discipline. **Target:** current `main`, not the concept-stack branch.

## Scope
Contract ownership, compatibility, and forbidden semantic promotions. Keep Excession's existing Rust/Git/SQLite authority and versioned source report semantics; do not restore `excession research`.

## Normative rules
- Choose one shared low-level vocabulary. Reuse v1 source and evidence reports verbatim; adapt rather than rewrite.
- Keep PR #88's concept model a derived projection and PR #101's CLI result a presentation contract.
- No additional authority for Git, SQLite, static semantics, scanner validity or runtime behavior.

## Initial schema and tests
- Contract: `contracts/evidence-core/boundary.v1.schema.json` (Draft 2020-12).
- Structural TDD: `python3 -m unittest discover -s tests -p 'test_ec_a_foundation.py' -v` using the existing `jsonschema` test dependency.
- Structural tests validate schema and negative examples; intentionally skipped behavioral test remains **not accepted**.
- Future implementation acceptance: A Rust cross-schema compatibility validator proves old reports retain byte-for-byte meaning after core normalization.

## Compatibility and coordination
Preserve `excession.semantic-observations.v1` and `excession.evidence-query.v1` on disk. Reuse native SCIP, compiler, configured graph and scanner evidence, retaining original producer meaning. This PR does not modify any existing concept/product draft branch.

## Out of scope
No Rust implementation, SQL mutation, CodeQL/REA execution, CI workflow addition, inferred completeness or security-verdict promotion.
