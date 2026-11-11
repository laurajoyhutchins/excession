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

## Linux source syntax and candidate relationships

Excession can inspect pinned Linux C preprocessor directives, Kbuild object
assignments and Kconfig declarations without building or executing the kernel.

```sh
excession kernel scan --repo /path/to/linux --rev HEAD \
  --roots drivers/rtc > /tmp/linux-rtc.json
```

The report binds source blobs and records conservative candidate links between
C includes, Make objects, matching source filenames and Kconfig symbols. It
does **not** evaluate a build configuration, perform complete C symbol
resolution, or prove that a dependency is compiled. See the
[Linux parser contract](docs/KERNEL_PARSERS.md) and
[reproducible kernel experiment](docs/LINUX_KERNEL_EXPERIMENT.md).

For a **trusted, configuration-selected** C/C++ translation unit,
Excession reuses a standard compilation database and Clang's AST/dependency
machinery in **any Git repository**:

```sh
excession analyze clang --repo /path/to/repository --source src/example.cpp \
  --compile-db /path/to/build/compile_commands.json --trusted-local true
```

This records translation-unit-specific declarations, calls and header hashes,
but does not prove build or execution. `excession kernel clang` remains a
compatibility alias. See [generic C/C++ compiler analysis](docs/COMPILER_ANALYSIS.md)
and [Linux compilation database details](docs/CLANG_TU.md).

To **assemble** these compiler observations with a real configured target,
Excession consumes CMake's File API codemodel and persists typed,
provenance-bearing graph edges in SQLite:

```sh
excession graph assemble --repo /path/to/repository --rev HEAD \
  --build /path/to/build --target fmt --observations /tmp/clang-receipts \
  --db /tmp/configured.sqlite > /tmp/configured-graph.json
```

This reports missing translation units explicitly, preserves configuration
differences, and does not claim complete dynamic or cross-TU call resolution.
See [configured graph assembly](docs/CONFIGURED_GRAPHS.md).

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
module relations, with additional Go-native research adapters and conservative
Linux C/Kbuild/Kconfig syntax observations. It cannot claim
that absent graph edges do not exist. Scanner source-to-sink flows are unverified
scanner reports until independent analysis establishes reachability and impact.
Excession does not automatically confirm CVEs, execute untrusted code safely,
or generate independently verified executable source reorganizations.

Third-party source fixtures retain their upstream licensing obligations; see
[third-party material](THIRD_PARTY.md).
