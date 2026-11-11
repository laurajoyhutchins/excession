# Excession

**Security analysis across code histories.** Excession binds static analysis, native counterexamples, and cross-revision relationships to independently auditable Git evidence. It builds on the RelGit Rust research core without pretending that scanner warnings are confirmed vulnerabilities.

**Current implementation:** import and independently audit CodeQL SARIF 2.1.0
and Semgrep JSON against exact source blobs, plus CodeQL path flow relations; execute trusted local scanners;
freeze an advisory-blind protocol and compare candidate sets across revisions.
**Not yet implemented:** an automatic exploitable-vulnerability detector,
custom CodeQL security-flow queries, or a verified-new-CVE claim.

See [the architectural specification](docs/ARCHITECTURE.md).

**Repository exploration:** generate dependency/security-flow views, compose them by original Git source identity, filter or namespace them, and browse their virtual files without a copied checkout. See [composable views](docs/COMPOSABLE_VIEWS.md) and [repository projections](docs/REPOSITORY_VIEWS.md).

## Build and test (offline, no external dependencies)

```bash
cargo test --offline --manifest-path core/Cargo.toml
cargo fmt --manifest-path core/Cargo.toml -- --check
cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s tests -v
```

Use Rust 1.99.1; the core requires `libsqlite3` and `libcrypto` installed on
the host. The source includes inherited RelGit test fixtures.

## Import an existing scanner result

```bash
python3 tools/pipeline.py import --engine codeql \
  --repo /path/to/clean/checkout \
  --report /tmp/codeql.sarif \
  --evidence /tmp/candidates.sqlite \
  --rust ./core/target/debug/relgit
```

The `semgrep` engine supports JSON scan output. The source checkout must be clean;
the database and report must be outside it.

To run the scanner first, replace `import` with `scan`, providing `--config
/path/to/local/rules` (Semgrep) or `--database /path/to/codeql-db --config
/path/to/query-suite` (CodeQL). These tools are **not** bundled or required for
offline tests. Native scan build and execution require isolation when running
untrusted repository code.

## Queryable repository layouts (read-only)

The same pinned Git revision can now be browsed through deterministic **dependency-focused** or **security-flow-focused** directory trees. Every virtual file maps to an original tracked source blob, including deliberate duplicates in multiple groups. Excession does not relocate source or promise that a materialized view is executable.

```bash
# First index the source, using the existing static dependency index:
relgit index /tmp/index.sqlite h11 /path/to/h11 HEAD h11,tests
relgit view-plan-deps /tmp/index.sqlite h11 HEAD h11._connection > /tmp/dep-view.json
relgit view-materialize /path/to/h11 /tmp/dep-view.json /tmp/dep-view
relgit view-plan-replay-deps /tmp/index.sqlite h11 HEAD h11._connection /tmp/dep-view.json
relgit view-audit /path/to/h11 /tmp/dep-view.json /tmp/dep-view
```

For security-focused views, first import and independently audit a CodeQL SARIF or Semgrep JSON report, then call `view-plan-security DB SOURCE_REPO RUN_ID`. Scanner flow labels remain **unverified scanner claims**, not confirmed trust boundaries. Views are copies of pinned Git blobs, permissioned read-only, with deterministic manifests and an independent audit. Details, refusal limits and all six CLI commands: [Repository views](docs/REPOSITORY_VIEWS.md).

## Blind historical protocol

```bash
python3 tools/protocol.py --repo /path/to/pinned/project \
  --family authorization-consistency \
  --family parser-differential --rules /path/to/local/semgrep.yaml \
  --out /tmp/protocol.json
```

The protocol includes no advisory or CVE labels and refuses overwrites.
Record candidate and native evidence before looking up advisories.

## Candidate comparison

```bash
python3 tools/compare.py /tmp/candidates.sqlite LEFT_RUN_ID RIGHT_RUN_ID
```

This report is a triage aid **only**. Candidate absence does not establish a fix.
Re-audit source and report against both run IDs before relying on a comparison.

## Historical RelGit work

The `core` directory contains the inherited Rust RelGit Git/SQLite
implementation and its bounded native examples, extended with
`security_ingest`. It is retained rather than replaced by a Python evidence
store. The security architecture is intentionally separate from the historical
experimental fixtures.

## Boundaries

This project does not authorize scanning third-party systems, execution of
untrusted code without isolation, or publishing unverified findings. A CVE is
an externally assigned identifier. Local observations alone do not create one.

## Naming and provenance

**Excession** is the project and repository name. `relgit` remains the internal Rust CLI name for compatibility with the imported, tested engine. It is not an independent security scanner. The original six-commit RelGit Security history is retained; see [Origins](docs/ORIGINS.md).

The GitHub repository lives at <https://github.com/laurajoyhutchins/excession>.
