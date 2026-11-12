# Source-bound SCIP evidence: import, explain and replay

This workflow connects Excession's existing decoded-SCIP refactoring analysis to its immutable evidence store. It keeps native SCIP symbol identities and the original raw index. A SCIP reference is **not** a verified call, import or build dependency, and missing references are **not** proof of absence.

## Produce native SCIP evidence

Use an actual language-native indexer and the upstream `scip print --json`. Run indexers and builds in an isolated workspace when the project is untrusted.

```sh
rust-analyzer scip ./core --output /tmp/excession.scip
scip print --json /tmp/excession.scip > /tmp/excession.scip.json
```

The first release accepts one externally decoded JSON index per import, maximum 4 MiB. This restriction is due to the current immutable in-database original-receipt limit, not a claim that larger SCIP indexes are invalid.

## Import and inspect

```sh
COMMIT=$(git rev-parse HEAD)
excession refactor scip-ingest --repo . --rev "$COMMIT" \
  --index /tmp/excession.scip.json --source-prefix core --db /tmp/evidence.sqlite \
  > /tmp/import.json
```

Read the returned `snapshot_id`, then:

```sh
excession inspect scip-evidence --db /tmp/evidence.sqlite \
  --snapshot "<snapshot_id>" --focus core/src/lib.rs \
  --repo . --rev "$COMMIT" --source-prefix core

excession refactor scip-store-audit --repo . --rev "$COMMIT" \
  --db /tmp/evidence.sqlite --snapshot "<snapshot_id>" --source-prefix core
```

- **`scip-ingest`** invokes Excession's existing Git-bound decoded-index parser, validates all index document paths, Git blobs and SCIP ranges, then independently validates each stored source subject by Git commit/tree/path/blob/mode.
- **`inspect scip-evidence`** is a read-only, bounded query. It verifies complete snapshot membership before reporting up to 256 matching source-to-destination SCIP references per page. The output includes original symbol identities, incoming/outgoing direction, original evidence IDs, scope status and a `next_cursor` when more observations are available. With `--repo`, `--rev`, and optional `--source-prefix`, the command also replays Git evidence from the retained raw index inside one consistent SQLite read snapshot before returning an authenticated answer.
- **`scip-store-audit`** reads the original index bytes from the audited SQLite receipt and recomputes the same Git-bound observations against a requested pinned Git revision. It refuses a changed revision or missing/modified evidence without requiring that the original external index file still exist.

The `--focus` argument is an exact repository-relative path, not an arbitrary graph query. Omitting it returns a bounded first page. Follow a response's `next_cursor` with `--after <cursor>` to retrieve the remaining matching observations, without reusing cursors across snapshots or focus scopes. The result reports total matching observations, page size, and explicit truncation. A cursor that is missing from the selected scope is refused.

The source-authentication flags `--repo` and `--rev` must be supplied together. An optional `--source-prefix` is only valid with both. If omitted, the query remains an integrity-audited but **not Git-authenticated** inspection; its `source_authentication` status says so explicitly.

```sh
# Fetch the next page, using the next_cursor value from the previous response.
excession inspect scip-evidence --db /tmp/evidence.sqlite \
  --snapshot "<snapshot_id>" --focus core/src/lib.rs \
  --repo . --rev "$COMMIT" --source-prefix core \
  --after "<next_cursor>"
```

## Source, evidence and behavioral authority

A successful `scip-store-audit` establishes that Excession can independently replay its interpretation of the **retained index** against the specified Git revision. It does **not** establish that the external indexer executed at the asserted time, that the indexer discovered every reference, or that SCIP semantic relationships accurately represent runtime calls.

The snapshot manifest remains marked `source_authentication: NOT_CHECKED` because a plain storage audit cannot replay Git by itself. Only the separate `scip-store-audit` performs the Git replay. An ordinary read-only explanation advertises `REQUIRES_GIT_REPLAY_FOR_CURRENT_QUERY`. An explanation requested with matching `--repo` and `--rev` instead returns `GIT_OBJECTS_REPLAYED_FOR_QUERY` after independently authenticating source objects within the same SQLite read transaction.

Coverage is explicitly `PARTIAL` or `UNKNOWN` for SCIP. This version records document and unresolved-reference counts, but does not claim a closed-world universe. An empty index or zero matching observations produces an empty **observed evidence** answer, never a negative relationship proof, automatic safe refactor, or authorization to skip tests.

The original `refactor scip-suggest` / `scip-audit` v1 reports are unchanged. Both the original hypothesis reporter and the new import use the same bounded parser. Import stores *every matched cross-file symbol reference*, including references that do not form a reciprocal or cross-language refactoring candidate.

## Validation

- `core/tests/evidence_scip_integration.rs`: isolated real Git repository, matching native decoded-index shape, importer/query/independent replay, source-authenticated query, pagination across 301 references, missing reference uncertainty, forged source input and SQLite tampering.
- The existing `benchmarks/experiments/scip_fixture_migration.py` and manual `SCIP source-evidence fixtures` workflow independently generate actual upstream SCIP indexes on pinned real open-source repositories. The new importer is to be exercised through this existing workflow, rather than another mandatory heavy CI job.
- Existing `core/tests/refactor_contracts.rs` is the backwards-compatibility regression for legacy SCIP refactoring.

In the current draft PR, the exact-head tests and upstream real-index acceptance are separately tracked. Passing structural-schema tests or retaining one raw report is not enough to claim producer correctness.
