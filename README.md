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

## Language-neutral evidence contracts

Excession now compares **declared input identities** for Go, JavaScript,
TypeScript, Rust, and other languages using the same Git-backed Rust core.
It records source/target commit identities, selected blobs, roles, and declared
environment assumptions; cross-revision comparisons are independently audited
from SQLite. This is *not* dependency-closure completeness or test-result reuse.

```sh
excession evidence contract-plan --repo "$REPO" --rev "$COMMIT" \
  --declaration /tmp/inputs.json > /tmp/contract.json
excession evidence contract-compare --repo "$REPO" \
  --source-contract /tmp/source.json --target-contract /tmp/target.json \
  --db /tmp/evidence.sqlite
excession evidence contract-audit --repo "$REPO" \
  --source-contract /tmp/source.json --target-contract /tmp/target.json \
  --db /tmp/evidence.sqlite
```

See [language-neutral input contracts](docs/INPUT_CONTRACTS.md) for the schema,
limits, and real Go/JavaScript historical tests. The Python executable-slice import semantics remain in a narrow CPython AST
adapter; the **slice planner, Git source construction, execution, auditing,
receipts, and conservative historical carry now run in Rust**. The old
`tools/excession_slice.py` CLI is a compatibility wrapper. See
[Rust-owned executable slices](docs/EXECUTABLE_SLICES.md).

## Rust-owned executable slices

The native `excession slice plan|validate|materialize|source-audit|run|carry`
commands now replace the previous Python Git and execution implementation.
Only CPython `ast` is needed for Python import resolution; the Rust core owns
all canonical Git/receipt identities and read-only filesystem operations.
Execution remains explicitly trusted-local and is **not** a remotely attested
or behavior-preservation proof. See [the slice guide](docs/EXECUTABLE_SLICES.md).

## Historical dependency discovery adapters

The optional Git-native adapter `tools/excession_discover.py` proposes bounded
Go, JavaScript, TypeScript and Rust input declarations at any retained commit.
Use the same Rust `evidence contract-plan|contract-compare|contract-audit`
commands to authenticate and compare their Git identities. The adapter refuses
unsupported dependency resolution and does not certify effect completeness or
behavioral equivalence. See [dependency discovery](docs/DEPENDENCY_DISCOVERY.md)
for examples and precisely bounded coverage.

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

## Native dependency resolution (opt-in)

For a pinned historical Git revision, **Rust now owns native source
staging, path and source validation, toolchain identity, input declarations,
limits and snapshot auditing**. The `excession evidence discover-native`
command invokes narrow language-native adapters: `go list`, Node/TypeScript
resolution, or `rustc --emit dep-info`. The old
`tools/excession_native.py` entrypoint remains only as a thin compatibility
wrapper for tests and benchmarks.

```sh
excession evidence discover-native --repo "$REPO" --rev "$COMMIT" \
  --entry src/lib.rs --language rust --trusted-local true \
  > /tmp/native.json
```

Use the result's `declaration` field with `excession evidence contract-plan`.
This remains a local, explicitly trusted compiler invocation, **not an
untrusted-code sandbox**. The output remains `DECLARED_INPUTS_ONLY`, not
proof of behavioral preservation or permission to skip tests.
See [Rust native discovery authority](docs/NATIVE_RUST_AUTHORITY.md) and
[native dependency benchmarks](docs/NATIVE_DEPENDENCY_RESOLUTION.md).
