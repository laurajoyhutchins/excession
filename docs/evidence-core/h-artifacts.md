# contracts: source-to-binary build provenance and artifact digest binding

**Status:** initial contract, design notes, and structural negative-first tests only. No feature implementation or verification claim.

**Tracking:** #80, #84 and existing configured build graph work; avoid new compiler authority. **Target base:** `main`.

## Purpose
Content-addressed artifacts with explicit build evidence level, configuration and declared vs observed distinction.

## Normative invariants
- Retain Git commit/tree/blob evidence and SHA-256 artifact digest as separate identities.
- A build recipe is never upgraded into an observed build execution; source-to-artifact binding requires qualifying execution evidence.
- Reuse Cargo/Clang/CMake native inputs and configured graph; isolated workers required for untrusted builds.

## Public wire contract
`contracts/evidence-core/build-provenance.v1.schema.json` uses JSON Schema Draft 2020-12. ODCS metadata will be consolidated with the shared evidence dataset contract during implementation; avoid creating another ODCS/validation authority.

## Test-driven implementation
`python3 -m unittest discover -s tests -p 'test_ec_h_artifacts.py' -v`

Structural positive/negative tests execute when the repo's existing `jsonschema` dependency is installed. The actual Rust behavior is intentionally marked skipped, and therefore **not acceptance**.

**Completion obligation:** Rebuild a pinned Rust fixture, compare actual output digest to claimed artifact and reject mismatched or recipe-only attribution.

## Integration boundary
Extend the existing Rust CLI and SQLite/Git authority. Preserve `semantic-observations.v1`, `evidence-query.v1` and existing concept/product drafts. Do not revive the retired `excession research` entry point.
