# Excession Rust core

Git is authoritative for source objects; SQLite stores indexed relations and
scoped evidence. This crate's package and library are both named `excession`.
The **one** binary is `excession`. It exposes read-only projection operations,
scanner ingestion and independent audit, and a `research` subgroup for exact
historical reproductions. The Rust engine does not claim to infer arbitrary
language semantics or confirm exploitable vulnerabilities.

## Run the checks

From the repository root:

```sh
cargo test --offline --manifest-path core/Cargo.toml
cargo fmt --manifest-path core/Cargo.toml -- --check
cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s tests -v
```

Historical differential checks remain available from inside `core/`:

```sh
bash scripts/reproduce.sh /tmp/excession-v10-fresh
python3 scripts/reproduce_v11.py /tmp/excession-v11-fresh
```

Both outputs must be empty or nonexistent. These use version-pinned bundled
Git histories, adapters, and test oracles, including immutable historical
protocol and Git attribution bytes. They are not general security tests.

## Responsibility boundaries

| Owner | Responsibility |
|---|---|
| Git | Source object, commit, path and tree identities |
| Rust `excession` | Index, relations, deterministic view manifests and evidence audits |
| Native Python / JavaScript / Go adapters | Limited syntax and runtime effects for selected languages |
| Installed CodeQL / Semgrep | Candidate findings, not confirmed vulnerabilities |
| Native tests and replay | Explicit bounded observations, with unsupported cases retained |

Refer to [architecture](../docs/ARCHITECTURE.md), [CLI](../docs/CLI.md),
[migration notes](../docs/MIGRATION.md), and [historical origins](../docs/ORIGINS.md).
