# Projection verification: reusable proof and measured cost

Compact role queries reduce stdout; they do not waive authentication of a
complete projection. The phase profiler is the existing native
`excession view profile-deps` command, with multi-size calibration in
`benchmarks/projection_phases.py`.

## Why this reuse is sound

For an **indexed dependency projection**:

1. Resolve the exact revision and read the Git commit's root tree OID.
2. Reconstruct the **entire** indexed tree, including path, mode, type,
   blob and subtree OIDs, and require its root hash to equal the root tree
   actually referenced by that Git commit.
3. Join every indexed module to a `tree_entry` for the same repository,
   commit and path. Require a regular blob and explicitly compare its
   separately stored module blob OID to the authenticated tree-entry OID.
4. Build and validate the full deterministic virtual projection, including
   roles, path order and collision checks. Compare every projected binding
   to the indexed module bindings before returning the manifest or a
   compact role selection.

After steps 1 and 2, another full `git ls-tree` would reconstruct the *same*
Git path/mode/object bindings that the index has just rederived and checked
cryptographically. Reusing this proof removes a redundant Git invocation,
**not** the full-tree reconstruction or per-module source check. A mutated
module blob is not authenticated merely because its path is present.

For an **external manifest** without an authenticated indexed-tree proof,
`view ls`, `view stat`, `view cat`, `view audit`, and `view materialize`
continue checking entries against Git. `view compose` can enumerate one
immutable Git commit/tree once after checking both manifests reference the
same source revision, but checks **both** entry sets independently.
`view security` similarly reuses the Git source table already fetched and
validated earlier in that invocation.

No proof is cached between CLI invocations, and no receipt or identifier
from a mutable SQLite file is independently trusted. A view ID commits to
the canonical view description; it is not a cryptographic signature.
Git's SHA-1 OIDs retain their usual collision assumptions. This does not
certify that the semantic dependency index is complete, that software
runs correctly, or that a vulnerability is present/absent.

## Reproduce the performance comparison

```sh
cargo +1.99.1 build --offline --release --manifest-path core/Cargo.toml
python3 benchmarks/projection_phases.py \
  core/target/release/excession projection-phases.json
python3 benchmarks/native_paired.py \
  core/target/release/excession native-calibration.json
```

The native phase benchmark covers pinned h11 and synthetic 32/128/512-module
graphs. It runs the *same dependency planner* used in production, while
adding stage timestamps without modifying the view schema or ID.
The `read_git_source_tree` stage on the old baseline becomes
`reuse_indexed_tree` in the optimized path. `reconstruct_index_tree`
and `validate_source_bindings` remain present. Stage names must not be
silently equated across versions.

The native paired benchmark compares direct Rust graph retrieval against
compact authenticated view retrieval, checking source-set parity and
full-view IDs. On a pull request, its CI workflow also builds the unmodified
base commit and the candidate on the **same runner**, retaining raw receipts
for both. Report paired medians and observed nearest-rank p95; they are
finite-fixture observations, not population bounds. Process startup and
serialization also contribute to end-to-end latency.

Earlier measurements identified the Git source scan as a dominant phase:
approximately 5.3 ms for pinned h11 and 5.8 ms for a synthetic 512-module
chain. See `benchmarks/results/projection-phases-2026-11-11.md`. These are
**pre-optimization observations** and should not be labeled speedups.

## Regression boundaries

- A forged module OID is rejected even if the entire indexed tree is valid.
- A changed tree-entry OID or mode is rejected by root reconstruction.
- A forged full-view ID cannot substitute for independent Git source binding.
- A second input to composition is verified despite reuse of one Git scan.
- A compact response retains the source set and full-view ID of its manifest.
- Independent external-manifest audit and direct blob checks remain available.

Cross-request proof caching would require a stronger transactionally pinned
index snapshot or independently verifiable proof artifact. It is deliberately
out of scope.
