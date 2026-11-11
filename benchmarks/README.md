# Excession vs simpler repository navigation: measurable benchmark

**Question:** What measurable cost or information advantage does Excession's
view machinery provide over ordinary tools? This is a falsification benchmark.
Do not score subjective "engineering task completion," "agent productivity,"
or "better understanding." Those terms are not operationalized here.

## Observable metrics

| Quantity | Operational definition | Interpretation and caveats |
| --- | --- | --- |
| Exact source-set equality | Set equality of Git-relative paths returned for a frozen query | Tests mechanical parity; **not** proof that either graph includes all real dependencies |
| Declared-path equality | Retrieved paths equal explicitly declared fixture paths | Only meaningful on tasks with separate declared paths; others are unscored |
| Raw output size | Exact bytes on stdout from each tool | Measures context *available* for consumption, not LLM tokens or actually consumed bytes |
| Output amplification | Full emitted references, distinct source paths, duplicates | Reveals how many entries the view machinery adds versus a direct query |
| Selected-output size | Byte count of normalized JSON containing selected paths | Same response shape for both arms; tests the information content after filtering |
| Candidate source bytes | Sum of Git object sizes for unique selected paths | Potential material to read, **not** bytes actually read by a model |
| Query latency | Monotonic elapsed time for an external process, median and observed p95 | Includes process startup and both implementations' verification work |
| CPU | Child user+system CPU from `resource.getrusage` | Per-process aggregate of timed child command, platform dependent |
| Index cost | Time, SHA-256 and on-disk bytes of shared source index | Shared setup cost; report amortized cost, never treat it as free for one arm |
| Tool failure/refusal | Exit status and error category, including empty and unsupported results | Report denominators and failures, never silently discard them |
| Source identity | Exact Git commit/tree and output SHA-256 | Reproducible references, not an independent semantic guarantee |

Peak per-process memory and model tokens are **not currently instrumented**, and
must not appear in claims. The graph arm runs as a fresh Python interpreter,
whereas Excession runs in Rust; latency measures end-user CLI invocation cost,
not a pure comparison of the view-layout algorithm. Process cache state is not
rigorously controlled.

## Executable calibration: graph vs derived views

Build `core/target/debug/excession` first (`make cli`). Clone the pinned
`core/fixtures/h11-crossrepo.bundle` outside the Excession checkout, then run:

```sh
python3 benchmarks/paired.py run \
  --repo /path/to/clean/h11 \
  --bin ./core/target/debug/excession \
  --tasks benchmarks/calibration-h11.json \
  --out /tmp/excession-calibration-001 \
  --name h11 --roots h11,tests --rev HEAD \
  --repetitions 3 --random-seed 20261110
```

The runner creates one Excession `module`/`dependency` SQLite index and uses
**that exact database** for both arms. The graph baseline performs bounded
breadth-first traversal in plain Python/SQLite; the view arm invokes
`excession view deps` and selects named roles from its JSON output. Both
queries are paired on the same task, revision, and index with randomized order.

There are 24 mechanically specified queries: six graph operations against four
h11 module seeds. These are **not 24 independent engineering tasks**. Four
focus queries have declared source paths; the other 20 test source-set parity,
not independent semantic correctness. An identical wrong output is still wrong.

Outputs are `metadata.json`, `trials.jsonl` and `summary.json`. The trials
record actual raw stdout bytes, SHA-256, reference counts, duplication, selected
files, Git blob-size sum, elapsed time and child CPU. Summary reports median and
observed p95 wall time and indexing amortized over 1, 5, 20, 100 queries.
Raw manifests may contain source files beyond the specific selected answer,
which is why raw versus selected output is reported separately.

The view arm does not independently replay or materialize every view. Audit
cost must be measured in a separate study against a straightforward Git/SQLite
hash-and-identity checking baseline.

## Next comparisons: fixed, observable retrieval probes

Implement the remaining arms using fixed, versioned query protocols, pinned
tools and Git commits:

| Arm | What is executed | What is measured |
| --- | --- | --- |
| Keyword | Fixed `rg` queries and bounded file reads | Exact hits, rank, bytes emitted, CPU/wall time, exit status |
| LSP | Pinned server: workspace symbols, definitions, references | Exact symbol locations, count, bytes, startup/index cost, latency, unsupported cases |
| Graph | Identical indexed edges with plain SQL/BFS | Path sets, edge/node visits, depth, query cost |
| Shell | Fixed read-only `git`, `find`, `rg`, `sed`, `awk` pipelines | Exact returned paths/bytes and process costs |
| Excession | Indexed views, select/compose, virtual source reads | Path sets, manifest amplification, verification and runtime costs |

**Do not compare made-up natural-language answers.** Create immutable queries
whose outputs are well-defined (symbol location, exact-revision file retrieval,
transitive closure, changed-file identification, source identity checking).
Measure recall@k and precision@k *only* where a separate, version-pinned reference
set exists; an Excession-derived reference set is not independent ground truth.
For unsupported operations, record `unsupported`, not zero retrieval cost.
Run the same probes across repository sizes and graph densities, cold/warm
configurations and changed Git revisions. Report setup cost, per-query costs,
coverage and counterexamples rather than a single "winner" score.

## Interpretation

If graph-only returns the same files with less output and time, Excession's
extra view work needs a specific demonstrated benefit. A view may still provide
source-identity verification, but compare that separately to a small external
hash checker and account for its time and bytes. Lower bytes, fewer process
calls, lower CPU, more complete *independently verified* references, and better
failure detection are measurable; hypothetical agent effectiveness is not.