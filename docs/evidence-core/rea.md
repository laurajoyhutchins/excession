# REA observations are core artifact evidence

REA (morluto/rea) exports Evidence v2 bundles containing native observations,
authorities, artifact subjects, raw/normalized results, and unresolved
questions. Excession ingests those as a first-class artifact evidence source,
without renaming native predicates to calls, proof, or security findings.

A REA observation is useful even if source code is unavailable. It is not
required to invent a Git commit or a build provenance record.

### Artifact-only

    excession evidence rea-import --db evidence.sqlite \
      --artifact /path/executable --bundle /path/rea-bundle.json

    excession inspect rea-evidence --db evidence.sqlite \
      --artifact /path/executable --id BUNDLE_ID

    excession inspect rea-record --db evidence.sqlite \
      --artifact /path/executable --id BUNDLE_ID --record ev_...

### Linked to a build observation

    excession evidence rea-import --db evidence.sqlite \
      --artifact /path/executable --bundle /path/rea-bundle.json \
      --repo . --rev COMMIT --build-snapshot BUILD_SNAPSHOT

The optional source **link** is a separately audited build receipt. REA
remains a core capability, including for third-party binaries with no source.

Imported records share the same Artifact subject identity as build provenance.
This permits joining byte-authenticated build inputs and REA-native
artifact observations without claiming that the declared recipe produced the
binary. The original REA bundle bytes, native ev_ identifiers, provider,
predicate, confidence, authority, locations, limitations, and unresolved
records are retained in the one SQLite database. No REA tool is run by import.

The Rust parser recomputes REA semantic identities using the supported
RFC 8785 safe-integer subset (UTF-16 key order and JSON string encoding).
Unsupported floating-point/unsafe numeric records refuse, rather than
pretending a different canonicalization is equivalent. The importer also
checks the artifact manifest lists the selected content hash and preserves
all foreign raw records. It has not yet independently validated the complete
REA manifest projection or residual-unknown revision graph, so those claims
remain NOT_ESTABLISHED, even if the original REA tool would accept them.

Raw evidence cannot independently establish semantic correctness, source
correspondence, process execution or the absence of unobserved edges. All
untrusted execution remains out of scope for this import path.

The versioned report schema is
[rea-import.v1.schema.json](../../contracts/evidence-core/rea-import.v1.schema.json);
real Rust CLI tests validate actual report output, adversarial native IDs,
artifact digests, original raw bytes and source-linked readback.
