# Direct canonical projection ID: native before/after benchmark

**Exact-head run:** [GitHub Actions #43035888865](https://github.com/laurajoyhutchins/excession/actions/runs/43035888865) with [raw JSON receipts](https://github.com/laurajoyhutchins/excession/actions/runs/43035888865/artifacts/13862658221). Candidate source commit: `ee7beda5ccf57071b1d50134e6cefc5e4567fb82`. Previous canonical serialization commit: `90cb4d5c1cbe7fde704a6d9189aa03cb5eff7a66`.

## Change and acceptance contract

The old `Plan::payload().sha256()` creates all six manifest payload fields and every nested entry as `json::Value` instances, canonicalizes the tree, then hashes the resulting byte string. The new `Plan::view_id()` writes the same canonical payload into one string directly from the existing typed `Plan` fields and hashes it. It uses the **same string escaping function** as the independent JSON serializer, but specifies the historical sorted field order directly.

The unchanged `Plan::payload()` remains the independent test oracle and continues to construct the public manifest; the optimized ID path is used in manifests, dependency queries, composition, workspace receipts and parsed-manifest validation. The Merkle/SQLite source checks, source metadata, manifest field names, and refusal policies are unchanged.

The Rust differential test checks direct canonical bytes and SHA-256 against the independent `Value` tree serializer for 1, 2, 17, 128 and 1,024 entries, with Unicode, ASCII control characters, escaping, alternate modes, and multiple roles. This enforces **historical canonical byte compatibility**, not just semantically similar JSON.

## Native A/B method

Two Rust 1.99.1 release binaries were compiled on the same GitHub runner. They read the **same Git repositories and SQLite catalogs**, created once by the baseline implementation: h11 and deterministic 32, 128, 512, 1,024 module Python chains.

All **48 unique** full/compact query variants were required to produce **byte-identical** JSON outputs before timing, including exact source commit/tree, complete entry fields and canonical `view_id`. The harness ran 480 randomized interleaved timed query pairs and 120 paired profile observations. Any identity discrepancy refused the benchmark.

### Measured medians

The canonical ID phase is measured in microseconds and includes construction of the canonical payload bytes **and** SHA-256. These are observed medians on one CI host, not a complexity guarantee.

| Corpus | ID before | ID after | ID speedup | Full-view before | Full-view after | Compact paths before | Compact paths after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| h11 | 49 µs | 19 µs | 2.58× | 5.255 ms | 5.277 ms | 5.274 ms | 5.251 ms |
| Synthetic 32 | 108.5 µs | 35.5 µs | 3.06× | 5.405 ms | 5.438 ms | 5.345 ms | 5.320 ms |
| Synthetic 128 | 363.5 µs | 121 µs | 3.00× | 6.468 ms | 6.542 ms | 6.390 ms | 6.187 ms |
| Synthetic 512 | 1,388.5 µs | 482 µs | 2.88× | 11.169 ms | 11.105 ms | 10.724 ms | 9.784 ms |
| Synthetic 1,024 | 2,894.5 µs | 1,071.5 µs | 2.70× | 18.014 ms | 17.937 ms | 16.806 ms | 14.920 ms |

At 1,024 modules, compact path queries improved by **11.2%** in observed median wall time, while full `view deps` was essentially unchanged. This distinction matters: full manifests still allocate and serialize a complete `Value` tree for output. Avoid treating faster ID computation as the same thing as faster full-manifest materialization.

## What this does not prove

- No memory allocation counts, peak resident memory, real-world cross-language scaling, or asymptotic bound was measured.
- Profiling includes the hash operation and canonical byte string creation; SHA-256 primitive-only performance is not isolated.
- The fixture is a small real Git repository plus deterministic Python chains. This does not establish semantic dependency completeness.
- The shared field order is enforced by differential tests, but the repository does not provide a formal proof of serializer equivalence for arbitrary input strings.
- The current full-view code still materializes a `Value` object, by design, to produce stable output. Reducing or streaming that would be a separate change needing independent manifest and refusal tests.

## Reproduce

The isolated `.github/workflows/direct-payload-ab.yml` builds both binaries from pinned commits, uses the shared benchmark harness, and retains `direct-payload-ab.json` outside normal quick commit checks.

```bash
python3 benchmarks/direct_payload_ab.py BEFORE_RELEASE_BIN AFTER_RELEASE_BIN direct-payload-ab.json
```

The benchmark reuse wrapper pins the correct baseline commit and imports the shared experiments/micro generators after the benchmark directory reorganization.
