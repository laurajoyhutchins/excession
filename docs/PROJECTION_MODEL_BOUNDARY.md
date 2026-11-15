# Projection: one source-bound model

The dependency and security query builders construct views. Composition combines
views. The workspace reads, materializes, and audits them. None of those
operations should own the definition of a valid view.

## Evidence for the boundary

The pinned [Excession self-analysis on main](https://github.com/laurajoyhutchins/excession/actions/runs/43659106884)
(replay artifact [14139686340](https://github.com/laurajoyhutchins/excession/actions/runs/43659106884/artifacts/14139686340))
contains an independent rust-analyzer SCIP index. Counting unique-definition
non-declaration cross-file references from the two projection child modules
to `src/projection.rs` gives:

| Consumer | Parent references, before |
|---|---:|
| `composition.rs` | 54 |
| `workspace.rs` | 58 |
| **Total** | **112** |

Most references are the `Plan` and `Entry` fields and their canonical
methods. Some are source-table authentication and file read helpers. The
old `use super::*` imports hid the ownership problem.

## Ownership

`projection/model.rs` is the single owner of:

- `Entry` and `Plan`, with the established public re-exports retained at
  `excession::projection::{Entry, Plan}`.
- Exact canonical payload/manifest encoding and SHA-256 view identity.
- Full manifest parse, sort, path, mode, size, collision, and kind validation.
- Source-table enumeration and per-entry source/blob/mode authentication.
- Checked manifest loading and deterministic plan construction.

`projection.rs` retains indexed dependency and security-query construction.
`composition.rs` depends on the model for source identity and validation,
and directly on `workspace::replay` for replay.
`workspace.rs` alone owns materialization filename and byte caps.

The source of truth is still the pinned Git tree. A manifest is not an
executable checkout, and scanner flow relationships remain unverified.
No new public API or alternate serializer was introduced.

## Acceptance

The Plan/Entry model, including validation and canonical encoding, was
moved without altering its function bodies. The prior optional
`authenticate_profiled` adapter was internal and only called with
`None`; authentication now delegates directly to the same validation,
source-table, and source-binding checks. The public dependency profiler
continues to use its own unchanged phase instrumentation.

The existing adversarial `projection_contracts.rs` suite remains the
independent contract oracle for sorting, path traversal, symlinks, forged
identities, source mutation, empty composition, revision mismatch,
replay, materialization, and file/directory collisions.

Merge requires format, Clippy, Rust and integration tests, portable CLI,
and an exact-head Excession self-analysis. SCIP counts are observations,
not formal proof of semantics or authorization to skip tests.
