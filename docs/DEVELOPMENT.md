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

Optional Go tests: `mise run go-check`. Optional syntax parsers:
`mise run syntax-setup`, then run the parser integration tests. The parser
package versions remain in `requirements/syntax.txt`. Prefetch the grammars
in a trusted setup step; Excession does not download them while scanning or
auditing a repository.

Mise is not required to build Excession. Developers can run the underlying
`make`, `cargo`, `npm`, and Python commands directly with compatible tools.
Git and the system SQLite/OpenSSL development/runtime libraries are still
system dependencies; this setup does not claim a hermetic OS image.

## Test reports

`make check` runs Rust format/Clippy/tests, Python integration checks, and
the JavaScript adapter tests. For JUnit XML, install `cargo-nextest` 0.9.151
and use `make junit` after `mise run setup`. Reports are written to
`core/target/nextest/ci/junit.xml` and `test-results/python.xml`. CI keeps
those artifacts on failure.

## Benchmark reports

`python3 benchmarks/reporting.py validate path/to/report.json` verifies the
Draft 2020-12 schema, report content digest, and referenced raw-artifact
hashes. The validator dependencies are pinned in `requirements/research.txt`.
`make benchmark-schema-check` runs report conversion and negative-case tests.
This verification is about benchmark evidence, not the installed environment.
