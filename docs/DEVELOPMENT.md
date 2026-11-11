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
python3 -m pip install --require-hashes --only-binary=:all: -r requirements/locks/research-linux-cp313-x86_64.txt
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
`requirements/locks/research-linux-cp313-x86_64.txt` before `make check` (as CI does). The optional Tree-sitter adapter is installed
separately:

```sh
python3 -m pip install --require-hashes --only-binary=:all: -r requirements/locks/syntax-linux-cp313-x86_64.txt
python3 -c 'from tree_sitter_language_pack import prefetch; prefetch(["rust", "go", "c", "java", "python", "javascript", "typescript"])'
python3 tools/grammar_artifacts.py verify
```

Do not download grammars during analysis. Grammar caches remain local and
are not source-controlled. Python version pins are not wheel-hash locks, and
a package version alone does not authenticate grammar binaries. Store
trusted parser artifacts and their hashes externally when the exact compiled
grammar bytes must be reproduced.

For CI, the reviewed wheel SHA-256 lock is enforced with `--require-hashes`
and `--only-binary` on CPython 3.13 Linux x86_64. Other operating systems
need independent locks. After trusted grammar prefetch,
`tools/grammar_artifacts.py record` inventories the native grammar bytes;
`verify` requires the committed hash manifest offline, with no download.
`make native-preflight` records loaded SQLite/OpenSSL versions and their
SHA-256 digests. This is a native ABI observation, not an immutable OS lock.

Run `make rust-check`, `make python-check`, or `make node-check` for a
focused loop; `make check` runs those checks together. For JUnit XML reports,
install `cargo-nextest` 0.9.151 and run `make junit`. This emits a Rust
report at `core/target/nextest/ci/junit.xml` and a Python report at
`test-results/python.xml`, without replacing the ordinary test commands. Go is isolated
behind `make go-check` and its path-filtered CI workflow. Generated SQLite
indexes, test output, and local packages are ignored; checked-in fixture
bundles and benchmark receipts are intentionally retained.

## Benchmark reports

`python3 benchmarks/reporting.py validate path/to/report.json` checks the
Draft 2020-12 schema, report content digest, and recorded raw-artifact hashes.
The validator's pins are in `requirements/research.txt`, with CI wheel
SHA-256 hashes in `requirements/locks/research-linux-cp313-x86_64.txt`.
`make benchmark-schema-check` runs conversion and negative-case tests.
Relational runs publish `report.json`; native CI wraps raw calibrations
as separate `*.report.json` files. No unavailable measurements are invented.
