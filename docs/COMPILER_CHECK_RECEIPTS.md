# Compiler checks

Excession invokes existing native checkers and records their results against a
Git revision. Static indexing is not a substitute for compiler execution, and
a successful check is not proof of an artifact build.

## One project

Run against a clean checkout at the selected HEAD. Put the SQLite database
outside the repository.

```sh
excession check run --repo . --language rust --config Cargo.toml \
  --db /tmp/excession-checks.sqlite --trusted-local true
excession check run --repo . --language typescript --config web/tsconfig.json \
  --db /tmp/excession-checks.sqlite --trusted-local true
excession check run --repo . --language python --config mypy.ini \
  --source-prefix src --db /tmp/excession-checks.sqlite --trusted-local true

excession check audit --repo . --rev HEAD \
  --db /tmp/excession-checks.sqlite --snapshot SNAPSHOT_ID
```

| Language | Native operation | Configuration |
| --- | --- | --- |
| Rust | `cargo check --all-targets --frozen` with offline Cargo and an external target directory | Exact `Cargo.toml` |
| TypeScript | `tsc --project ... --noEmit` with external incremental build info | Exact `tsconfig.json` |
| Python | `mypy --config-file ... --no-incremental` with an external cache | Exact `mypy.ini` and optional source scope |

The executable can be selected with `--tool /absolute/path`. A zero exit
code records `PASS`. A nonzero checker exit records `FAIL`, writes JSON to
stdout, and exits 1. A missing tool, unsafe input, timeout, excessive output,
or changed checkout **refuses** instead of reporting a check result.

## Several configurations

Commit a file named `compiler-checks.json` to the repository:

```json
{
  "schema": "excession.compiler-matrix-config.v1",
  "checks": [
    {
      "language": "rust",
      "configuration": "crates/core/Cargo.toml",
      "source_prefix": null,
      "declared_inputs": ["Cargo.lock"]
    },
    {
      "language": "typescript",
      "configuration": "web/tsconfig.json",
      "source_prefix": null,
      "declared_inputs": ["package-lock.json"]
    },
    {
      "language": "python",
      "configuration": "mypy.ini",
      "source_prefix": "src",
      "declared_inputs": ["requirements/typecheck.txt"]
    }
  ]
}
```

```sh
excession check matrix --repo . --manifest compiler-checks.json \
  --db /tmp/excession-checks.sqlite --trusted-local true

excession check matrix-audit --repo . --rev HEAD \
  --db /tmp/excession-checks.sqlite --snapshot SNAPSHOT_ID
```

Use `--rust-tool`, `--typescript-tool`, or `--python-tool` with absolute
paths to select non-default installations. The matrix verifies the manifest,
every configuration, and every declared input against Git **before** running
the first checker. It runs every listed configuration even if a preceding
checker returned diagnostic errors, then stores the per-check receipts and an
aggregate `excession.compiler-matrix.v1` receipt. The aggregate is `PASS`
only when all listed checks pass.

Equivalent Python scopes `null` and `"."` refer to the same project and
cannot be listed twice. Configuration filenames must be exact. A source or
lockfile in `declared_inputs` must be Git-tracked and unchanged; the list is
an explicit declaration, not a compiler-observed dependency graph.

Limits: 1–16 checks per matrix, up to 32 declared inputs per check, 64 KiB
for the matrix manifest, 16 MiB per declared input, and 1 MiB per compiler
configuration. Inputs are checked individually rather than loaded together.

## What an audit establishes

Every `excession.compiler-check.v1` receipt contains the Git commit, tree,
configuration path and blob; selected source scope; checker launcher digest and
version; exit code; bounded output hashes and previews; and its precise
coverage limitations. The original canonical receipts and typed observations
are persisted in the existing SQLite evidence store.

`check audit` and `check matrix-audit` verify the stored evidence's integrity
and Git identities without executing the checkers again. A matrix audit also
verifies every child receipt, its declared inputs, and the aggregate status.

The following claims remain **unverified**:

- **Complete compiler inputs:** declared lockfiles are not proof of actual
  dependency consumption, transitive closure, generated code, or package
  integrity. Other feature flags, project references, environments, and
  configurations may not have been checked.
- **Independent execution:** a local launcher digest and immutable receipt
  do not attest the full toolchain or prove that an independent runner
  executed the command.
- **Artifact provenance and REA:** `cargo check`, `tsc --noEmit`, and mypy
  do not establish source-to-binary causality, deployed behavior, or REA
  evidence. The separate `evidence build-record` contract remains
  artifact-byte and declared-input evidence, not a build execution proof.

Native tools, plugins and build scripts may execute code. `--trusted-local`
explicitly permits running them on the checkout; **it is not a sandbox**.
Use an isolated worker for untrusted repositories.
