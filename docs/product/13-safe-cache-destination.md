# Review and refactor: safe analysis cache destinations

## Defect

The first-run CLI checked cache locations with a lexical `Path::starts_with`. Paths containing `..`, an existing symlink parent, or an existing symlink database file could bypass that check and write SQLite into the source repository, violating the committed-source read-only contract.

## Change

`analyze::checked_cache_destination` validates absolute normalized paths, resolves the nearest existing ancestor **before creating directories**, checks the resolved parent against the canonical repository root, and refuses symlink database files. The indexer and SQLite adapter stay authoritative; this is a narrow input-boundary helper, not a new path abstraction or filesystem sandbox.

The CLI integration test uses a real Git repository and verifies refusal of traversal, symlink-directory, and symlink-file destinations without writing into the checkout. The existing positive fixture still uses the normal external nested cache path.

## Limits

This defends against pre-existing path aliases, not an adversarial concurrent filesystem race or hard-link attacks. Trusted-local native execution remains a separately documented boundary. No JSON report schema or evidence identity changes are made.
