# Excession CLI

`excession` is the supported Rust executable for repository exploration,
source-bound evidence, and bounded analysis. A single `clap` command tree
defines the complete interface, including nested help and strict options.
There is no parallel positional interface.

## Install

Prerequisites: Rust 1.99.1, Cargo, Git (SHA-1 object repositories), and system SQLite and crypto libraries where required. Dependencies are pinned in `core/Cargo.lock`. Fetch them once before using offline builds.

```sh
# From the repository root.
cargo fetch --locked --manifest-path core/Cargo.toml
cargo install --offline --locked --path core --bin excession
excession --version
excession --help
excession view replay deps --help
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

## Optional physical materialization

```sh
excession view materialize --repo "$REPO" --manifest "$OUT/combined.json" \
  --output "$OUT/read-only-tree"
excession view audit --repo "$REPO" --manifest "$OUT/combined.json" \
  --output "$OUT/read-only-tree"
```

Materialization requires a new **absolute** destination outside the source checkout, produces mode-0444 regular files, and audits every file against its original Git blob. Materialized views are **not runnable checkouts** and cannot be used to claim test or security equivalence.

## Refactoring hypotheses

```sh
excession refactor suggest --db /tmp/index.sqlite --name project --rev HEAD > /tmp/refactor.json
excession refactor audit --db /tmp/index.sqlite --name project --rev HEAD --report /tmp/refactor.json
```

Both commands require an indexed Git revision. Results are unverified
refactoring hypotheses, not safe edits. The audit recomputes indexed relations
without rerunning the language parsers. See [refactoring hypotheses](REFACTORING.md).

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

## Complete command families

- `excession index`: index an exact Git revision and configured dependency roots.
- `excession refactor suggest|audit`: emit or replay bounded source-bound refactoring hypotheses.
- `excession inspect dependencies|manifest|impact|review`: query indexed relations. Revision flags accept Git refs such as `HEAD`, resolved to commit IDs.
- `excession security scan|ingest|audit`: run an installed trusted scanner, or import and independently recheck CodeQL SARIF or Semgrep JSON.
- `excession view deps|security`: derive deterministic, source-bound views.
- `excession view compose|select|mount`: combine and filter read-only views.
- `excession view replay deps|security|compose|select|mount`: regenerate the result from the original inputs and compare with the saved manifest. For composition, selection and mounting the saved result is named by `--output`.
- `excession view ls|stat|cat`: list, inspect or read exact virtual source entries on demand.
- `excession view materialize|audit`: generate a read-only physical tree or verify it.
- `excession browse`: terminal browser, without a Python process or copied checkout.
- `excession syntax native-run`: compare scoped compiler observations with source-bound declarations.
- `excession inspect evidence`: inspect focused, bounded source and runtime evidence.
- `excession benchmark validate|report`: validate or assemble source-bound benchmark reports.

All file contents and view identities remain grounded in Git objects. The CLI does not silently turn a candidate into a vulnerability, execute untrusted code, install scanners, or claim that unindexed edges do not exist. JSON commands write canonical JSON to standard output; `view cat` writes raw bytes. Errors and refusals go to standard error and return a nonzero exit code. The CLI parses every command exactly once with `clap`, and refuses unknown or duplicate flags.

The retired `research` positional interface is not installed or accepted.
Source history remains available through Git; supported workflows are
implemented directly by the commands listed above.
