# Artifact-centered evidence

`excession inspect artifact-evidence --db evidence.sqlite --artifact
/path/to/binary` joins native REA and build receipts by the **typed
SHA-256 and byte-count Artifact subject**, not filename or symbol spelling.

A dossier is read-only, bounded to at most 16 REA bundles per query, and
includes the total matching stored bundle count and a truncation indicator.
For a provenance-linked REA bundle, Excession requires explicit replay of
its original build snapshot; without that, the entry says
`BUILD_REPLAY_REQUIRED` and does not publish unaudited native record IDs.

To independently replay the claimed build inputs alongside REA evidence:

    excession inspect artifact-evidence --db evidence.sqlite \
      --artifact /path/to/binary --repo . --rev COMMIT \
      --build-snapshot SNAPSHOT_SHA256

The source-linked view rereads the artifact, verifies the pinned Git input
identities and reconstructed build receipt, and audits original REA bytes
and supported native Evidence IDs. It does not infer what compiler created
the artifact, that imported REA semantics are correct, or that unobserved
relationships are absent. Causal source-to-binary attribution needs a
separate independently attested isolated build.

The report contract is
[artifact-evidence.v1.schema.json](../../contracts/evidence-core/artifact-evidence.v1.schema.json).
Real Rust/Node/Git CLI fixture tests validate a dossier containing both
source-free and source-linked REA observations, the same artifact subject
identity as the build receipt, bounded results, and fail-closed input.
