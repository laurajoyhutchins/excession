# Source-impact witness forest

`excession impact` computes one rank-certified witness for each distinct
source reachable from a changed source in the observed relationship graph.
It does not enumerate alternative simple paths. In layered dependency graphs,
there can be exponentially many such paths and only linearly many sources.

The sole report format is `excession.impact.v3`. There is no legacy `effects`
list, path claim selector, or compact-mode switch.

## Source witness

Each `candidate_source_impacts` entry contains:

- `entity`: the Git-pinned source identity, including revision, path, blob,
  tree and mode
- `next_hop`: one original source-to-target edge with the producer's
  relationship type and evidence unchanged
- `distance_to_change`: the shortest observed path length to a changed seed
- `assessment: CANDIDATE_PATH`: graph reachability, not behavioral breakage

A changed seed has implicit rank zero. A witness at rank `d > 0` points to
the changed seed when `d = 1`, or to another source of rank `d - 1` at
the same revision. This makes every witness independently reconstructible
and guarantees a finite explanation in the *observed* graph, even with cycles.

The multi-source reverse BFS traverses each discovered vertex once.
With ordered maps and sets, the reachability traversal has
`O((|V| + |E|) log |V|)` time and `O(|V| + |E|)` working space.
These bounds exclude Git indexing, language analyzers, report serialization,
and any missing real dependency relationships.

## Source and coverage bounds

`candidate_source_count` counts all observed non-seed reachable sources
across the two revision graphs. The output contains up to 10,000 entries;
`candidate_source_enumeration_complete` is false and a `TRUNCATED` gap
is recorded if that limit is reached.

Java compiler-supplied source references have a separate
`native_reference_impacts` inventory and completeness flag.
These observations remain explicitly unattested; they are not silently
folded into static-import or SCIP observations.

`coverage.status` is `PARTIAL` and
`safe_to_skip_unlisted_tests` is always false. Completeness of traversal
over an *observed graph* is not completeness of dependencies in a program.

## Replay and explanation

```sh
excession impact . --base BASE_COMMIT --head HEAD_COMMIT \
  --output /tmp/impact.json --json

excession explain . --report /tmp/impact.json \
  --source src/pkg/handler.py --source-commit HEAD_COMMIT --json

excession explain . --report /tmp/impact.json --all-sources --json
excession explain . --report /tmp/impact.json \
  --source src/pkg/handler.py --source-commit HEAD_COMMIT --audit
```

Use `--source-commit` if the same source path occurs in both revision
inventories. Otherwise the selector explicitly refuses ambiguity.
`--all-sources` emits at most 128 reconstructed explanations and
refuses a larger batch rather than silently truncating it.

Explanations recompute the entire v3 report from pinned Git objects and the
stored producer selection, then require canonical equality before returning
source witnesses. A saved report is not a trusted cache. Forged or stale
evidence is refused. The source witness's content hash is exposed as
`claim_id` in the explanation, but it is not a legacy path claim and is not
used to select explanations.

A successful audit authenticates source replay, not behavioral correctness.
The output explicitly reports `semantic_correctness: NOT_EVALUATED`.

## Performance evidence

The earlier manual Requests experiment established that omitting alternative
path enumeration reduced the serialized impact report from 2,235,217 to
37,680 bytes and reduced one command's measured wall time from about 1.325 to
1.246 seconds. This was a one-host result, not a general speedup guarantee.
The post-refactor format removes the now-unused empty path metadata as well.

Source reanalysis and Git/SQLite authentication still dominate one explanation
in that experiment. Improvements to that cost require independent
phase-level measurements and unchanged refusal behavior. Track the remaining
work in issue #244.
