# PR 2: Semantic observation normalization

Status: contract + TDD scaffold only. No new parser, SCIP decoder, or Rust adapter.

## Input and authority

Accept pinned Git source, an explicit collection of upstream receipts (SCIP, syntax, configured compiler, runtime, scanner), and producer-specific coverage. The Rust adapter must authenticate every source path/span/blob against Git, preserve original upstream symbol strings and relationship kinds, and never silently collapse producer categories.

The existing `excession.semantic-observations.v1` means *static import candidates only* and remains unchanged. Official `scip print --json` remains the SCIP decoder. Multiple indexes require individual producer/configuration receipts; matching names alone do not establish a cross-index relationship.

## TDD scenarios

- Valid explicit partial static observation batch is accepted structurally.
- Missing source binding, producer kind, or coverage is rejected.
- A SCIP reference must not be normalized as a compiler input or observed runtime call.
- Duplicate observation identities with conflicting contents, untracked paths, stale blobs, invalid offsets, and incomplete indexer coverage are refused or reported unresolved, not guessed.
- Canonical output and audit replay are byte-identical.

## Implementation acceptance

Build a Rust normalization boundary feeding the v1 model; use existing Git validation and external SCIP decoding. Test against real h11 and rust-analyzer SCIP fixtures. This PR intentionally does not provide that implementation.
