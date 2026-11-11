# contracts: predicate-specific claim verification and non-promotion rules

**Status:** initial contract, design notes, and structural negative-first tests only. No feature implementation or verification claim.

**Tracking:** #81, #96, #97 and security validity PR #87. **Target base:** `main`.

## Purpose
Evidence-bound claims have named obligations; source replay, analyzer correctness, and observed execution are separate dimensions.

## Normative invariants
- Claims declare predicate-specific evidence requirements and separate source, semantic, runtime and impact authority.
- A producer report match cannot establish scanner finding validity; standalone behavioral ground truth belongs to PR #87.
- Audit must not silently promote references to calls or review approval to behavior proof.

## Public wire contract
`contracts/evidence-core/verification.v1.schema.json` uses JSON Schema Draft 2020-12. ODCS metadata will be consolidated with the shared evidence dataset contract during implementation; avoid creating another ODCS/validation authority.

## Test-driven implementation
`python3 -m unittest discover -s tests -p 'test_ec_e_verification.py' -v`

Structural positive/negative tests execute when the repo's existing `jsonschema` dependency is installed. The actual Rust behavior is intentionally marked skipped, and therefore **not acceptance**.

**Completion obligation:** Rust verification rejects unsupported predicate promotions, invalidated inputs and bogus positive scanner verdicts.

## Integration boundary
Extend the existing Rust CLI and SQLite/Git authority. Preserve `semantic-observations.v1`, `evidence-query.v1` and existing concept/product drafts. Do not revive the retired `excession research` entry point.
