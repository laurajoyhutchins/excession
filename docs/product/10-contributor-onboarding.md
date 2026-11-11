# PR 10: Contributor and user onboarding

**Status:** draft policy/documentation and pending executable examples. Installing `excession analyze .` from a public binary is **not yet working**; do not present it as shipped.

- Add root `CONTRIBUTING.md` describing quick vs deep tests, Rust/Git/SQLite authority, pinned fixtures, small PRs and independent oracles.
- Add `SECURITY.md` describing responsible nonpublic reporting without inventing a contact address or pretending scanner findings are verified vulnerabilities.
- Proposed getting-started path: release archive → doctor → analyze → impact → explain → audit. Until commands ship, a machine-readable example contract says `PROPOSED_NOT_SHIPPED`.
- Current users can still use the README's supported `index`, `inspect`, `graph`, and `view` interfaces.
- Later add docs-example smoke which must run against the **actual CLI**, not a textual mock; old `excession research` stays retired.

Auto-ontology #88–#95 is an optional analysis and concept context. Do not instruct new users to download or run it for basic module impact, nor imply its unmerged contracts are implemented.

**TDD:** schema fixture tests now; executable user quickstart test deliberately skipped until release behavior exists.

Tracks #86, #82, #83, #101.
