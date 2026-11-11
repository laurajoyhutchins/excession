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
must not appear in claims. One graph arm runs as a fresh Python interpreter, while the native Rust graph
and Excession views share the same Rust binary and indexed tree checks. The
view additionally authenticates source references; this remains a CLI and
verification comparison, not a pure layout-algorithm microbenchmark. Cache
state is not rigorously controlled.

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
**that exact database** for all three arms: Python/SQLite BFS, Rust/SQLite BFS
through `excession inspect graph`, and `excession view deps` with role filtering.
They share tasks, commit and index, with rotated execution order. Query results
are tested for source-set parity without claiming graph completeness.

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

## Implemented simpler-tool probes

The native graph comparison uses `benchmarks/paired.py`. For four fixed module
file-location queries, `benchmarks/locate.py` compares `rg --files`, `git ls-tree`,
native Rust graph and views. The Git arm enumerates tracked `h11` paths and
filters afterward, preserving the raw output amplification in the receipt.
The `rg` arm requires a clean working tree at the exact queried HEAD.

```sh
python3 benchmarks/locate.py --repo /path/to/clean/h11 \
  --db /tmp/excession-calibration-001/index.sqlite \
  --bin ./core/target/debug/excession \
  --tasks benchmarks/calibration-h11.json --name h11 \
  --out /tmp/excession-file-lookup-001
```

A separate real JSON-RPC `textDocument/definition` probe uses the tiny tracked
C fixture. `benchmarks/symbol_compare.py` compares returned definition
locations to exact `rg` and `git grep` occurrences and records LSP initialization
latency, definition-request latency and wire bytes. An unavailable language
server is `unsupported`, not a zero-cost success. Excession currently lacks
C-symbol navigation; it is unsupported on that probe.

```sh
python3 benchmarks/symbol_compare.py --repo . --out /tmp/excession-symbol-001
```

These probes test different query operations. Do **not** aggregate exact-file
lookup, transitive dependency closure and semantic symbol resolution into one
score. The C fixture is a protocol smoke, not representative LSP evidence.

## Further fixed, observable retrieval probes

Expand to independent source revisions, languages, and indexed queries:

| Arm | What is executed | What is measured |
| --- | --- | --- |
| Keyword | Fixed `rg` queries and bounded file reads | Exact hits, rank, bytes emitted, CPU/wall time, exit status |
| LSP | Pinned server: workspace symbols, definitions, references | Exact symbol locations, count, bytes, startup/index cost, latency, unsupported cases |
| Graph | Identical indexed edges with Python and Rust BFS | Path sets, query time, CPU and output bytes |
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