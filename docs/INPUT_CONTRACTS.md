# Language-neutral input contracts

**Status:** implemented Rust/SQLite declared-input transport. This deliberately does
not infer a complete dependency graph or attest to an external execution.

A language adapter (or a user) declares which tracked Git files and which
environment assumptions a computation relies upon. The *same Rust core* then
validates the declaration against any retained Git commit, computes a canonical
content identity, compares it to another revision and persists an independently
auditable transport edge. No agent, merge, checkout, Python runtime, or specific
programming language is required.

## Declaration

Create `/tmp/inputs.json` for a Go test (JavaScript, TypeScript, Rust, C and
other languages use the **identical** format):

```json
{
  "schema": "excession.input-declaration.v1",
  "coverage": "DECLARED_INPUTS_ONLY",
  "language": "go",
  "subject": "calc_test.go",
  "inputs": [
    {"path": "calc.go", "role": "source"},
    {"path": "calc_test.go", "role": "source"},
    {"path": "go.mod", "role": "configuration"}
  ],
  "assumptions": {"go-toolchain": "pinned-go-version", "network": "off"}
}
```

Inputs must be sorted by path, unique, regular tracked blobs and total at most
32 MiB across no more than 512 entries. Roles are `source`, `asset` or
`configuration`. The subject must occur in the list. Assumptions are explicit
*declarations*, not facts Excession independently verified. An adapter cannot
claim that `DECLARED_INPUTS_ONLY` is a complete program dependency closure.

## From historical revisions to comparable contracts

```sh
BIN=./core/target/debug/excession
REPO=/path/to/repository
OLD=0123456789abcdef0123456789abcdef01234567
NEW=89abcdef0123456789abcdef0123456789abcdef

"$BIN" evidence contract-plan --repo "$REPO" --rev "$OLD" \
  --declaration /tmp/inputs.json > /tmp/input-old.json
"$BIN" evidence contract-plan --repo "$REPO" --rev "$NEW" \
  --declaration /tmp/inputs.json > /tmp/input-new.json
"$BIN" evidence contract-compare --repo "$REPO" \
  --source-contract /tmp/input-old.json --target-contract /tmp/input-new.json \
  --db /tmp/input-evidence.sqlite
"$BIN" evidence contract-audit --repo "$REPO" \
  --source-contract /tmp/input-old.json --target-contract /tmp/input-new.json \
  --db /tmp/input-evidence.sqlite
```

The Rust core accepts `HEAD` or an exact lowercase 40-character SHA-1 commit
identifier. It works directly from Git objects, without checking out either
revision or trusting the working tree. It records the source/target commits,
exact selected blob SHA-1, independent SHA-256, Git mode, byte size, input role,
subject, language, and declared environmental assumptions.

`DECLARED_INPUTS_MATCH_ONLY` means *every listed input and every declaration
matches*. `REVERIFY_REQUIRED` means at least one does not. The comparison
reports changed paths and separate flags for subject, language, and assumptions.
Both decisions explicitly set `source_behavior_verified=false` and
`target_behavior_verified=false`. The contract is a portable **dependency
identity**, not an execution receipt, verified effect set, semantic equivalence
proof, or license to skip tests. Unlisted files, dynamic imports, compiler
behavior, environment influence, network and hidden effects are **outside this
contract**.

The `contract-compare` operation persists canonical source and target contract
manifests plus a canonical result in SQLite; `contract-audit` independently
recomputes them from Git objects and checks the entire row. Duplicate inserts,
forged blob IDs, altered declaration roles, missing tracked inputs, unexpected
Git modes, noncanonical content hashes and changed SQLite rows are rejected.
A new comparison requires a new row, preserving earlier historical evidence.

## Relationship to existing Python evidence

`core/adapters/python_slice_imports.py` computes the bounded CPython static closure as a language adapter; Rust constructs and validates executable-slice evidence and
produces its own finite execution receipt. Its historical transport now shares
the **same Rust changed-path comparison primitive** as the language-neutral
contract, but retains its stronger source-receipt requirements and conservative
legacy state names. Existing Python manifests and receipts are unchanged.

Go and JavaScript tests demonstrate the new language-neutral contract using
*actual* `go test` and `node --test` executions as independent oracles. Those
oracles are **not** automatically authenticated or ingested by this API.
Language-native dependency resolvers and an enforced hermetic effect contract
are future adapters. Do not substitute a manually declared list for one.

## Verification

```sh
cargo fmt --manifest-path core/Cargo.toml -- --check
cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
cargo test --offline --manifest-path core/Cargo.toml
python3 -m unittest discover -s tests -p 'test_language_contracts.py' -v
```

The Go/Node integration tests create disposable real Git repositories,
independently run test commands, transport across historical commits, and
exercise forged contracts, environment/role changes, regressions and SQL
tampering. No network fetches, package downloads or global installs required.
