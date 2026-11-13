# Compose source and configured-build observations

The ordinary analysis/impact CLI can reuse previously imported **native**
producer evidence. No parser, compiler or build tool is run by the aggregate
query. Each source relationship retains its native predicate, producer receipt,
observation identity and exact Git revision.

The user must select the evidence explicitly. The SQLite database must exist
outside the repository. This avoids accidentally mixing unrelated native
compiler configurations and stale source indexes.

## Native SCIP reference

For an index prepared using a language-native SCIP indexer, import its
`scip print --json` representation with the existing command:

```sh
excession refactor scip-ingest --repo "$REPO" --rev "$REV" \
  --index "$SCIP_JSON" --db "$DB"
# Copy the returned snapshot_id.
excession analyze "$REPO" --rev "$REV" --db "$DB" \
  --scip-snapshot "$SCIP_SNAPSHOT" --json
```

Use `--source-prefix core` when the index is rooted inside a pinned
subproject. Excession independently replays the stored original SCIP bytes
against the selected Git commit and refuses a mismatched tree or observation.

For revision impact, independently import indexes from **each** revision:

```sh
excession impact "$REPO" --base "$BASE" --head "$HEAD" --db "$DB" \
  --base-scip-snapshot "$BASE_SCIP" \
  --head-scip-snapshot "$HEAD_SCIP" --json
```

`SCIP_REFERENCE` is an observed source-level symbol reference. Its reverse
reachability is a `CANDIDATE_PATH` for change impact, not proof of runtime
dependency, breakage, or safe test skipping. All source references are pinned
to the original blob IDs; the original native symbol and occurrence remain
available in the relationship's `native_observation` and retained snapshot.

## Configured-build graph

Import a graph through the existing `graph import`, `graph assemble` or
`graph kbuild` commands. Select its exact graph ID:

```sh
excession analyze "$REPO" --rev "$REV" --db "$DB" \
  --graph-id "$GRAPH_ID" --json
```

For impact use `--base-graph-id` and/or `--head-graph-id`. The aggregate
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

Current scope: one SCIP snapshot and one configured graph per revision,
explicit selection, Git SHA-1 checkout support, bounded observations, and
read-only integration. It does not execute language indexers, join cross-language
symbol semantics, independently verify compiler correctness, or establish
complete build closure. No new graph store was introduced.

## Deterministic producer discovery

An existing SQLite evidence store can be inspected without running parsers,
compilers or creating another index:

```sh
excession inspect producer-candidates --repo "$REPO" --rev "$REV" --db "$DB"
excession analyze "$REPO" --rev "$REV" --db "$DB" --auto-evidence --json
excession impact "$REPO" --base "$BASE" --head "$HEAD" --db "$DB" --auto-evidence --json
```

Discovery lists the revision's persisted SCIP and configured-build candidates
with the original producer identity, target, configuration and source prefix.
It selects a facet **only when there is exactly one candidate for that facet**.
Different target/configuration graphs are deliberately not coalesced. Multiple
SCIP indexes, including duplicate-language indexes with distinct source scopes,
are ambiguous, not ranked by recency. Use explicit producer IDs after reviewing
`inspect producer-candidates` to resolve conflicts.

A native index's nested source prefix is derived only from its original
`metadata.project_root` declaration within the selected Git checkout.
Selected evidence is subsequently replayed against Git; discovery alone does
not certify semantic correctness. The database must already exist and be outside
the source checkout. A repository without imported evidence retains its
ordinary static-index behavior; unsupported languages are not silently treated
as semantically indexed.

Automatically selected IDs are written to the resulting analysis and impact
reports. Saved impact reports replay the *exact selected IDs*, not a future
reinterpretation of whatever happens to be the newest evidence in SQLite.
`--auto-evidence` and explicit producer IDs cannot be mixed in one invocation.
