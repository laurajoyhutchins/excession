# Migration: RelGit to Excession

The Rust crate, library, and only installed binary are now named **`excession`**.
This is an operational migration, not a rewriting of historical research.

## Commands

| Previously | Now |
|---|---|
| `cargo install --path core --bin relgit` | `cargo install --offline --locked --path core --bin excession` |
| `relgit view-plan-deps DB NAME REV SEED` | `excession view deps --db DB --name NAME --rev REV --seed SEED` |
| `relgit security-ingest REPO REPORT DB ENGINE` | `excession security ingest --repo REPO --report REPORT --db DB --engine ENGINE` |
| `relgit security-ingest-audit REPO REPORT DB ENGINE` | `excession security audit --repo REPO --report REPORT --db DB --engine ENGINE` |
| `relgit view-ls REPO MANIFEST [DIR]` | `excession view ls --repo REPO --manifest MANIFEST [--path DIR]` |
| `relgit view-open REPO MANIFEST FILE` | `excession view cat --repo REPO --manifest MANIFEST --path FILE` |
| `relgit carry ROOT DB REPORT` | `excession research carry ROOT DB REPORT` |
| `relgit compose SPEC DB` | `excession research compose SPEC DB` |
| `relgit project SPEC DB DIR` | `excession research project SPEC DB DIR` |

For other pinned, positional research commands, use
`excession research OLD_COMMAND [OLD_ARGS...]`. The research subprocess runs
within the same executable and uses the same Rust library; it is not a shell
wrapper or a separate binary.

Rust embedding clients should change `relgit::` imports to `excession::`.
Cargo's package and library names have changed, so downstream `Cargo.lock`
files should be regenerated or updated normally.

## Explicitly preserved history

Historical Git-commit author metadata, generated source for exact projection
parity, test fixture labels, and versioned wire/schema names are deliberately
unchanged. Changing those would break source hashes, replay receipts, or
stored evidence without adding meaningful functionality. No old executable
is built, installed, or required by current tests and scripts.

The migration must retain one-command local verification (`make check`),
exact parity on the retained fixture experiments, and strict refusal behavior
for malformed Git or scanner evidence.
