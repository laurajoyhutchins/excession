# Origins and stable research identities

Excession is the successor to the **RelGit Security** and earlier **RelGit**
Rust research experiments. That history explains some remaining identifier
strings in archived fixtures and versioned research protocols. The old name is
**not** the name of an installed package, library, or executable.

The GitHub repository began with an independent MIT-license commit and then
imported and curated the source/provenance work. See the Git history, including
`archive/pre-history-refactor-2026-11-10`, for the original imported ancestry.
The original local `relgit-security-0.1.0.bundle` is a historical artifact,
not an external build dependency.

`core/fixtures/` includes pinned third-party source snapshots and Git bundles
from h11, Requests, Click and PostCSS, with some older Kubernetes pilots.
The tests reproduce exact Git tree and commit identities, finite observations,
and existing SQLite/report protocols. They do **not** establish universal
semantic equivalence or independently discovered vulnerabilities.

## Why some strings are intentionally not renamed

Historical Git author names/emails, generated compatibility shim source,
commit message bytes, and protocol identifiers such as
`relgit.security.ingest.receipt.v1`, `relgit-scoped-v1`, and
`RELGIT_OBSERVATION:` may be inputs to byte-for-byte reproduction or existing
stored evidence. Renaming them without a versioned migration would invalidate
known receipts. They remain **historical wire/provenance identifiers**, not a
second project or CLI. New code should use `excession` naming and versioned
`excession.*` interfaces.

The active Rust crate/package and sole binary are named **`excession`**.
Research operations with historical positional arguments are accessed via
`excession research ...`; see [the migration guide](MIGRATION.md).
