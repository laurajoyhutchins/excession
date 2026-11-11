# Development environment

Excession is a Rust application with small Python, JavaScript/TypeScript, and
Go research adapters. Do not introduce a second Rust package or a Python
application packaging layer just to manage the tests.

## Pinned tools

| Tool | Version source | Purpose |
| --- | --- | --- |
| Rust 1.99.1 | `rust-toolchain.toml` | CLI, library, Clippy, rustfmt |
| Python 3.13.5 | `.python-version` | Standard-library integration tests and optional syntax bridge |
| Node 22.16.0 / npm 10.9.2 | `.node-version`, `package.json` | Local TypeScript compiler used by JS adapters |
| Go 1.26.11 | `.go-version` | Optional, standard-library-only Go research adapter |

Native SQLite and OpenSSL libraries, the Git executable, and the operating
system are still system dependencies. These files pin language toolchains, not
a complete hermetic OS image.

## Setup and checks

```sh
npm ci --ignore-scripts --no-audit --no-fund
python3 -m pip install -r requirements/research.txt
make check
make go-check                  # optional Go research adapter
```

The Rust dependency graph is empty; `core/Cargo.lock` remains committed, and
Cargo commands use both `--locked` and `--offline`. The JS adapters import
`typescript` from the repository's locked `node_modules`, never from a
machine-global npm installation. `npm ci` verifies the tarball integrity
recorded in `package-lock.json`.

The direct Python integration suite uses `unittest`, but Rust adversarial
checks run a Python research probe that imports pytest. Install the pinned
`requirements/research.txt` before `make check` (as CI does). The optional Tree-sitter adapter is installed
separately:

```sh
python3 -m pip install -r requirements/syntax.txt
python3 -c 'from tree_sitter_language_pack import prefetch; prefetch(["rust", "go", "c", "java", "python", "javascript", "typescript"])'
```

Do not download grammars during analysis. Grammar caches remain local and
are not source-controlled. Python version pins are not wheel-hash locks, and
a package version alone does not authenticate grammar binaries. Store
trusted parser artifacts and their hashes externally when the exact compiled
grammar bytes must be reproduced.

Run `make rust-check`, `make python-check`, or `make node-check` for a
focused loop; `make check` runs those checks together. For JUnit XML reports,
install `cargo-nextest` 0.9.151 and run `make junit`. This emits a Rust
report at `core/target/nextest/ci/junit.xml` and a Python report at
`test-results/python.xml`, without replacing the ordinary test commands. Go is isolated
behind `make go-check` and its path-filtered CI workflow. Generated SQLite
indexes, test output, and local packages are ignored; checked-in fixture
bundles and benchmark receipts are intentionally retained.
