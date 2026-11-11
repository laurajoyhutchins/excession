# Third-party material

Excession uses the maintained `rusqlite` bindings (MIT) and bundled SQLite
(public domain) as well as RustCrypto `sha1` and `sha2` (MIT OR Apache-2.0)
for existing object/evidence identities. The project also uses other pinned
Cargo dependencies recorded in `core/Cargo.lock`. Their licenses apply to
the redistributed software. The optional TypeScript sidecar retains its
upstream license in each release archive.



Excession incorporates historical source fixtures from the previous RelGit research project, which was
packaged with historical open-source test fixtures. Fixture bundles in
`core/fixtures/` contain code from third-party projects including h11,
Requests, Click, and PostCSS. Older native pilot materials reference
Kubernetes. These projects have their **own** upstream copyrights and
licenses; this repository's MIT license does not relicense their code.

The included source snapshots are for regression and provenance testing.
Before distributing or extracting them separately, consult their upstream
LICENSE and NOTICE files and preserve all required attributions. No claims of
upstream endorsement or universal redistribution clearance are made here.
