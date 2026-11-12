# SCIP evidence in Excession concepts

The `concepts scip-*` commands consume JSON decoded by the **official** `scip print --json` CLI. They do not run a native indexer, interpret raw protobuf, rewrite imports, or require a hosted ontology service.

```sh
scip print --json index.scip > scip.json
excession concepts scip-preview --repo "$REPO" --rev "$COMMIT" --index scip.json
excession concepts scip-compile --repo "$REPO" --rev "$COMMIT" --index scip.json --db analysis.sqlite
excession concepts scip-audit --repo "$REPO" --rev "$COMMIT" --index scip.json \
  --db analysis.sqlite --snapshot "$SNAPSHOT"
```

For a nested index, supply `--source-prefix core`. A nested SCIP metadata `project_root` must agree with the exact checked-out root plus this prefix, or the operation refuses.

## Authority and meaning

The existing `scip_refactor` Git-binding scan is the sole index validation path. It checks the pinned commit/tree, tracked regular-file blobs, optional embedded source text, native symbol IDs, indexer metadata, occurrence ranges and resource caps. **No indexer is independently reexecuted.** Only uniquely resolved cross-file symbol references within the input index become `SCIP_REFERENCE` observations. Missing, local, external and ambiguous definitions cannot be merged by matching names. Unresolved counts and input scope remain explicit.

`SCIP_SYMBOL_REFERENCE` is not `IMPORTS`, `CALLS`, `FFI_BOUNDARY`, behavioral equivalence, compiler verification, or an authenticated runtime relationship. Reciprocal symbol references produce **proposed, untested architecture hypotheses** with separate versioned concept IDs. One-way references do not. All imported snapshots have `PARTIAL` coverage.

Existing `refactor scip-suggest`, static import semantics, and SQL source authority remain unchanged. Source identity binds the evidence to Git, but does not establish whether an upstream tool accurately indexed every real relationship.
