# PR 2: Zero-configuration repository discovery

Status: **contract, initial documentation and TDD only**. No automatic discovery or cache implementation yet.

## Intended behavior
- Discover Git worktree root from any subdirectory, resolve `HEAD` to an exact SHA-1 commit/tree, and record the actual source root selection. Preserve a user-specified revision and roots.
- Reuse `excession doctor --json` and existing language/native indexer boundaries; never execute a target project's build just to discover adapters.
- Identify output cache outside the source checkout. Normalize path safety and avoid claiming that absolute local paths are portable source identities.
- Missing parsers, unsupported object formats, ambiguous roots, and configured build gaps remain explicit; do not infer completeness from a successful walk.
- Discover **source**, not ontology: concept subjects and claims belong to the independent semantic stack #88–#95. This scan must not classify dependencies through concept name matches.

## Implementation seam
Extend the existing clap CLI, Git root/rev utilities, and `doctor.rs`. The new `discovery.v1` report is an operational preflight, not another semantic graph.

## TDD
Active: valid/invalid schema and unexpected adapter states. Pending: fixture invocation from subdirectories, Unicode paths, missing Python/Node, dirty worktrees and no modifications to the checkout.

**Refs:** #79, #82, #88, #89, #101.
