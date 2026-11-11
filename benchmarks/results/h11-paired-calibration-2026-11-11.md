# h11 calibration: graph-only versus dependency views

Measured on GitHub Actions run [42972816155](https://github.com/laurajoyhutchins/excession/actions/runs/42972816155), commit [34f3901](https://github.com/laurajoyhutchins/excession/commit/34f390147eb6205ce65b57476752b73ab90d21ac). Raw receipts: [excession-paired-calibration artifact](https://github.com/laurajoyhutchins/excession/actions/runs/42972816155/artifacts/13833649282).

**Scope:** 24 fixed queries over four h11 modules, three repetitions, for 72 paired comparisons. Both arms use the same indexed `module`/`dependency` data and exact Git revision. This is calibration, not independent proof of real-world dependency completeness or productivity.

| Measurement | Graph-only (Python/SQLite CLI) | Excession (Rust CLI) |
| --- | ---: | ---: |
| Source-set parity | 72/72 pairs | 72/72 pairs |
| Queries returning successfully | 72 | 72 |
| Refusals, timeouts or invalid outputs | 0 | 0 |
| Empty selected answers | 6 | 6 |
| Declared path matches (4 tasks x 3) | 12 | 12 |
| Median wall time | 51.172 ms | 13.040 ms |
| Observed p95 wall time | 53.131 ms | 13.905 ms |
| Median child CPU (user + system) | 51.020 ms | 13.192 ms |
| Median stdout | 65.5 bytes | 3012 bytes |
| Median emitted references | 2.5 | 12 |
| Median duplicate references | 0 | 1.5 |
| Median normalized selected-output bytes | 62 | 62 |
| Median selected candidate Git blob bytes | 20,115 | 20,115 |

Amortized wall time per query **including a shared one-time index**:

| Query count | Graph-only | Excession |
| --- | ---: | ---: |
| 1 | 173.430 ms | 134.997 ms |
| 5 | 75.946 ms | 37.513 ms |
| 20 | 57.668 ms | 19.235 ms |
| 100 | 52.793 ms | 14.360 ms |

## Interpretation

- Path outputs are identical for all paired calibration queries.
- Excession's median raw response is about **46x larger**, because it emits a complete multi-role view before the benchmark filters the requested role. Both normalized answers have the same median byte count.
- Excession's end-to-end CLI wall time is lower, but this is **not** an algorithm-speedup result: the graph arm pays Python interpreter startup, while Excession is a native Rust binary doing additional Git/index validation. A native graph-only baseline is needed to isolate view overhead.
- Candidate source bytes are potential files that could be read, not actual bytes consumed by an LLM.
- The 24 queries consist of six related graph operations at four seeds, and are not independent samples or engineering success assessments.
- Only four focus tasks have declared path expectations. Agreement on the remaining tasks must not be promoted to independent correctness.
- This run does not measure peak RSS, tokens consumed by an agent, ease of navigation, bug discovery, or improved task completion.

## Next falsification step

Add a **Rust graph-only query** sharing the same Git/index validation path, and compare it with `excession view deps` at fixed seeds. Add fixed `rg`, LSP and shell probes using outputs and resource measurements, never subjective agent scores. Investigate whether a filtered view output can avoid full-manifest amplification while retaining source authentication.
