# Excession

**Security analysis across code histories.** Excession binds static analysis, native counterexamples, and cross-revision relationships to independently auditable Git evidence. It builds on the RelGit Rust research core without pretending that scanner warnings are confirmed vulnerabilities.

**Current implementation:** import and independently audit CodeQL SARIF 2.1.0
and Semgrep JSON against exact source blobs, plus CodeQL path flow relations; execute trusted local scanners;
freeze an advisory-blind protocol and compare candidate sets across revisions.
**Not yet implemented:** an automatic exploitable-vulnerability detector,
custom CodeQL security-flow queries, or a verified-new-CVE claim.

See [the architectural specification](docs/ARCHITECTURE.md).

**Repository exploration:** generate dependency/security-flow views, compose them by original Git source identity, filter or namespace them, and browse their virtual files without a copied checkout. See [composable views](docs/COMPOSABLE_VIEWS.md) and [repository projections](docs/REPOSITORY_VIEWS.md).

## Dedicated CLI

Install the first-party `excession` executable, not the historical `relgit` binary:

```bash
cargo install --offline --locked --path core --bin excession
excession --help
```

The command groups are `excession index`, `excession view`, `excession browse`,
`excession security`, and `excession inspect`. For example:

```bash
excession index --repo /path/to/h11 --db /tmp/h11.sqlite --name h11 --roots h11,tests
excession view deps --db /tmp/h11.sqlite --name h11 --rev HEAD \
  --seed h11._connection > /tmp/h11-view.json
excession browse --repo /path/to/h11 --manifest /tmp/h11-view.json
```

The CLI preserves exact Git blob provenance and never interprets a virtual
layout as executable code. See the [complete CLI reference](docs/CLI.md),
including composition, scanner ingestion, independent audit, and the read-only
browser. `relgit` remains available only for historical compatibility.

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
  --rust ./core/target/debug/excession
```

For trusted local scanner execution, `excession security scan` can launch an
installed Semgrep/CodeQL executable with a reviewed ruleset and bounded runtime,
then independently audit its report. This **is not isolated execution**:

```bash
excession security scan --repo /path/to/clean/checkout \
  --engine semgrep --config /path/to/rules.yaml --trusted-local true \
  --report /tmp/semgrep.json --db /tmp/candidates.sqlite
```

The CLI is the preferred interface; the Python runner remains as a compatible
orchestration adapter. See [CLI reference](docs/CLI.md).

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
excession index --repo /path/to/h11 --db /tmp/index.sqlite --name h11 --roots h11,tests
excession view deps --db /tmp/index.sqlite --name h11 --rev HEAD --seed h11._connection > /tmp/dep-view.json
excession view materialize --repo /path/to/h11 --manifest /tmp/dep-view.json --output /tmp/dep-view
excession view replay deps --db /tmp/index.sqlite --name h11 --rev HEAD --seed h11._connection --manifest /tmp/dep-view.json
excession view audit --repo /path/to/h11 --manifest /tmp/dep-view.json --output /tmp/dep-view
```

For security-focused views, first import and independently audit a CodeQL SARIF or Semgrep JSON report, then call `view-plan-security DB SOURCE_REPO RUN_ID`. Scanner flow labels remain **unverified scanner claims**, not confirmed trust boundaries. Views are copies of pinned Git blobs, permissioned read-only, with deterministic manifests and an independent audit. Details, refusal limits and all six CLI commands: [Repository views](docs/REPOSITORY_VIEWS.md).

## Executable Python slices and evidence carry (trusted-local prototype)

A bounded `tools/excession_slice.py` CLI can derive an exact Git-backed static
Python import closure for one test entrypoint, materialize its source files
without moving package paths, audit the reconstructed files, and execute the
test only with explicit `--trusted-local` approval. After independent branches
are merged, `carry` compares the historical and integrated closures and can
optionally rerun the target slice to produce **fresh**, separate evidence.

A matching slice reports `SOURCE_MATCH_ONLY`, never proof of behavioral
preservation. Changes report `REVERIFY_REQUIRED`. The optional
`--isolated-linux` worker uses fail-closed local namespaces and a read-only
source mount. Its receipts remain unsigned. Rust binds those receipts to exact
Git blobs in SQLite and independently audits them via:

```bash
excession slice ingest --repo /src/repo --manifest /tmp/base-slice.json \
  --receipt /tmp/base-receipt.json --db /tmp/slice.sqlite
excession slice audit --repo /src/repo --manifest /tmp/base-slice.json \
  --receipt /tmp/base-receipt.json --db /tmp/slice.sqlite
```

This is not proof of cross-branch behavioral equivalence or trusted remote
execution. See [Executable slices and evidence carry](docs/EXECUTABLE_SLICES.md).

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

**Excession** is the project, repository, and dedicated CLI name. `relgit` remains an additional Rust binary for compatibility with inherited experiments. It is not an independent security scanner. The original six-commit RelGit Security history is retained; see [Origins](docs/ORIGINS.md).

The GitHub repository lives at <https://github.com/laurajoyhutchins/excession>.
