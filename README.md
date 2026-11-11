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

The native CLI builds offline with Rust 1.99.1 and no third-party Cargo
crates. Git (SHA-1 object format) and SQLite remain required for repository
operations. Digests use portable Rust and no longer require OpenSSL.
Python 3.13.5 and Node 22.16.0 are optional language runtimes; Go 1.26.11,
Tree-sitter grammars and native scanners are separate opt-in adapters.

Prebuilt Linux, macOS and Windows archives package the binary and the
locked TypeScript parser sidecar. See [portable installation, supported
targets and limitations](docs/PORTABILITY.md).

```sh
cargo install --offline --locked --path core --bin excession
excession --help
# For development and the complete language-adapter test suite:
npm ci --ignore-scripts --no-audit --no-fund
python3 -m pip install -r requirements/research.txt
make check
```

`make check` runs Rust tests, formatting, strict Clippy, the JS adapter
checks, and Python integration tests. Use `make cli` for the development
binary and `make go-check` for the optional Go adapter. See
[development setup](docs/DEVELOPMENT.md) for toolchain files and separately
pinned research/grammar dependencies.

For JUnit XML reports, install `cargo-nextest` 0.9.151 and the pinned
Python research requirements,
then run `make junit`. This retains the existing `unittest` test cases while
producing separate Rust and Python reports:

- `core/target/nextest/ci/junit.xml` (Rust, including failure output)
- `test-results/python.xml` (Python, including skipped and failed tests)

CI uploads JUnit XML as artifacts, including after a test failure.
`make check` remains the simpler local validation path without nextest; Rust doctests
are also checked by `cargo test --doc` in the JUnit workflow.

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

## Self-analysis (dogfooding)

A dedicated [self-analysis workflow](docs/SELF_ANALYSIS.md) runs Excession on
its own clean, pinned Git source, audits syntax and conservative dependency
candidates, and independently verifies source identities using Git. It keeps
unresolved and unsupported observations visible. The resulting change-impact
hints are advisory and **never** authorize skipping tests.

## Multi-language syntax parsing

A separate optional Tree-sitter adapter reuses established grammars for many
languages. It emits Git-bound structural and import observations, with explicit
statuses when parsing is incomplete or a grammar is unavailable offline:

```sh
excession syntax scan --repo "$REPO" --roots src,tests > /tmp/syntax.json
excession syntax audit --repo "$REPO" --report /tmp/syntax.json
```

The optional parser package must be installed and grammars prefetched in advance;
no parser is downloaded during analysis. The existing Python/JS dependency index
remains a separate, language-aware path. See [syntax parsers](docs/SYNTAX_PARSERS.md)
for installation, limits and provenance.

## Early report validation

`excession syntax preflight --report /tmp/syntax.json` and
`excession syntax resolve-preflight --report /tmp/candidates.json` perform
cheap, parser-free report contract checks before costly regeneration. They
validate shapes, counts and canonical content digests but **cannot authenticate
Git sources**. Full `syntax audit` and `syntax resolve-audit` remain required.
See [syntax parser preflight](docs/SYNTAX_PARSERS.md#fast-preflight-before-expensive-source-replay).

## Candidate import resolution

Parsed imports can be mapped to scoped, Git-bound file candidates with
`excession syntax resolve --repo "$REPO" --report /tmp/syntax.json`.
A separate `syntax resolve-audit` command independently replays the mapping.
Go, Rust, JS/TS and Python use language-specific conservative path rules.
Ambiguous, external, configuration-dependent and unsupported cases remain
explicit. These are **navigation candidates, not compiler-established edges**.

## Refactoring candidates

`excession refactor suggest --db INDEX --name PROJECT --rev COMMIT` reports
deterministic, unverified import-cycle and repeated-body hypotheses from the
existing Python/JavaScript index. `excession refactor audit` replays the
indexed query against Git source identities and checks a saved report. It does
not reparse semantic relations, prove equivalence, or modify source.
See [refactoring hypotheses](docs/REFACTORING.md).

## Native declaration boundary

To compare native compiler-declared inputs with conservative syntax
candidates **before constructing repository views**, use
`excession syntax native-preflight --declaration native.json` followed by
`excession syntax native-compare --repo "$REPO" --syntax-report /tmp/syntax.json
--resolution-report /tmp/candidates.json --declaration native.json`.
The comparison reports in-scope candidate agreements, scope differences,
unresolved imports, and exact Git input identities. It never authenticates
a compiler process or asserts complete dependency closure. See
[language-specific boundary checks](docs/SYNTAX_PARSERS.md#compiler-declaration-boundary-before-view-assembly).

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

## Benchmarks

The [benchmark program](benchmarks/README.md) separates observable relational
experiments from [single-query microbenchmarks](benchmarks/micro/README.md).
Reproducible measurements do not claim improved agent productivity.

## Limits

The relational dependency index currently supports bounded Python and JavaScript static
module relations, with additional Go-native research adapters. A separate optional
Tree-sitter syntax scan covers more languages without claiming resolved dependencies. It cannot claim
that absent graph edges do not exist. Scanner source-to-sink flows are unverified
scanner reports until independent analysis establishes reachability and impact.
Excession does not automatically confirm CVEs, execute untrusted code safely,
or generate independently verified executable source reorganizations.

Third-party source fixtures retain their upstream licensing obligations; see
[third-party material](THIRD_PARTY.md).
