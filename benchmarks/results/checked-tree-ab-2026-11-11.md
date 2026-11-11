# Checked Git-tree reuse: controlled native before/after benchmark

**Verified stronger-refusal run:** [GitHub Actions #43032568607](https://github.com/laurajoyhutchins/excession/actions/runs/43032568607), [full JSON receipt](https://github.com/laurajoyhutchins/excession/actions/runs/43032568607/artifacts/13862665447), commit `c20ca2ca29a1c3b5548d3429d76de03b4b7ca01f`.

The benchmark restores the **pre-reuse** implementation at `fa1d15248210ee6ec2a4c744603b271d613fa7b5` into a separate Git worktree. It builds both pre-reuse and optimized binaries using pinned Rust 1.99.1, release mode, on the **same GitHub Actions runner**. All query pairs read the exact same Git checkout and SQLite index, produced by the baseline CLI.

## Observed results

Each query variant was warmed and checked for byte-identical output before timing. For each, 12 repeated A/B pairs ran in randomized order with alternating arms. All **50 distinct query variants** produced exactly the same canonical response in both binaries, including source commit/tree and full authenticated view ID. This represents **600 timed A/B pairs (1,200 timed CLI calls)**.

The numbers below are observed **medians of CLI wall time**, not a proven performance bound.

| Corpus | Query | Before (ms) | After (ms) | Observed speedup |
| --- | --- | ---: | ---: | ---: |
| h11, 12 indexed modules | Full `view deps` | 9.482 | 5.477 | 1.731× |
| h11 | Compact `view paths` | 9.451 | 5.417 | 1.745× |
| Synthetic 32 Python modules | Full `view deps` | 9.649 | 5.562 | 1.735× |
| Synthetic 32 Python modules | Compact `view paths` | 9.562 | 5.546 | 1.724× |
| Synthetic 128 Python modules | Full `view deps` | 11.345 | 6.921 | 1.639× |
| Synthetic 128 Python modules | Compact `view paths` | 10.903 | 6.703 | 1.627× |
| Synthetic 512 Python modules | Full `view deps` | 18.598 | 13.188 | 1.410× |
| Synthetic 512 Python modules | Compact `view paths` | 16.213 | 11.622 | 1.395× |

Observed nearest-rank p95 values and child CPU timings are present per arm and workload in the JSON artifact. The synthetic fixtures contain 33/129/513 indexed modules (including `bench/__init__.py`) and 31/127/511 import edges. The h11 fixture contains 12 indexed modules and 31 edges.

## Source-authentication regression checks

The independent index is copied before mutation. Both pre-reuse and optimized binaries **refused** both full and compact queries in each of these four altered cases:

1. The `module.blob_oid` claim for `h11._connection` was changed to another **existing** indexed blob OID (the `_headers` module), not an invalid all-zero OID.
2. The `tree_entry.oid` for `h11/_connection.py` was changed to the existing `_headers.py` OID. The indexed Git Merkle tree can no longer reconstruct the commit's root.
3. The `revision.tree_oid` was replaced with an existing subtree OID rather than the actual root.
4. A dependency target was changed to a missing module. This last case tests referential integrity, **not** cryptographic authenticity of the semantic dependency graph.

These checks exercise representative corruptions, not every conceivable attack. A *plausible but false* dependency relation between two existing modules is **not** authenticated merely by reconstructing the Git tree. Source identities and semantic relationships are different guarantees; the benchmark does not claim otherwise.

For indexed dependency views, reusing the already reconstructed Git Merkle tree eliminates a redundant source enumeration while preserving the source blob/mode/path binding checks. Externally supplied view manifests continue to require their independent Git source checks. The A/B byte comparison and adversarial refusals confirm these properties for the tested fixtures, not a universal equivalence theorem.

## Interpretation and limits

The measured wall-time improvement ranges from approximately **1.40× to 1.75×**. The largest gain is on small queries, where the formerly repeated Git verification and subprocess work occupied a bigger fraction of the total. At 512 modules, complete canonical view construction and hashing become comparatively more important.

The control is a historical commit, not a synthetic one-diff build. The binaries differ by other intervening changes, including parser work; therefore, the entire speedup cannot be attributed uniquely to checked-tree reuse. Interleaving and the shared index remove major host/startup/corpus confounders, but do not establish causal attribution. This measures a fixed h11 Git fixture and deterministic Python chains on one runner, not agent productivity, real-world repository distributions or an asymptotic complexity bound.

To reproduce, run the isolated workflow `checked-tree-ab.yml`, which builds both binaries and produces `checked-tree-ab.json`. It does not burden ordinary short commit checks.

## Next performance question

At larger view sizes, measure the allocation and canonicalization overhead of the full view identity separately, without reducing the strength of the Git tree proof or changing existing manifest hashes.
