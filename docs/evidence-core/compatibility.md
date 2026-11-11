# Compatibility and authority map

This document is the reconciliation plan for [semantic concepts PR #88](https://github.com/laurajoyhutchins/excession/pull/88), the downstream semantic series #89–#95, and [product contract PR #101](https://github.com/laurajoyhutchins/excession/pull/101). **Do not merge a second generic evidence model from any of these drafts.**

## Normative ownership

| Meaning | Existing source of truth | Shared-core extension | Downstream |
| --- | --- | --- | --- |
| Git source identities | `core/src/git.rs`, `repository`/`revision`/`tree_entry` | Typed source identity + exact object replay (#117) | Concepts source refs; product source refs |
| Native syntax/import candidates | `semantic-observations.v1` | Original typed predicate/receipt, without promotion | Concepts observation batch #89 |
| Unverified compiler inputs | `evidence-query.v1`, `native_boundary.rs` | Typed source/producer receipt with distinct authority | Concept compilation #91; impact #80 |
| Runtime witnesses | `evidence-query.v1` | Execution subject and observed facet coverage | Product evidence reference |
| Configured graph relations | `configured_store.rs` | Reuse its typed neutral projection, retain native kind | Impact #80 and explain #81 |
| Semantic concepts | Draft `concepts/model.v1` (#88) | Derived claims linked to immutable evidence | Compilation #91, snapshots #90 |
| CLI presentation | Draft `product/common.v1` (#101) | Read-only references to evidence/coverage IDs | Analyze #79, explain #81, impact #80 |
| Security candidates | `security_ingest.rs` and SARIF/Semgrep reports | Candidate claim with no implicit validity | Behavior oracle PR #87 |

## Versioning and migration

1. Preserve exact historical `semantic-observations.v1` and `evidence-query.v1` report structure and field meanings. Do not rewrite or replace the existing source formats in place.
2. Treat the new evidence-core wire formats as **optional projections** until full Rust validation and SQLite readback are implemented.
3. Maintain existing original producer/native identifiers in addition to Excession-derived IDs.
4. Define one shared code path for digest and Git source replay in Rust; retain product and concept JSON schemas as separate *views*, not duplicate persistence stores.
5. Use JSON Schema Draft 2020-12 for interchange, ODCS 3.2 for quality metadata, and Rust/SQLite for referential and semantic invariants.
6. Avoid duplicating `core/src/configured_store.rs` or introducing another graph/ontology server. `excession research` is retired.

## No-promotion matrix

| Original observation | Must never be promoted to |
| --- | --- |
| SCIP symbol reference | Verified runtime call |
| Syntax-derived import candidate | Complete dependency closure |
| Compiler-declared input | Observed compiler execution |
| Self-hashed runtime receipt | Independent execution attestation |
| CodeQL/Semgrep report | Valid vulnerability / exploit |
| Reviewed semantic concept | Proven behavior equivalence |
| Same source blob | Same runtime behavior under all configurations |
| Same binary name | Build from a specified Git revision |

## Acceptance

Before either #88 or #101 becomes authoritative, include an automated compatibility check that reads both schemas and verifies field ownership, versioned source semantics and the no-promotion invariants. Until those PR branches are reconciled and the Rust behavior is executed, this is a **normative design decision**, not an implementation claim.

Fast schema tests belong on the commit checkpoint; native-tool and independent-producer oracles remain explicit deep verification.
