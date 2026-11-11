# Excession Rust core

Git is authoritative for source objects; SQLite stores indexed relations and
scoped evidence. This crate's package and library are both named `excession`.
The **one** binary is `excession`. Its maintained `clap` grammar defines
all commands and flags, including views, syntax, security, refactoring,
benchmarks, and independent audits. The engine does not claim to infer
arbitrary language semantics or confirm exploitable vulnerabilities.

## Optional multi-language parsing

`excession syntax scan` uses preinstalled Tree-sitter grammar-pack parsers to
produce Git-authenticated structural and import observations; `syntax audit`
independently re-parses and compares those observations. No external source
code is executed and no unverified syntax fact becomes a dependency or proof.
See [syntax parser documentation](../docs/SYNTAX_PARSERS.md).

## Run the checks

From the repository root:

```sh
cargo fetch --locked --manifest-path core/Cargo.toml  # initial setup
cargo test --offline --locked --manifest-path core/Cargo.toml
cargo fmt --manifest-path core/Cargo.toml -- --check
cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s tests -v
```

To collect CI-compatible JUnit XML, run `make junit` from the repository root
with `cargo-nextest` 0.9.151 and `pytest` 9.0.2 installed. The reports are
written to `core/target/nextest/ci/junit.xml` and
`test-results/python.xml`; the tests and assertions are unchanged.

## Responsibility boundaries

| Owner | Responsibility |
|---|---|
| Git | Source object, commit, path and tree identities |
| Rust `excession` | Index, relations, deterministic view manifests and evidence audits |
| Native Python / JavaScript / Go adapters | Limited syntax and runtime effects for selected languages |
| Installed CodeQL / Semgrep | Candidate findings, not confirmed vulnerabilities |
| Native tests and replay | Explicit bounded observations, with unsupported cases retained |

Refer to [architecture](../docs/ARCHITECTURE.md), [CLI](../docs/CLI.md),
[development setup](../docs/DEVELOPMENT.md), and [repository views](../docs/REPOSITORY_VIEWS.md).
