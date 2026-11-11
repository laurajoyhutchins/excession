# Excession CLI

`excession` is the sole first-party Rust executable and crate for repository
exploration and security-evidence inspection. It calls the Git/SQLite Rust
authority directly. Pinned historical research operations are a **subcommand
family** (`excession research`), not a second executable.

## Install

Prerequisites: Rust 1.99.1, Cargo, Git (SHA-1 object repositories), and system `libsqlite3` and `libcrypto`. No third-party Rust crates or network downloads are required.

```sh
# From the repository root; one binary is installed.
cargo install --offline --locked --path core --bin excession
excession --version
excession --help
```

For development, `make cli` produces `core/target/debug/excession`. `make check` runs the existing source/evidence tests and the dedicated CLI tests.

## One repository, many layouts

The following example uses an **existing clean** h11 checkout and writes SQLite and manifests outside the repository. The indexer currently resolves static Python/JavaScript dependencies within configured roots. Use the actual root directories for your project.

```sh
REPO=/path/to/h11
OUT=/tmp/excession-h11
mkdir -p "$OUT"

excession index --repo "$REPO" --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --roots h11,tests

excession view deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._connection > "$OUT/connection.json"
excession view deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._headers > "$OUT/headers.json"

# Re-execute the original graph query against the saved manifest.
excession view replay deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._connection --manifest "$OUT/connection.json"

# Mount separately to preserve both virtual focus groups.
excession view mount --repo "$REPO" --manifest "$OUT/connection.json" \
  --namespace connection > "$OUT/mounted-connection.json"
excession view mount --repo "$REPO" --manifest "$OUT/headers.json" \
  --namespace headers > "$OUT/mounted-headers.json"
excession view compose --repo "$REPO" --op union \
  --left "$OUT/mounted-connection.json" --right "$OUT/mounted-headers.json" \
  > "$OUT/combined.json"

# No source files are copied for any of these virtual operations.
excession view ls --repo "$REPO" --manifest "$OUT/combined.json"
excession browse --repo "$REPO" --manifest "$OUT/combined.json"
```

In the browser use `ls [directory]`, `cd directory`, `pwd`, `stat file`, `show file`, `help`, or `quit`. The browser reads Git blobs on demand and limits rendered text to 8192 bytes by default. Use `view cat` for exact raw bytes:

```sh
excession view stat --repo "$REPO" --manifest "$OUT/connection.json" \
  --path views/00-focus/h11/_connection.py
excession view cat --repo "$REPO" --manifest "$OUT/connection.json" \
  --path views/00-focus/h11/_connection.py
```

A virtual path must be obtained from the manifest or `view ls`. The code above is a representative h11 path, not a generic path for other projects.

## Python import roots in monorepositories

By default, module identities follow their Git paths. Some repositories put a
nested package directory directly on Python's import search path, so a source
file such as `scripts/gdb/linux/tasks.py` imports `linux.utils`, not
`scripts.gdb.linux.utils`. Set an explicit physical `--import-base`
containing all indexed roots:

```sh
# LINUX is a clean checkout of torvalds/linux, for example v6.12.
excession index --repo "$LINUX" --db "$OUT/linux.sqlite" --name linux \\
  --rev HEAD --roots scripts/gdb --import-base scripts/gdb
excession view deps --db "$OUT/linux.sqlite" --name linux \\
  --rev HEAD --seed linux.tasks > "$OUT/tasks.json"
```

The Git commit, source paths, file modes and blob OIDs remain unchanged.
This mapping only changes logical Python module names for the given index,
and it does not infer runtime `sys.path`, dynamic imports, C includes,
Kbuild, or Kconfig dependencies. Different module naming configurations
should use distinct databases.

## Optional physical materialization

```sh
excession view materialize --repo "$REPO" --manifest "$OUT/combined.json" \
  --output "$OUT/read-only-tree"
excession view audit --repo "$REPO" --manifest "$OUT/combined.json" \
  --output "$OUT/read-only-tree"
```

Materialization requires a new **absolute** destination outside the source checkout, produces mode-0444 regular files, and audits every file against its original Git blob. Materialized views are **not runnable checkouts** and cannot be used to claim test or security equivalence.

## Scanner evidence

For an already installed Semgrep scanner and a reviewed, local ruleset, the CLI can execute the scanner without a shell, then import and independently replay the resulting evidence:

```sh
excession security scan --repo "$REPO" --report "$OUT/report.json" \
  --db "$OUT/security.sqlite" --engine semgrep \
  --config /path/to/reviewed-rules.yaml --trusted-local true
```

For CodeQL, use `--engine codeql --codeql-db /path/to/prebuilt/codeql-db` and a reviewed local query suite for `--config`. Use `--scanner /path/to/executable` when the engine binary is not on `PATH`. The default timeout is 300 seconds; `--timeout-seconds` supports 1 through 300. The source checkout must remain clean, the report must not already exist, and report/database files must be outside the checkout. Output paths are required to be absolute.

