# contracts: import REA Evidence bundles without source or semantic promotion

**Status:** schema, documentation, negative-first structural tests only. The behavior remains **not implemented**.

**Issue coordination:** #96, #84, #98; REA reference integration (proposed E7). **Base:** current `main`; do not depend on unmerged concept branches.

## User-facing requirement
Optional REA import preserves the original raw evidence, native IDs, artifact binding, coverage, and limitations.

## Normative rules
- REA native ev_ IDs and raw evidence stay intact; Excession gives normalized records separate IDs.
- One missing or unsupported producer detail cannot be invented as Git binding or complete coverage.
- No embedding/decompiler/agent session framework is added to Excession; REA remains optional external tooling.

## Schema and TDD
- `contracts/evidence-core/rea-import.v1.schema.json`: Draft 2020-12 initial wire shape.
- `python3 -m unittest discover -s tests -p 'test_ec_i_rea.py' -v`: positive/negative structural tests using existing `jsonschema` dependency.
- Behavior test intentionally skipped; a passing structural suite is **not** feature acceptance.
- Completion: Import a pinned real REA Evidence v2 export, round-trip native IDs, refuse malformed paths/closure and audit artifact digest.

## Existing interfaces
Reuse `configured_store.rs`, `semantic-observations.v1`, `evidence-query.v1`, existing SCIP/native/scanner adapters, and the single Rust CLI. The semantic concept PRs #88–#95 and product #101 remain separate derived/presentation layers. Do not restore `excession research`.

No CI workflow, additional database, new parser, decompiler, remote service or security-verdict engine is introduced in this draft.
