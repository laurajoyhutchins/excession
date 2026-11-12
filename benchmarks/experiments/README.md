# Relational experiments

`excession benchmark run-relational` (implemented in `core/src/benchmark/relational.rs`) measures repeated source-relation queries over an
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
core/target/debug/excession benchmark run-relational \
  --out /tmp/relational-small \
  --sizes 8 --fanouts 1,2 --revisions 2 --repetitions 1
```

The smoke workload uses 8 source modules, two fanout configurations,
two commits and one repetition; no timing threshold is used as a CI gate. The runner and source-written oracle are Rust; generated Python files are the language-under-test fixtures.
A broader run should use more sizes, graph densities and revisions, and
report raw receipts rather than a single headline number.

`summary.json` records per-revision index time and growing index size,
exact adjacency equality, query counts, status counts, median timing, total
query time and stdout bytes. `trials.jsonl` records each comparison against
the independent oracle. Neither graph nor view has a special oracle channel.

**Still incomplete:** scalable independent replay on several real repositories,
truly incremental view maintenance and resource accounting for process memory.
The retained relation runner measures repeated queries and changed histories,
not automatic caching. The bounded full indexed-query audit below is now
implemented separately.

## Additional retained experiments

- `checked_tree_ab.py`: historical-vs-current Rust binaries, identical Git
  and SQLite relations, canonical byte parity and representative tamper
  refusals. Dedicated `checked-tree-ab.yml` remains an isolated long run.
- `real_fixtures.py`: source-bound projections on pinned h11 and upstream Git
  commits, with an independent, bounded import-edge inventory in
  `dependency_oracle.py` and `js_import_oracle.cjs`. Missing and invented SQL
  dependency rows must be caught by the oracle. An adversarial Python/JS
  fixture covers absolute `src/` imports and CJS/TypeScript index files.

These suites verify exact in-index static import edges for their named fixtures,
not all runtime dependencies in arbitrary code. Dynamic imports, unsupported
import-resolution configurations, and external packages remain outside the
coverage denominator. See [dependency scope](../../docs/DEPENDENCY_COVERAGE.md).

## Independent indexed-query verifier

`independent_audit.py` is an independent full Git/SQLite replay
implementation, complementary to the already established Python/JavaScript
source-import candidate oracle. It compares exact accept/refuse outcomes,
full manifest commitments, and per-command cost against Excession's native
`view replay deps` rather than comparing an unauthenticated graph
query to an authenticated view.

```sh
python3 benchmarks/experiments/independent_audit.py benchmark \
  --bin core/target/debug/excession --out /tmp/replay-audit --repetitions 2
```

Intact manifest and eight corruption variants are checked on a pinned h11
Git bundle. A passing result does not authenticate arbitrary dependency
relations that are self-consistent but absent from the source. The existing
`dependency_oracle.py` and `real_fixtures.py` supply
the separate source-level falsification experiment.
