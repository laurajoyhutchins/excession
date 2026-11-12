# Development environment

Excession uses [mise](https://mise.jdx.dev/) for developer toolchains and tasks.
It does not need an additional package-management or OS-attestation layer.

## Setup

Install mise once, then in the repository checkout run:

```sh
mise install
mise run setup
mise run check
mise run cli
```

`mise.toml` opts into the language-native version files so versions are not
declared twice:

| Runtime | Authoritative version file |
| --- | --- |
| Rust (including rustfmt and Clippy) | `rust-toolchain.toml` |
| Python | `.python-version` |
| Node.js | `.node-version` |
| Go, optional adapter | `.go-version` |

Mise creates a local `.venv` for Python dependencies, installs the ordinary
pinned `requirements/research.txt`, and runs `npm ci` with the existing
integrity-checked `package-lock.json`. Cargo keeps its `core/Cargo.lock`;
Rust build and test commands use `--offline --locked`.

Optional Go tests: `mise run go-check`. Install the default syntax grammars
with `mise run syntax-setup`, or every language covered by Excession's
recognized source extensions with `mise run syntax-setup-all`. These tasks
install upstream `ts-pack-cli` 1.25.0 and explicitly prefetch grammar libraries
using its Rust CLI. The Excession Rust binary links the same version of the
upstream Rust language-pack core with its download feature disabled. Scans and
audits never download grammars or require a Python interpreter.

## Other supported languages

These toolchains are optional **mise configuration environments**. They are
not installed by ordinary `mise install`, `mise run setup`, or CI's
default jobs. Choose only what a particular repository actually needs:

| Source language | Optional environment | Toolchain |
| --- | --- | --- |
| Java, Kotlin, Scala | `jvm` | Temurin JDK 25 |
| C# | `dotnet` | .NET SDK 10 |
| Ruby | `ruby` | Ruby 4.0 |
| PHP | `php` | PHP 8.4 via mise's registry |
| Elixir | `beam` | Erlang/OTP 29 + Elixir 1.20.4 compiled for OTP 29 |
| Swift | `swift` | Swift 6.4 (Linux/macOS only) |
| C/C++ | `native` | CMake 3.31; native system Clang/GCC |
| Bash/shell | none | Existing shell on the host |
| JSON, YAML, TOML, SQL | none | No language runtime needed for syntax |

For example:

```sh
mise -E jvm install
mise -E jvm exec -- javac -version
mise -E dotnet install
mise -E dotnet exec -- dotnet --info
mise -E beam install
mise -E beam exec -- elixir --version
```

`mise -E jvm,swift install` can combine profiles on a supported host.
Kotlin and Scala compilers and dependency resolution belong to each
inspected project's Gradle/Maven/sbt configuration, not to Excession.
C/C++ semantics depend on a concrete compiler and build configuration,
not merely the installed CMake version. Swift and native/plug-in backed
toolchains have platform-specific installation requirements.

**Support levels differ:** the optional Tree-sitter grammar provides syntax
observations only. JVM, .NET, Ruby, PHP, BEAM, and Swift toolchains are
conveniences for native analysis and are *not* Excession-verified dependency
resolvers. Only language-specific implementations with independent replay
may promote dependency relationships. Missing parsers or build contexts
remain explicit, not evidence of absent dependencies.

Mise is not required to build Excession. Developers can run the underlying
`make`, `cargo`, `npm`, and Python commands directly with compatible tools.
Git remains an external dependency. rusqlite compiles the bundled SQLite C
sources using a host C compiler, so building Excession requires a working
native C build environment, but running its release archives does not require
a separately installed SQLite or OpenSSL library. The dependency cache must
be populated once before `cargo --offline --locked`; this does not claim a
hermetic OS image. Run `excession doctor --json` to inspect local capabilities.

## Test reports

`make check` runs Rust format/Clippy/tests, Python integration checks, and
the JavaScript adapter tests. For JUnit XML, install `cargo-nextest` 0.9.151
and use `make junit` after `mise run setup`. Reports are written to
`core/target/nextest/ci/junit.xml` and `test-results/python.xml`. CI keeps
those artifacts on failure.

## Benchmark reports

`excession benchmark run-relational --out RESULTS --sizes 8,24 --fanouts 1,3 --revisions 3 --repetitions 2`
generates pinned Git fixtures, independently checks their indexed edges, runs alternating
paired CLI queries, and emits raw measurements and a validated Rust report.
The runner uses Unix child-process CPU accounting and refuses unsupported platforms.

`excession benchmark report --kind relational --directory RESULTS` builds a
schema-validated Rust report from existing relational receipts and source
bundles. `excession benchmark report --kind native --directory . --input
native-calibration.json` produces a separate native calibration envelope.

`excession benchmark validate --report path/to/report.json` verifies the
Draft 2020-12 schema, content digest, and referenced raw-artifact hashes.
The Rust `jsonschema` dependency is pinned in `core/Cargo.lock`.
`make benchmark-schema-check` runs report conversion and negative-case tests.
This verification checks benchmark evidence, not the installed environment.

## Test ownership

Cargo integration suites in `core/tests/` cover CLI, Git/SQLite, repository
views, security evidence, parser preflight, dependency resolution, refactor
hypotheses, SCIP, and native compiler/declaration comparisons. Python tests remain only for Python-specific adapters, source-language oracles,
and historical experiments not yet migrated. The relational runner, its
independent fixture oracle, and benchmark report validation are Rust-owned.
