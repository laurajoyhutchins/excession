# contracts: append-only unknowns and evidence-resolution event history

**Status:** initial contract, design notes, and structural negative-first tests only. No feature implementation or verification claim.

**Tracking:** #103, #96; align concept review PR #93. **Target base:** `main`.

## Purpose
Unknowns are persistent investigation questions with explicit obligations, conflict history and versioned decisions.

## Normative invariants
- Unknowns retain their question, source scope, all supporting and contradicting receipts, and event history.
- Append-only compare-and-swap is mandatory for updates, including conflict and reopened states.
- Review and administrative dismissal do not establish independent behavioral correctness.

## Public wire contract
`contracts/evidence-core/unknown-event.v1.schema.json` uses JSON Schema Draft 2020-12. ODCS metadata will be consolidated with the shared evidence dataset contract during implementation; avoid creating another ODCS/validation authority.

## Test-driven implementation
`python3 -m unittest discover -s tests -p 'test_ec_f_unknowns.py' -v`

Structural positive/negative tests execute when the repo's existing `jsonschema` dependency is installed. The actual Rust behavior is intentionally marked skipped, and therefore **not acceptance**.

**Completion obligation:** Persist concurrent unknown updates, reject stale expected revisions, and reopen resolution after contradictory new evidence.

## Integration boundary
Extend the existing Rust CLI and SQLite/Git authority. Preserve `semantic-observations.v1`, `evidence-query.v1` and existing concept/product drafts. Do not revive the retired `excession research` entry point.
