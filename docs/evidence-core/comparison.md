# Coverage-qualified source-byte comparison

`core/src/evidence/comparison.rs` compares exact source bytes from two
Git revisions, including across different repositories when each source
reference is independently authenticated with Git. It reads the original
blobs under a strict per-source byte budget and emits a deterministic
versioned result. The
[`comparison.v1.schema.json`](../../contracts/evidence-core/comparison.v1.schema.json)
format is validated against the **actual Rust output** in the real Git
fixture test.

Both producer-owned coverage reports must bind to the corresponding
source commit and the same evidence facet. Different selection or
configuration is explicitly marked noncomparable.

## What the result proves

- Whether the bounded, independently read source file contents are
  byte-for-byte identical or different.
- Each exact Git source commit, tree, path, mode, blob ID and SHA-256
  of the read bytes.
- Which source-scoped producer coverage reports accompanied the comparison.

## What it does not prove

- That matching source bytes behave the same under different build
  configurations, dependencies, generated inputs or runtime environments.
- That a changed source breaks a caller.
- That no unobserved semantic relationship exists.
- That a source file corresponds to a binary artifact or that two
  independently produced artifacts are behaviorally equivalent.

Artifact correspondence, predicate-specific evidence checking, native
compiled-build identity and cross-repository full scopes remain separate,
unfinished requirements of #122 and #123. No second Git/SQLite authority or
generic dependency edge is introduced.