**This is trusted-local execution, not a sandbox.** In particular, CodeQL database creation can execute repository build steps; build databases in a separately reviewed, isolated process. Scanner rule packs and executables must also be trusted. The CLI neither installs tools nor downloads rules. Report provenance is not a signed attestation. A controlled fake-scanner integration test verifies orchestration; it does not establish that CodeQL or Semgrep is installed here.

After obtaining an actual scanner report through an authorized workflow:

```sh
excession security ingest --repo "$REPO" --report "$OUT/report.sarif" \
  --db "$OUT/security.sqlite" --engine codeql
excession security audit --repo "$REPO" --report "$OUT/report.sarif" \
  --db "$OUT/security.sqlite" --engine codeql

# The run ID comes from the ingestion receipt.
excession view security --db "$OUT/security.sqlite" --repo "$REPO" \
  --run YOUR_64_HEX_RUN_ID > "$OUT/security-view.json"
excession view replay security --db "$OUT/security.sqlite" --repo "$REPO" \
  --run YOUR_64_HEX_RUN_ID --manifest "$OUT/security-view.json"
```

The `tools/pipeline.py` scanner runner uses the dedicated CLI by default. CodeQL and Semgrep remain optional external programs. An imported report records **unverified static candidates**, not confirmed CVEs. Before trusting a security view, independently audit the underlying scanner ingestion as shown above.

## Compiler-backed C and C++ analysis

For a chosen C/C++ source file and one JSON compilation database entry,
`analyze clang` records real Clang AST declarations and calls, actual
preprocessor dependency paths, and source/flag/headers/toolchain identities.
This works on any SHA-1 Git repository and requires explicit trusted-local
execution permission.

```sh
excession analyze clang \
  --repo /path/to/project --rev HEAD \
  --source src/main.cpp --compile-db /path/to/build/compile_commands.json \
  --trusted-local true > /tmp/main-clang.json
```

`excession kernel clang` is a compatibility alias to the exact same Rust and
Clang code. It does not require or infer Linux Kbuild semantics. Clang analysis
is a one-translation-unit observation, not a successful build or complete
call graph. See [compiler-backed evidence](COMPILER_ANALYSIS.md).

## Configured target graph assembly

Use CMake's File API codemodel and a directory of Clang receipt JSON files
to join selected target sources, configured target dependencies, generated
artifacts, preprocessor inputs, declarations, and name-only call candidates:

```sh
excession graph assemble \
  --repo /path/to/project --rev HEAD \
  --build /tmp/cmake-debug --target example \
  --observations /tmp/clang-receipts --db /tmp/configured.sqlite
```

Assembly reads existing build/analysis evidence; it does not execute
the build or invoke Clang. A missing receipt yields explicit partial coverage,
not a claim of complete dependency resolution. The selected target's graph
is a configured snapshot, not proof of execution. See
[configuration-scoped graph assembly](CONFIGURED_GRAPHS.md).

## Complete command families

- `excession index`: index an exact Git revision and configured dependency roots.
- `excession analyze clang`: Clang-backed, build-configuration-scoped C/C++ source observations. `kernel clang` is an alias.
- `excession graph assemble`: reconcile exact Git source blobs, configured CMake target structure and prior Clang receipts into SQLite.
- `excession inspect dependencies|manifest|impact|review`: query indexed relations. Revision flags accept Git refs such as `HEAD`, resolved to commit IDs.
- `excession security scan|ingest|audit`: run an installed trusted scanner, or import and independently recheck CodeQL SARIF or Semgrep JSON.
- `excession view deps|security`: derive deterministic, source-bound views.
- `excession view compose|select|mount`: combine and filter read-only views.
- `excession view replay deps|security|compose|select|mount`: regenerate the result from the original inputs and compare with the saved manifest. For composition, selection and mounting the saved result is named by `--output`.
- `excession view ls|stat|cat`: list, inspect or read exact virtual source entries on demand.
- `excession view materialize|audit`: generate a read-only physical tree or verify it.
- `excession browse`: terminal browser, without a Python process or copied checkout.

All file contents and view identities remain grounded in Git objects. The CLI does not silently turn a candidate into a vulnerability, execute untrusted code, install scanners, or claim that unindexed edges do not exist. JSON commands write canonical JSON to standard output; `view cat` writes raw bytes. Errors and refusals go to standard error and return a nonzero exit code. The CLI uses strict named options and refuses unknown or duplicate flags.

**Historical compatibility:** use `excession research COMMAND [historical arguments]`
for bounded reproduction operations such as `carry`, `compose`, `replay`,
`contracts-import`, `identity`, `contract-impact`, `score`, and `project`.
There is no separately installed legacy binary, and the crate is named
`excession`. Versioned on-disk protocols and exact-replay fixture identities
are retained. See [migration notes](MIGRATION.md).
