# Excession benchmark program

The old tests answered, "How quickly can Excession look up a file or construct
a dependency view?" That is a **microbenchmark**, not the main research
question.

The reorganized question is: **Can Excession return exact, independently
validated source relationships at lower *total* resource cost across multiple
queries and Git revisions, while preserving a stated verification guarantee?**

## Structure

| Area | Status | Purpose |
| --- | --- | --- |
| [Observable protocol](protocol.md) | Defined | Equal information contracts, independent oracle and four cost axes |
| [Relational experiments](experiments/README.md) | Executable | Source-written oracle, multi-revision retrieval, size/fanout/revision variation |
| [Checked-tree A/B](experiments/checked_tree_ab.py) | Existing | Reusable source-binding checks with adversarial refusal parity |
| [Real repository fixtures](experiments/real_fixtures.py) | Existing | Pinned Git objects and independent bounded import-edge oracle; dynamic imports unsupported |
| [Single-query microbenchmarks](micro/README.md) | Existing, preserved | Git, ripgrep, clangd, graph-only and view phase diagnostics |
| [Historical reports](results/) | Retained | Original measured results and GitHub Actions artifact pointers |

**No agent task-completion or subjective productivity metric is used.**

## First executable experiment

After compiling `core/target/debug/excession`:

```sh
python3 benchmarks/experiments/relationships.py \
  --bin core/target/debug/excession \
  --out /tmp/excession-relational-v1 \
  --sizes 8,24 --fanouts 1,3 --revisions 3 --repetitions 2
```

Outputs include `summary.json`, `trials.jsonl`, and one reproducible Git
bundle per workload. Each committed revision is indexed into the *same*
per-repository SQLite database; the independent source specification is checked
against all parsed edges. An edge mismatch fails the experiment and writes a
coverage diagnostic, instead of silently reporting an apparent success.

The two timed arms issue the same four query kinds (focus, direct dependencies,
direct dependents and reachable indexed tests) via one native CLI call each,
with alternating paired order. Indexing cost is recorded separately and belongs
in any end-to-end comparison. The compact view authenticates a source-bound
projection; the simple graph query does not authenticate an equivalent view.
The difference must not be presented as a comparison of equal guarantees.

Existing `micro/` scripts remain separately runnable and have their original
results under `results/`. They are not inputs to a global "Excession wins" score.

See [experiment details](experiments/README.md) and
[protocol and limitations](protocol.md).
