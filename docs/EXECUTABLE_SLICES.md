# Rust-owned executable slices and historical evidence carry

**Status: Rust-authoritative planning, materialization, execution, source auditing,
receipts, and historical carry.** Only the bounded *CPython AST import semantics*
remain a Python adapter (`core/adapters/python_slice_imports.py`). The
`tools/excession_slice.py` command is a backward-compatible client and contains
no Git, SQLite, trust, source-generation, or receipt-identity authority.

The public schema and conservatism are unchanged:
`excession.executable-slice.v1`, `excession.slice-observation.v1`, and
`excession.evidence-carry.v1`. Their existing Git/receipt identities continue
to validate, including pre-migration fixtures with ASCII source paths.

## Authority boundary

| Responsibility | Owner |
| --- | --- |
| Pinned SHA-1 Git revision and tracked file modes/blobs | Rust `git` and `slice_runtime` |
| Static Python import and top-level export analysis | CPython `ast` adapter, via bounded stdin/stdout JSON |
| Canonical manifest creation, `view_id`, SHA-256 | Rust `slice_runtime` |
| Read-only materialization and post-execution source audit | Rust `slice_runtime` |
| Trusted-local and optional namespace execution | Rust process supervisor |
| Exit/output digest receipt, conservative source carry | Rust `slice_runtime` |
| Receipt ingestion and independent SQLite audit | Rust `slice_evidence` and `evidence_transport` |

Rust independently rebuilds the selected static import closure by calling the
language adapter on authenticated Git data before materialization, audit,
execution, and carry. The AST adapter has no access to Git or SQLite and cannot
construct or validate evidence on its own.

**An import closure is not a complete runtime effect contract.** A manifest or
receipt matching source blobs does not imply that a later revision passed the
test or can safely skip CI. Receipts are unsigned local observations.

## Native CLI

```bash
REPO=/path/to/checkout
COMMIT=$(git -C "$REPO" rev-parse HEAD)

# Strict clean HEAD by default. Add `--history true` for any retained commit.
excession slice plan --repo "$REPO" --rev "$COMMIT" \
  --entry tests/check.py --assets '["tests/input.json"]' \
  > /tmp/slice.json

excession slice validate --repo "$REPO" --manifest /tmp/slice.json
excession slice materialize --repo "$REPO" --manifest /tmp/slice.json \
  --directory /tmp/excession-slice
excession slice source-audit --repo "$REPO" --manifest /tmp/slice.json \
  --directory /tmp/excession-slice

# Execution requires explicit local consent. Do not run unreviewed code.
excession slice run --repo "$REPO" --manifest /tmp/slice.json \
  --directory /tmp/excession-slice --timeout 10 --trusted-local true \
  --isolated-linux true > /tmp/receipt.json

# Different operation: authenticate and store a receipt in SQLite.
excession slice ingest --repo "$REPO" --manifest /tmp/slice.json \
  --receipt /tmp/receipt.json --db /tmp/evidence.sqlite
excession slice audit --repo "$REPO" --manifest /tmp/slice.json \
  --receipt /tmp/receipt.json --db /tmp/evidence.sqlite

# Compare the prior passing observation against a later or divergent commit.
excession slice carry --repo "$REPO" --manifest /tmp/slice.json \
  --receipt /tmp/receipt.json --rev "$OTHER_COMMIT"

# Optional fresh target execution: produces a SEPARATE unsigned receipt.
excession slice carry --repo "$REPO" --manifest /tmp/slice.json \
  --receipt /tmp/receipt.json --rev "$OTHER_COMMIT" \
  --replay true --trusted-local true --isolated-linux true
```

The Python script still accepts the earlier `plan`, `materialize`, `audit`,
`run` and `carry` positional interface. Its JSON result and exception shape are
preserved for existing tests and scripts. Unlike the native CLI, the script's
`audit` verb means **source-directory audit**; the native `slice audit` verb
means **SQLite receipt audit**. Use `slice source-audit` for the former.

## Explicit unsupported cases

- Only tracked, nonexecutable regular Git blobs. Source symlinks, gitlinks,
  missing modules, untracked files and source mutations are refused.
- Strict plan requests require clean HEAD. Historical plans and carry can
  read any retained commit without resetting HEAD or touching the worktree.
- The AST adapter supports ordinary local Python packages with `__init__.py`.
  Namespace packages, external dependencies, star imports, unresolved symbols,
  and common dynamic code-loading constructs are refused. Completeness of
  runtime file reads, environment variables, reflection and native extensions
  remains unverified.
- Bounded closure: 512 files and 32 MiB. The Rust planner currently bounds the
  Python inventory it supplies to CPython to 4,096 files and 32 MiB. Huge
  repositories may refuse even when a smaller actual closure exists. Paths
  that are non-ASCII are currently refused pending an identity-version update.
- The worker has bounded output (1 MiB per stream) and wall time (1–60 seconds).
  Rust kills its subprocess group on timeout to avoid orphaned observers
  retaining output pipes. Read-only materialized files are re-audited after
  execution. Execution that writes extra files inside the slice is refused.
- With `--isolated-linux true`, a supported local Linux host uses user, mount,
  PID, network, IPC and UTS namespaces; readonly source mount; private chroot;
  dropped capabilities; `no_new_privs`; resource limits; and a worker-ready
  marker emitted *after* setup. Failure refuses, never falls back to a plain
  process. The marker is not cryptographic attestation. Kernel shared-state,
  privileged filesystem effects and host trust still matter. Treat both
  execution modes as **trusted-local only**.
- A matching source closure returns `SOURCE_MATCH_ONLY` with
  `target_behavior_verified:false`. Changed source or imports require fresh
  verification (`REVERIFY_REQUIRED`). No inherited test-pass claim is issued.

## Verification

```bash
cargo fmt --all --manifest-path core/Cargo.toml -- --check
cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
cargo test --offline --manifest-path core/Cargo.toml
python3 -m unittest discover -s tests -p 'test_executable_slices.py' -v
python3 -m unittest discover -s tests -p 'test_evidence_history.py' -v
python3 -m unittest discover -s tests -p 'test_slice_rust_migration.py' -v
```

The migration regression reconstructs the expected manifest from independent
Git blob identities, verifies receipt hashes and target replay, and tests dirty
historical worktrees, namespace no-fallback, refusal on forged manifests,
source outside the checkout, and strict trust opt-in. A separate local
differential test of pre-migration versus Rust implementation established
identical v1 manifests, receipts and carry decisions for a pinned Git fixture.

This migration does **not** add runtime dependency observation or cryptographic
remote-worker attestation. Those remain distinct proof obligations, not
features inferred from moving Python implementation into Rust.
