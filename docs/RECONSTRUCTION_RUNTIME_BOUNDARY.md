# Reconstruction execution boundary

## Git-bound reason for this refactor

Excession's [November 14 self-analysis](https://github.com/laurajoyhutchins/excession/actions/runs/43642540565)
retained a decoded, Git-bound rust-analyzer SCIP index for `5293dc1dcb2f5eeb07876acfcc2d963eb1726d86`.
The index contains **33** references from
`core/src/evidence/reconstruction/automatic.rs` to its
`reconstruction.rs` parent: 7 scratch-path uses, 6 compiler-flag uses,
4 bounded file reads, 3 code-region extractions, 3 artifact identities,
and references to shared byte samples, subprocess execution, and constants.

The parent and child both need these facilities. Making the automatic
search import them from the parent creates unnecessary reverse dependency
on the module that owns the public API.

## Single owner, unchanged authority

`evidence/reconstruction/runtime.rs` now owns:

- Temporary directories and guaranteed cleanup through `Scratch`.
- A single bounded child-process lifecycle with explicit per-caller deadlines
  (15 seconds for the manual evaluator; 30 seconds for the search path).
- The existing fixed C compiler flags and object-code `.text` extraction.
- Byte-length and regular-file guards, artifact subject identities, and
  bounded binary comparison examples.

The original public `evaluate`, `history`, `search`, `searches`,
and `SearchOptions` remain in `evidence::reconstruction`. Receipt
schemas, source identities, timeouts, polling intervals, and the
`--trusted-local` gate remain unchanged. No code is executed by merely
building or inspecting the source.

The parent and automatic search both depend on the internal execution
primitive owner, not on each other's implementation or presentation
surfaces. The `runtime` module is private, not a new public API.

## Verification and limitations

- Rust unit tests cover stable artifact IDs, bounded mismatch examples,
  file-size and regular-file refusals, and scratch cleanup.
- Existing reconstruction integration and reproducibility tests must pass,
  including real compiler/binutils cases when available.
- Strict rustfmt, Clippy, and full repository test checks are required
  before merging.
- A fresh source-bound SCIP index should no longer resolve the 33 helper
  references to their old parent definitions. It may still include module
  declarations or other unrelated references.
- SCIP records observed compiler-symbol relationships, not full semantic
  equivalence; unchanged source for helpers plus tests is evidence of
  behavioral preservation, not a universal proof.

This scope is deliberately separate from PR #209's Git-history architecture
and PR #66's mechanical module-layout changes.
