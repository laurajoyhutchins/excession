# Relational experiments

`relationships.py` measures repeated source-relation queries over an
independently specified synthetic Python import graph. The graph definition
produces Git-tracked sources, and separately produces the expected adjacency
and reachable source paths. The Excession index is **not** used to derive the
oracle. Every indexed revision is checked for exactly matching dependency
edges before retrieval timings are accepted.

Dimensions:
- **size:** number of Python modules;
- **fanout:** number of forward imports per module;
- **revisions:** number of real Git commits, with a changed backward edge
  on each commit after the first;
- **repetitions:** independent repeated query calls against already indexed
  revisions, with deterministic randomized order.

All variants include a tracked test module importing two source modules.
The runner stores Git bundles to reconstruct the generated histories exactly.

```sh
python3 benchmarks/experiments/relationships.py \
  --bin core/target/debug/excession \
  --out /tmp/relational-small \
  --sizes 8 --fanouts 1,2 --revisions 2 --repetitions 1
```

The smoke workload uses 8 source modules, two fanout configurations,
two commits and one repetition; no timing threshold is used as a CI gate.
A broader run should use more sizes, graph densities and revisions, and
report raw receipts rather than a single headline number.

`summary.json` records per-revision index time and growing index size,
exact adjacency equality, query counts, status counts, median timing, total
query time and stdout bytes. `trials.jsonl` records each comparison against
the independent oracle. Neither graph nor view has a special oracle channel.

**Incomplete experiments:** an equivalent-guarantee tamper/audit baseline
and a truly incremental reuse baseline. The current runner measures repeated
queries and changed histories, not whether Excession automatically caches
views, and does not assert that it does.

## Additional retained experiments

- `checked_tree_ab.py`: historical-vs-current Rust binaries, identical Git
  and SQLite relations, canonical byte parity and representative tamper
  refusals. Dedicated `checked-tree-ab.yml` remains an isolated long run.
- `real_fixtures.py`: source-bound projections on a pinned h11 bundle and
  selected real upstream Git commits, with independent Git blob/mode/path
  checking and explicit unsupported-language limits.

These existing suites verify source identity and selected corruption refusals.
They do **not** constitute an equal-guarantee comparison against an independent
Git/SQLite security auditor or a complete semantic relationship oracle.
