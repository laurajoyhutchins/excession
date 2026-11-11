# Excession

**Git-grounded exploration, source relationships, and auditable security evidence.**

Excession derives focused, read-only repository views from pinned Git revisions
and indexed relationships. It also ingests CodeQL SARIF and Semgrep JSON as
**unverified security candidates** and independently audits their Git source
identities, SQLite records, and reported flows. A scanner finding is not a
confirmed vulnerability or a CVE.

The **only installed Rust executable** is `excession`. The library and Cargo
package are also named `excession`. The positional `research` command is
retired; historical protocols remain in Git history, not in the supported CLI.
See [migration notes](docs/MIGRATION.md), the
[architecture](docs/ARCHITECTURE.md), and [origins](docs/ORIGINS.md).

## Install and verify

The Rust CLI uses pinned Cargo dependencies, including clap for its single
modern command grammar. After installing the locked dependencies, it builds
offline with Rust 1.99.1. Git (SHA-1 object format) and native SQLite are
required for repository operations; digests currently use portable Rust.
Python 3.13.5 and Node 22.16.0 are optional language runtimes, and Go,
Tree-sitter grammars and security scanners are opt-in adapters.

Prebuilt Linux, macOS and Windows archives carry the binary and the
locked TypeScript parser sidecar. See [portable installation and supported
targets](docs/PORTABILITY.md) for runtime dependencies and limitations.

For full development checks, the pinned mise workflow remains available:

```sh
mise install
mise run setup
mise run check
mise run cli
mise exec -- core/target/debug/excession --help
```

`make check` runs Rust tests, formatting, strict Clippy, the JS adapter
checks, and Python-specific adapter and independent-oracle tests. Git, SQLite,
projection, security, syntax, and refactoring integration tests run under Cargo.
Use `make cli` for the development
binary and `make go-check` for the optional Go adapter. See
[development setup](docs/DEVELOPMENT.md) for mise tasks, native toolchain
version files, and optional Tree-sitter grammars. Mise is a convenience layer;
Cargo, npm, and pip remain usable independently.

For JUnit XML reports, install `cargo-nextest` 0.9.151 and the pinned
Python research requirements,
then run `make junit`. Rust owns application integration contracts;
Python retains its adapters and independent reference checks. Reports:

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

A dedicated [self-analysis workflow](docs/SELF_ANALYSIS.md) uses
**rust-analyzer SCIP** for resolved Rust symbols and references, while
Tree-sitter remains a separate syntax observation path. The SCIP consumer
checks indexed source against pinned Git, and its results are advisory:
they **never** authorize skipping tests.

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
Go, JS/TS and Python use language-specific conservative path rules.
Rust intentionally has no handwritten import resolver: use rust-analyzer SCIP.
Ambiguous, external, configuration-dependent and unsupported cases remain
explicit. These are navigation candidates, not compiler-established edges.

## Refactoring candidates

`excession refactor suggest --db INDEX --name PROJECT --rev COMMIT` reports
deterministic, unverified import-cycle and repeated-body hypotheses from the
existing Python/JavaScript index. `excession refactor audit` replays the
indexed query against Git source identities and checks a saved report. It does
not reparse semantic relations, prove equivalence, or modify source.
An optional decoded SCIP input enables language-native reference hypotheses
without a custom Protobuf decoder or Rust language parser. For nested Rust
projects, use `--source-prefix core` to bind index paths to repository-root Git.
See [refactoring hypotheses](docs/REFACTORING.md).

## Integrated compiler-backed discovery

For supported Go, Rust and JS/TS projects, run the trusted-local language
resolver and source-bound comparison together:

```sh
excession syntax native-run --repo "$REPO" \
  --syntax-report /tmp/syntax.json \
  --resolution-report /tmp/candidates.json \
  --entry src/main.rs --language rust --trusted-local true
```

The compiler runs only **after** cheap report and Git-revision checks.
Output preserves the actual native input declaration, runtime statistics and
a Git-backed boundary report with unresolved and divergent scopes. This is
trusted-local parsing, not sandboxing or independently attested behavior.

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

## Historical experiments

Pinned earlier experiments retain their Git objects, schema strings,
identity-forming metadata, and fixture bundles. Their positional command
interface is **not supported** by the current executable. Reproduce an old
protocol using its pinned historical Git revision and environment, rather
than adding a second command grammar to Excession. Supported source-bound
view and audit workflows are documented in the [CLI](docs/CLI.md).

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

## Versioned source evidence contracts

\`excession semantic scan\` emits JSON Schema Draft 2020-12 language-neutral
syntax candidates and unresolved imports, with replay against Git.
\`excession inspect evidence\` retains a separately typed, read-only query
contract that distinguishes static candidates, compiler-declared inputs and
runtime witnesses. Both datasets use ODCS 3.2.0 metadata and fail closed on
schema drift without claiming complete dependency discovery. See the
[semantic contract](docs/SEMANTIC_CONTRACTS.md) and
[evidence query contract](docs/EVIDENCE_QUERY.md).
