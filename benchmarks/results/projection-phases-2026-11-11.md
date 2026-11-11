# Authenticated dependency projection: phase-cost benchmark

**Verified run:** [GitHub Actions #42976804943](https://github.com/laurajoyhutchins/excession/actions/runs/42976804943), with the [raw phase receipts](https://github.com/laurajoyhutchins/excession/actions/runs/42976804943/artifacts/13835298251). Pinned Rust 1.99.1 release build on `ubuntu-24.04`, branch head `26d8182272560be8adeb56098431bbee843dca0e`.

The benchmark calls `excession view profile-deps`, which runs **the same dependency plan and source authentication path** as ordinary `view deps`. The profiler emits timings separately from the manifest; every query is checked against an independently generated full view ID and source commit/tree. The ordinary `view deps` manifest remains unchanged.

## Measured total and dominant phase

Values are **median wall time**, within the instrumented Rust command unless marked CLI. Microsecond measurements are converted to milliseconds below.

| Workload | Samples | Internal total | CLI wall | Git source scan | Index-tree reconstruction | Canonical view ID |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| h11 (12 indexed modules) | 48 | 9.711 ms | 11.601 ms | 5.272 ms | 1.003 ms | 0.084 ms |
| Synthetic 32-module chain | 24 | 9.845 ms | 11.729 ms | 5.297 ms | 1.035 ms | 0.164 ms |
| Synthetic 128-module chain | 24 | 11.276 ms | 13.197 ms | 5.427 ms | 1.193 ms | 0.626 ms |
| Synthetic 512-module chain | 24 | 16.715 ms | 18.773 ms | 5.824 ms | 1.852 ms | 2.354 ms |

The `read_git_source_tree` stage is the largest median stage in every workload. It includes repository verification and Git source enumeration; its roughly 5–6 ms cost here is **not** simply iteration over the number of files. Revision resolution (about 1.6 ms) and checking the Git commit/tree (about 1.4 ms) add largely constant overhead. The actual graph traversal and entry construction remains comparatively small: **0.016 ms on h11** and **0.488 ms at 512 modules**.

At 512 modules, canonical view ID generation (2.354 ms) and index-tree reconstruction (1.852 ms) grow in importance. Even with compact output, Excession still constructs and hashes the complete authenticated multi-role projection. Reducing stdout alone cannot eliminate that cost.

## How the scale control was constructed

The script creates three independent, deterministic Git repositories with a `bench` Python package and 32, 128 or 512 generated modules arranged as a linear import graph. The indexer must discover **N+1 module rows** (including `bench/__init__.py`) and **N-1 import edges**, or the benchmark refuses. It profiles both the first and final module. It also clones the retained h11 Git bundle and profiles four existing modules.

All 120 observations matched independently computed full view IDs, Git commit/tree, and expected entry counts. There were 48 h11 samples and 24 samples at each synthetic size. The script randomizes task order, retains per-sample phase durations and subprocess stdout digests, and reports median/observed nearest-rank p95. Index setup and on-disk bytes are recorded separately in the artifact; they are not silently amortized into the phase timings.

## Interpretation and next falsification

The evidence points to **repeated Git subprocess/source-tree verification** as the first target to investigate, not rewriting BFS. A worthwhile controlled follow-up would independently rederive source path/OID/mode bindings from the already verified SQLite tree and compare their identity and refusal behavior to the present `git::entries` scan. The exact Git tree and source-blob checks must still be independently justified; skipping them merely because the index says they passed would weaken the guarantee.

The measurements do **not** establish an algorithmic asymptotic bound, improved agent performance, unchanged correctness across all languages, or how much the source scan could safely be optimized. They come from one runner, one release binary and a small h11 project plus synthetic chains. `Instant` observations include profiler overhead and no memory or token measurements.

To reproduce, build the release CLI using Rust 1.99.1 and run:

```bash
python3 benchmarks/projection_phases.py core/target/release/excession projection-phases.json
```

The dedicated workflow retains `projection-phases.json` as a CI artifact, outside ordinary fast commit checks.
