# Predicate-scoped native observation audit

`excession inspect verify-observation` examines an *existing* immutable SCIP
observation, not an arbitrary proposed graph edge. Required arguments are
`--db`, `--repo`, `--rev`, `--snapshot`, `--observation`, `--source`,
and `--predicate SCIP_SYMBOL_REFERENCE`.

The read-only Rust operation recomputes the original evidence snapshot and
receipt integrity, checks the original native kind and source subject identity,
and independently binds both endpoints and the recorded line to real Git blobs.

The emitted v1 JSON result is described by
[`predicate-audit.v1.schema.json`](../../contracts/evidence-core/predicate-audit.v1.schema.json).
Real CLI integration tests validate the output, forged relationships, invalid
source lines and unsupported predicate changes.

The result establishes **Git source identity and original producer byte
integrity only**. It does not independently rerun the SCIP producer or prove
that a symbol reference is a call, import, build input or vulnerability. The
result retains `UNATTESTED` producer execution, `NOT_ESTABLISHED`
semantic correctness and explicit unresolved verification obligations.

Additional native predicates require separate supported input/semantic
verification, not relabeling or dispatching everything as a generic edge.
