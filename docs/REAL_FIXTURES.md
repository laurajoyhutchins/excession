# Additional pinned projection fixtures

The real-repository check complements the existing bundled h11 and generated
32/128/512-module chains. Its fixture specification lives in
`benchmarks/fixtures/real-repositories.json`; external source repositories
are fetched at **literal commit SHAs**. No third-party source is vendored.

The suite exercises bundled h11, Pallets ItsDangerous, Pallets Click, Requests,
and Axios. It measures index cost and compact-query latency separately. Every
seed's full view is compared to the same Git commit's independent blob/mode
enumeration and a separately canonicalized view ID. It then verifies all six
compact role selections against the full manifest, four selections against
direct graph retrieval, and the full profile's source identity. Finally, it
requires refusal after corrupting a module blob binding **or an unrelated
tree entry**, demonstrating that projecting a small source set still
authenticates the complete indexed tree.

Run on a host with Git, Python, Node, globally installed TypeScript 5.9.3,
and a built Excession binary:

```sh
python3 benchmarks/real_fixtures.py \
  --bin core/target/release/excession \
  --manifest benchmarks/fixtures/real-repositories.json \
  --out real-fixture-results.json
```

The dedicated GitHub Actions workflow runs on pull requests changing this
suite or its Rust indexing/projection code and can be dispatched manually.
It preserves a machine-readable artifact even if one fixture fails. Repository
availability is an external CI dependency, but moving upstream refs cannot
silently change the pinned input.

**Interpretation limits:** The Rust graph arm and view arm both use the same
semantic SQLite dependency index. Agreement proves query/output parity, **not**
whether the parsers found every true dependency. Checking source blob OIDs
and Merkle-root reconstruction does not authenticate the derivation of
semantic dependency edges. Report the upstream commit, root tree, row counts,
rejection cases, timings, and any failures. Do not mix these per-fixture
latencies into a general speedup claim or treat them as measured agent quality.
