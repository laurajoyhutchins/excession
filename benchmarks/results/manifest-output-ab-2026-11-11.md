# Full dependency manifest serialization: controlled native A/B

**First independently successful run:** [GitHub Actions #43038374603](https://github.com/laurajoyhutchins/excession/actions/runs/43038374603), with [full JSON receipt](https://github.com/laurajoyhutchins/excession/actions/runs/43038374603/artifacts/13864017442). Initial candidate source: `cd5d8e10ba3caabae99e0ee3dd60da784ce4e7c1`. Pinned pre-change implementation: `670c3e4b80d8cc887719c4a020cd254489a2b83d`.

## Question

Does full `view deps` still pay unnecessarily for constructing `Value::Object`, a per-entry `BTreeMap` and the full canonical JSON traversal **after** source authentication and canonical view hashing have already been optimized?

## Implementation and preserved guarantees

`Plan::canonical_manifest()` reuses the **existing byte-exact** `Plan::canonical_payload()`, computes the same SHA-256 over the six-field payload, then appends `view_id` as the last lexicographically sorted field. This directly produces the same complete manifest as `Plan::manifest().canonical()` without allocating the intermediate nested JSON Value structure.

Only ordinary, non-replay `excession view deps` uses the new output path. Security, composition, selection, replay, and manifest validation retain their existing code paths. The type-based `Plan` validation, Git Merkle tree and SQLite source checks, source-path/blob/mode bindings, and materialization refusals are not modified. The CLI writes the same final newline.

The Rust test compares every byte of the new manifest to the independent legacy `Value` serializer and exercises `Plan::from_bytes` at **1, 2, 17, 128, 1,024, and 5,000 entries**, including Unicode, control characters, both executable and regular file modes, and multiple view roles. It verifies the exact original `view_id` and round-trip identity.

## Native A/B measurements

Both native release binaries were built with pinned Rust 1.99.1 on one `ubuntu-24.04` runner. The benchmark creates each Git/SQLite index only once using the prior implementation. The binaries query identical commits, trees, and catalog relations, with randomized arm ordering and 10 repetitions per variant.

All **48 unique query variants** were required to produce **byte-for-byte identical JSON** (including full manifest fields and existing view IDs) before timing. The harness then measured **480 A/B pairs** and **120 paired canonical-ID profile observations**.

The table reports measured **median CLI wall time** in milliseconds, including process startup and Git/SQLite checks.

| Workload | Full before | Full after | Change | Compact paths before | Compact paths after |
| --- | ---: | ---: | ---: | ---: | ---: |
| h11 | 6.535 ms | 6.458 ms | 1.2% faster | 6.497 ms | 6.515 ms |
| Synthetic 32 modules | 6.607 ms | 6.517 ms | 1.4% faster | 6.558 ms | 6.532 ms |
| Synthetic 128 modules | 7.848 ms | 7.565 ms | 3.6% faster | 7.630 ms | 7.722 ms |
| Synthetic 512 modules | 13.253 ms | 11.771 ms | 11.2% faster | 11.479 ms | 11.486 ms |
| Synthetic 1,024 modules | 20.359 ms | 17.425 ms | 14.4% faster | 17.346 ms | 17.431 ms |

The compact output control is effectively unchanged. Canonical-ID calculation is also essentially stable, at **998.5 µs before vs 961.5 µs after** for the 1,024-module workload; this optimization targets manifest output construction rather than ID hashing. Per-sample wall/child CPU measurements, observed nearest-rank p95, binary SHA256 identities, input revisions and indexed fixture counts are retained in the raw artifact.

## Limits and interpretation

- This is a **single-query full dependency manifest optimization**, not a new serialization format or a change to all view commands.
- The A/B compares historical repository commits, not single-edit binaries; intervening changes may influence absolute timing.
- The source graph controls are deterministic Python chains and one real small h11 fixture, not a representative distribution of large repositories.
- Heap allocation counts and peak memory were not measured. The code removes nested JSON allocation work, but the benchmark does **not** quantify memory savings.
- The unchanged compact-query timings and rising benefit with view size support the interpretation that avoided output construction is the cause, but this is not a mathematical performance proof.
- Exact byte identity plus the existing Git/SQL refusal tests establish the intended compatibility for these fixtures; they are not a proof of arbitrary-language semantic completeness.

## Reproduce

The dedicated `.github/workflows/manifest-output-ab.yml` builds both binaries and invokes:

```bash
python3 benchmarks/manifest_output_ab.py BASELINE_EXCESSION CANDIDATE_EXCESSION manifest-output-ab.json
```

It preserves the raw A/B receipt independently of ordinary short commit checks. The prior Rust `Plan::manifest()` serializer remains the test oracle and replay/complex-view implementation.
