# Excession

**Git-grounded exploration, source relationships, and auditable security evidence.**

Excession derives focused, read-only repository views from pinned Git revisions
and indexed relationships. It also ingests CodeQL SARIF and Semgrep JSON as
**unverified security candidates** and independently audits their Git source
identities, SQLite records, and reported flows. A scanner finding is not a
confirmed vulnerability or a CVE.

The **only installed Rust executable** is `excession`. The library and Cargo
package are also named `excession`. The earlier research implementation is
accessible through the `excession research ...` command family, not through
an additional binary. See [migration notes](docs/MIGRATION.md), the
[architecture](docs/ARCHITECTURE.md), and [origins](docs/ORIGINS.md).

## Install and verify

Requirements: Rust 1.99.1, Git with SHA-1 repositories, native `libsqlite3` and
`libcrypto`, and Python 3 for the repository integration tests. The Rust crate
has no third-party Cargo dependencies; a network connection is not required.

```sh
cargo install --offline --locked --path core --bin excession
excession --help
make check
```

`make check` runs Rust tests, Rust formatting and strict Clippy, plus the Python
integration suite. Use `make cli` for a local development binary.

## One repository, multiple layouts

```sh
REPO=/path/to/clean/h11
OUT=/tmp/excession-h11
mkdir -p "$OUT"
excession index --repo "$REPO" --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --roots h11,tests
excession view deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._connection > "$OUT/deps.json"
excession view replay deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._connection --manifest "$OUT/deps.json"
excession browse --repo "$REPO" --manifest "$OUT/deps.json"
```

Other view operators include `compose`, `select`, `mount`, `ls`, `stat`, `cat`,
`materialize`, and `audit`. Views read pinned Git blobs on demand without
copying code. Materialized views are read-only **and not executable checkouts**.
See [CLI](docs/CLI.md), [repository views](docs/REPOSITORY_VIEWS.md), and
[composable views](docs/COMPOSABLE_VIEWS.md).

## Source-bound scanner evidence

```sh
excession security ingest --repo "$REPO" --report /tmp/report.sarif \
  --db /tmp/findings.sqlite --engine codeql
excession security audit --repo "$REPO" --report /tmp/report.sarif \
  --db /tmp/findings.sqlite --engine codeql
```

To invoke an already installed Semgrep or CodeQL scanner, use
`excession security scan ... --trusted-local true` with a reviewed, local
configuration. This is trusted-local execution, **not a sandbox**; database
creation and untrusted builds require their own isolation. The executable
neither downloads scanners nor asserts that a finding is exploitable.

## Executable slices and integration evidence

The Python adapter can derive a bounded static import closure from a clean,
pinned SHA-1 Git checkout and materialize only those exact source blobs while
retaining their original package layout. It can execute a trusted entrypoint
and conservatively compare historical evidence with a later integrated revision.
A source match is **not** proof that the merged program passed its tests.

On supported Linux machines, \`--isolated-linux\` runs the slice inside an
unprivileged, fail-closed namespace worker with read-only source, limited
resources, and no network namespace access. This does not authenticate a
remote worker, and the receipts remain unsigned.

\`\`\`sh
python3 tools/excession_slice.py plan "$REPO" tests/check.py > /tmp/slice.json
python3 tools/excession_slice.py materialize "$REPO" /tmp/slice.json /tmp/excession-slice
python3 tools/excession_slice.py run "$REPO" /tmp/slice.json /tmp/excession-slice \
  --trusted-local --isolated-linux > /tmp/slice-receipt.json
excession slice ingest --repo "$REPO" --manifest /tmp/slice.json \
  --receipt /tmp/slice-receipt.json --db /tmp/slice-evidence.sqlite
excession slice audit --repo "$REPO" --manifest /tmp/slice.json \
  --receipt /tmp/slice-receipt.json --db /tmp/slice-evidence.sqlite
\`\`\`

Rust independently checks the Git object identities, declared blob bytes,
manifest and receipt checksums, and SQLite rows. It does **not** independently
establish import-closure completeness or remotely attest to execution.
See [executable slices](docs/EXECUTABLE_SLICES.md).

## Research compatibility

Pinned earlier experiments retain their original Git blobs, schema strings,
identity-forming metadata, and fixture bundles. They can be replayed through
one binary:

```sh
excession research --help
excession research compose SPEC_JSON DATABASE
excession research compose-audit SPEC_JSON DATABASE
```

The `research` commands accept historical positional arguments. They are
separate from the modern named-option CLI and intentionally preserve existing
research receipts. See [migration notes](docs/MIGRATION.md) for the mapping.

## Limits

The source index currently supports bounded Python and JavaScript static
module relations, with additional Go-native research adapters. It cannot claim
that absent graph edges do not exist. Scanner source-to-sink flows are unverified
scanner reports until independent analysis establishes reachability and impact.
Excession does not automatically confirm CVEs, execute untrusted code safely,
or generate independently verified executable source reorganizations.

Third-party source fixtures retain their upstream licensing obligations; see
[third-party material](THIRD_PARTY.md).
