# PR 11: Public release provenance and publication gate

**Status:** specification, schema and tests only; **no release is published** by this PR.

Reuse the existing `portable-cli.yml`, `package_release.py`, checksum generation, and third-party notices. A release manifest binds exact Git SHA-1 source commit, semver package version, platform target, archive filename and content SHA-256. Attestation claims require an actual attestation reference. A checksum is not publisher-authenticated provenance.

After product PRs merge, run full supported OS artifact matrix and offline extracted-archive E2E (#83/#84). Publish only verified archives from the exact tag. Provide release notes, install/update/uninstall documentation, and use established GitHub provenance/attestation rather than another signature format.

Do not duplicate ODCS data contracts or the semantic-intelligence model (#88–#95). Semantic concept/ontology bundles are optional, independently versioned research artifacts, not hidden release prerequisites.

**TDD:** active checks for malformed metadata, missing source, false attestation claim. Actual release publication and attestation verification deliberately unimplemented.

Tracks #83, #84, #95, #101.
