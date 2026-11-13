# Compose source and configured-build observations

The ordinary analysis/impact CLI can reuse previously imported **native**
producer evidence. No parser, compiler or build tool is run by the aggregate
query. Each source relationship retains its native predicate, producer receipt,
observation identity and exact Git revision.

Excession discovers imported native evidence by default from the selected
SQLite store, without running the original indexers. The store must be outside
the repository. Explicit snapshot and graph IDs are an optional scope override.

## Native SCIP reference

For an index prepared using a language-native SCIP indexer, import its
`scip print --json` representation with the existing command:

```sh
excession refactor scip-ingest --repo "$REPO" --rev "$REV" \
  --index "$SCIP_JSON" --db "$DB"
# No snapshot ID needed for normal analysis.
excession analyze "$REPO" --rev "$REV" --db "$DB" --json
```

For nested indexes, supply `--source-prefix core` when importing them.
Later discovery recovers that prefix from the indexer's project_root declaration. Excession independently replays the stored original SCIP bytes
against the selected Git commit and refuses a mismatched tree or observation.

For revision impact, independently import indexes from **each** revision:

```sh
excession impact "$REPO" --base "$BASE" --head "$HEAD" --db "$DB" --json
```

`SCIP_REFERENCE` is an observed source-level symbol reference. Its reverse
reachability is a `CANDIDATE_PATH` for change impact, not proof of runtime
dependency, breakage, or safe test skipping. All source references are pinned
to the original blob IDs; the original native symbol and occurrence remain
available in the relationship's `native_observation` and retained snapshot.

## Configured-build graph

Import graphs through the existing `graph import`, `graph assemble` or
`graph kbuild` commands. All imported graph configurations for the selected
revision remain independently labeled:

```sh
excession analyze "$REPO" --rev "$REV" --db "$DB" --json
```

For narrowed impact use explicit `--base-graph-id` and/or `--head-graph-id`. The aggregate
returns authenticated `configured_observations` containing graph identity,
source path/blob, related target/entity, native relation, status and evidence.
These relations are **not converted into source-to-source dependencies**.
Changed-source membership is included as impact context, never as a
candidate affected source by itself.

`producer_coverage` preserves producer-specific limitations; all reports
remain partial. Queries reject oversized selections instead of silently
discarding evidence.

## Replay and limits

`impact --output FILE` retains the exact producer selections. A later
`excession explain --report FILE --id CLAIM --db DB` recomputes source
evidence and refuses stale, missing, corrupted or mismatched receipts.

Current scope: up to 24 SCIP snapshots and 24 configured graphs per revision,
bounded observation counts, Git SHA-1 checkout support, and read-only
composition. No native indexers or build tools are executed during analysis.
No new graph store or alternate authority was introduced.

## Discovery, composition, and conflicts

An existing SQLite evidence store can be inspected without running a parser:

```sh
excession inspect producer-candidates --repo "$REPO" --rev "$REV" --db "$DB"
excession analyze "$REPO" --rev "$REV" --db "$DB" --json
excession impact "$REPO" --base "$BASE" --head "$HEAD" --db "$DB" --json
```

`analyze` and `impact` discover evidence **by default**, including the
default Excession cache database when no `--db` is given. A missing database
is not created just to search for observations. Without native evidence,
supported static analysis still runs, and other languages retain explicit
coverage gaps rather than disappearing.

Multiple SCIP indexes are composed only where they cover disjoint Git paths.
If indexes overlap, source-reference impact edges touching the overlapping
paths are excluded. The conflicting paths are reported under
`producer_selection_plan.blocked_paths` and the coverage gaps. Unaffected
source references from those indexes remain usable.

All imported configured-build graphs remain independently represented with
their exact graph IDs, target and configuration. Multiple observations of the
same target produce a `MISSING_CONFIGURATION` coverage gap, but do not block
independent SCIP and static analysis. Configured source-to-target membership
is never promoted into a source-to-source dependency.

Explicit `--scip-snapshot` / `--graph-id` (and revision-specific impact
variants) narrow the selection intentionally; they do not silently union with
other database observations. Unknown, corrupted and stale selected evidence is
refused. The removed `--auto-evidence` switch is no longer needed.

`producer_selection_plan` records the exact selected identities, derived
conflict paths and coverage gaps. Saved impact reports replay **only those
recorded identities**. Adding new evidence to SQLite cannot retroactively
change a previously saved impact explanation.

Semantic correctness, full dynamic dependency coverage and build execution
are **not** inferred from these producer observations.
