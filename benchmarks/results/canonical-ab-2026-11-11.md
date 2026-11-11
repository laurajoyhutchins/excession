# Canonical JSON and view-manifest allocation benchmark

**Verified measurement:** [GitHub Actions #43034113618](https://github.com/laurajoyhutchins/excession/actions/runs/43034113618), [raw A/B JSON artifact](https://github.com/laurajoyhutchins/excession/actions/runs/43034113618/artifacts/13861598396), tested commit `4a8435635135a89b64bbf249de2d44faec2e1aa1`.

## Change and contract

The old `Value::canonical()` recursively allocated individual serialized strings for nested array and object members and joined temporary vectors of those strings. The optimized implementation writes the same canonical bytes directly into a single growable `String` buffer. The final `Plan::manifest()` now moves the previously constructed object fields rather than deep-cloning the entire entries list.

**Nothing changes in the canonical schema, object key ordering, ASCII escaping, Unicode surrogate pair encoding, persisted view IDs, or source authentication.** The independent test oracle copies the previous serializer and compares exact bytes, including unusual Unicode and nested values.

## Controlled native A/B

Two native release binaries use **the same Rust 1.99.1 toolchain**: baseline [`2a7080f`](https://github.com/laurajoyhutchins/excession/commit/2a7080f3404b42a9f90357e91d1a40af1ead7e3e) and the optimized branch. Both are queried against a shared indexed h11 checkout and deterministic 32, 128, 512 and 1,024 module Python Git repositories. The pre-optimization binary indexes each repository once; source revisions and SQLite catalogs are shared.

Before timing, all **48 distinct query variants** were verified **byte-for-byte identical**, across full `view deps` and compact `view paths` responses. Both include matching canonical `view_id` hashes and Git source identity. The timed section contains **480 interleaved A/B pairs** plus **120 profiling pairs** that isolate the existing `canonical_view_id` step.

### Observed medians

The `canonical_view_id` measurement covers *constructing the canonical payload representation plus SHA-256*; it is not a SHA-256 primitive-only timing.

| Workload | ID before (µs) | ID after (µs) | ID speedup | Full-view before (ms) | Full-view after (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| h11 | 75 | 56 | 1.34× | 6.652 | 6.603 |
| 32 modules | 166.5 | 119 | 1.40× | 6.870 | 6.775 |
| 128 modules | 601.5 | 428.5 | 1.40× | 8.351 | 7.982 |
| 512 modules | 2,359 | 1,639 | 1.44× | 15.127 | 13.569 |
| 1,024 modules | 4,673.5 | 3,239.5 | 1.44× | 23.969 | 20.761 |

At 1,024 modules, the **15.5% reduction** in median full-view wall time is measurable on this runner. The compact role-query case improved **7.2%**, from 21.184 ms to 19.753 ms at that size. For h11 and 32-module queries, end-to-end changes are close to zero because process and Git/SQLite costs dominate.

For comparison, a separately passing earlier run [#43033985069](https://github.com/laurajoyhutchins/excession/actions/runs/43033985069) observed 2,711→1,966 µs for 1,024-module canonical IDs, and 14.178→12.697 ms for full views. Absolute times vary considerably across GitHub runners, so **within-run comparisons** and independent repetitions matter more than comparing a raw number between runs.

## Limitations

- This is an observed cost reduction, not a proof of an asymptotic time or memory bound.
- The benchmark does **not measure heap allocation counts or peak resident memory**. Avoid claiming a precise allocation or memory saving merely because the algorithm no longer materializes intermediate strings.
- The fixtures are one real small Git repository and synthetic Python module chains, not representative large source trees in arbitrary languages.
- Existing source Git/SQLite authentication, manifest/hash identity, refusal paths and schema are unchanged; this does not independently prove the completeness of semantic dependency discovery.
- CLI wall time includes process startup and Git checks. The profile instrumentation itself also introduces small overhead.

## Reproduction

The isolated `.github/workflows/canonical-ab.yml` builds both binaries from pinned commits and retains `canonical-ab.json` as an artifact. It is triggered only by relevant source or benchmark changes or by manual dispatch, rather than every ordinary commit. The script can also be invoked directly with already-built binaries:

```bash
python3 benchmarks/canonical_ab.py BASELINE_EXCESSION OPTIMIZED_EXCESSION canonical-ab.json
```

**Next performance question:** Is a direct canonical serialization of the six fixed projection fields and entry structure cheaper than first materializing the intermediate `Value::Object` tree? This must preserve field ordering, escaping, existing IDs and all manifest parse/refusal behavior.
