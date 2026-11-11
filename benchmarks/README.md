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
| [Real repository fixtures](experiments/real_fixtures.py) | Existing | Pinned Git objects and independent bounded import-edge oracle; dynamic imports unsupported |
| [Single-query microbenchmarks](micro/README.md) | Existing, preserved | Git, ripgrep, clangd, graph-only and view phase diagnostics |

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

Diagnostic `micro/` scripts remain separately runnable. They are not
inputs to a global "Excession wins" score.

See [experiment details](experiments/README.md) and
[protocol and limitations](protocol.md).

## Machine-readable reports

Relational runs now produce a Draft 2020-12 JSON Schema validated `report.json`
alongside the **unchanged** `summary.json`, `trials.jsonl`, and Git bundles.
See [report schema](schemas/benchmark-report.v1.schema.json). The envelope records
source revisions, executable digest, independently assessed checks, unit-bearing
metrics, and SHA-256 digests of referenced artifacts.

```sh
make benchmark-schema-check
core/target/debug/excession benchmark validate --report /tmp/excession-relational-v1/report.json
core/target/release/excession benchmark report --kind native --directory . --input native-calibration.json
core/target/debug/excession benchmark validate --report native-calibration.report.json
```

The Rust CLI refuses missing or tampered artifacts, unavailable measurements
encoded as zeros, duplicate metrics, unsafe paths, and false success labels.
Report IDs are digests, **not signatures**. A native calibration explicitly
declares a shared-index oracle, while relational experiments declare
independently authored source relationships. Store raw measurements with the run rather than committing stale benchmark receipts.

## Full indexed-query audit: independently implemented

`experiments/independent_audit.py` independently checks a dependency
projection's complete deterministic query against Git's actual tree objects,
the SQLite index, module source bindings, and the canonical view ID. It compares
against `excession view replay deps` on intact inputs and eight
manifests/database corruption variants, recording refusal parity and execution
costs. This does not prove semantic dependency truth: the independent Python
and JS source-import oracle in `experiments/dependency_oracle.py`
establishes that *separately*, within its declared static scope.
