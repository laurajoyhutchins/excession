# test/contracts: preregister evidence-core end-to-end validity and cost evaluation

**Status:** schema, documentation, negative-first structural tests only. The behavior remains **not implemented**.

**Issue coordination:** #84, #105, #96; align evaluation draft PR #95. **Base:** current `main`; do not depend on unmerged concept branches.

## User-facing requirement
Independent acceptance spanning Git source, symbolic references, build artifacts, optional REA evidence and audited conclusions.

## Normative rules
- Select independent real-repository oracles and freeze source revisions before running predictions.
- Report unsupported, unknown, truncated, incorrectly promoted and false-correspondence cases in denominators.
- Record real wall time, peak RSS and evidence size; separate fast local TDD from manual Cloud Build/decompiler/native runs.

## Schema and TDD
- `contracts/evidence-core/acceptance.v1.schema.json`: Draft 2020-12 initial wire shape.
- `python3 -m unittest discover -s tests -p 'test_ec_k_acceptance.py' -v`: positive/negative structural tests using existing `jsonschema` dependency.
- Behavior test intentionally skipped; a passing structural suite is **not** feature acceptance.
- Completion: Execute pinned h11, Rust binary/REA and Linux scope experiments; independently adjudicate false-positives and measured resource costs.

## Existing interfaces
Reuse `configured_store.rs`, `semantic-observations.v1`, `evidence-query.v1`, existing SCIP/native/scanner adapters, and the single Rust CLI. The semantic concept PRs #88–#95 and product #101 remain separate derived/presentation layers. Do not restore `excession research`.

No CI workflow, additional database, new parser, decompiler, remote service or security-verdict engine is introduced in this draft.
