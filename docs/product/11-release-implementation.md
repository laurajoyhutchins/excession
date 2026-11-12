# Implemented release manifest validation

The release build now verifies exact Cargo version, Git checkout commit, optional tag-to-commit identity, each archive SHA-256 checksum, and its bundled TypeScript parser digest. It publishes a `release-manifest.json` only after the full release matrix succeeds. This is **not** a SLSA attestation or cryptographic publisher signature: the manifest says `NOT_ATTESTED`.

Existing archives are never overwritten on a rerun. A release's source/tag mismatch or altered archive is a release-blocking error.

Local test: `python -m unittest tests.test_release_manifest -v`. Installed end-to-end analysis remains tracked under #113 and must pass before any public release tag is pushed.
