# Native compiler impact and independent baselines

This experiment is designed to find where native observations **actually
improve impact analysis**, not to assign a winner to incomparable compiler
predicates.

The source repositories are three independently maintained upstream projects
at fixed, reviewed Git commits: [JSON-java](https://github.com/stleary/JSON-java),
[golang-lru](https://github.com/hashicorp/golang-lru), and
[zlib](https://github.com/madler/zlib). Pins and selected files are in
[native-impact-cases.json](native-impact-cases.json). No repository owned by
the Excession developer appears in the benchmark.

## Execution

```sh
python3 -m unittest discover -s tests -p 'test_native_impact_baselines.py' -v
cargo +1.99.1 build --locked --manifest-path core/Cargo.toml --bin excession
python3 benchmarks/experiments/native_impact_baselines.py \
  --manifest benchmarks/acceptance/native-impact-cases.json \
  --bin core/target/debug/excession \
  --out-dir /tmp/native-impact-results
```

Requires Git and network access for the exact pinned repositories, Go 1.23+,
JDK 25, Clang and Python 3.11+. The Git, toolchain and compiler invocations
are bounded. No Maven/Gradle, project builds or annotation processors run.
`--trusted-local` invokes the native toolchains without a sandbox, so
review and pin the source repositories before execution.

## Three different oracles, not a synthetic universal dependency score

| Case | Native observation | Independently invoked oracle | Naive baseline |
| --- | --- | --- | --- |
| JSON-java | JDK AST-resolved source-to-source references to `JSONObject.java` | Independently compiled Java bytecode, then `javap -v` class-pool references to `JSONObject` | Other Java sources containing the token `JSONObject` |
| golang-lru | Go compiler-selected source files | Direct `go list -json` on pinned checkout | All root `.go` files except `_test.go` |
| zlib | Configured tracked Clang header observations | Independent `clang -MM` source dependencies | Literal direct `#include` matches in source |

These have different denominators. Java bytecode class references are
**not** the entire set of valid Java source-level references. Differences
between AST and classfile observations must be inspected, not automatically
called false positives. Go input membership is **not** a Go call graph.
Configured Clang headers are a translation-unit dependency, not a
whole-program call graph.

The harness records independently sourced precision, recall, missing paths,
extra paths, source identity, native receipt and snapshot, report sizes,
process wall times, and measured impact candidate sets. It deliberately
injects a comment-only commit in a temporary checkout to ask whether
native observations reach Excession's source-candidate traversal, without
claiming any actual behavioral breakage.

## Costs and interpretation

The benchmark measures separate cold and repeat native invocations, one
independent oracle invocation, immutable evidence import/audit and
`excession impact`. **Repeat time is not incremental maintenance**.
There is no true incremental native caching yet. The response explicitly
records `cached_incremental_native_execution: NOT_IMPLEMENTED`.

Native receipts can legitimately be exposed as configured observations
without entering the candidate graph. The harness records both
`configured_observations` and `candidate_source_impacts` and does not
treat the former as though they established the latter. No graph edge
may be invented from Go source membership or a raw Clang call count.

Any rejected source, unavailable tool, changed Git pin, unsuccessful
source audit or failed language oracle is a **failure**, not an empty
successful result. The summary preserves failures and partial case
artifacts for diagnosis. See the
[native impact workflow](../../.github/workflows/native-impact-baselines.yml)
for the reproducible runner environment.
