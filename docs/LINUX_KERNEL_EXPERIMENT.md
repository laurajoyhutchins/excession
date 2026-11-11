# Linux kernel source experiment (v6.12)

This is a reproducible integration experiment, not a claim of Linux C
dependency analysis or kernel vulnerability detection.

## Source and method

- Upstream: `torvalds/linux`, annotated tag `v6.12`.
- Exact commit: `adc218676eef25575469234709c2d87185ca223a`.
- Git tree: `ac4266ccaf1cf79e8fb22ad3e0d86deac358ffb9`.
- Sparse worktree: `scripts/gdb`; the index still covers the entire
  pinned Git tree.
- Semantic scope: `scripts/gdb` Python helpers, with explicit import base
  `scripts/gdb`.
- Focus: `scripts/gdb/linux/tasks.py`, imported as `linux.tasks`.

Run `tests/linux_kernel_probe.py` with the CLI built from this repository,
or inspect `.github/workflows/linux-kernel-probe.yml`. The script records
timings and indexed row counts in `metrics.json`, saves a deterministic view
manifest, replays that view, and compares virtual source bytes against Git.

## Observations

The first pre-fix run indexed 92,437 Git entries and 26 Python modules in
13.188 seconds. A dependency view incorrectly contained only the focus file:
Python imports named `linux.utils` and `linux.lists` were absent because
the Git-derived module names were prefixed with `scripts.gdb`.
The run did correctly replay the manifest and reproduce Git bytes.
Evidence: [first run](https://github.com/laurajoyhutchins/excession/actions/runs/42970977904).

With the explicit `--import-base scripts/gdb`, the same source revision
produced 106 dependency records and an 11-entry view. Both independently
AST-observed imports, `linux.utils` and `linux.lists`, were present.
Git-tree reconstruction, manifest replay and raw-blob comparison passed.
The measured index and view times were 10.172 and 3.919 seconds.
Evidence: [corrected run](https://github.com/laurajoyhutchins/excession/actions/runs/42971386424).

These are individual GitHub Actions executions, not controlled performance
comparisons. Baseline `git grep` and `git show` timings are now also
recorded, but they perform much less integrity and graph work. Benchmark
claims require repeated paired runs with equivalent retrieval tasks.

## Boundaries and next questions

- The full tree had 86,680 files, including 34,665 C files. No C include,
  symbol, Kbuild, or Kconfig semantics were extracted.
- The Git path/module mapping must be explicitly supplied; Excession does
  not infer Python import paths or dynamic imports.
- One indexed dependency graph does not establish completeness, security,
  behavior preservation, or permission to skip tests.
- Measuring Linux C navigation will require a compiler-database-aware C
  adapter and configuration-scoped dependency evidence, compared against
  ordinary `git grep`, language-server navigation, and dependency traversal.
- Replaying a focused view takes several seconds on this kernel-sized Git
  tree. Measure the cost of repeated full-tree authentication and
  amortization of the initial index before claiming retrieval speedups.
