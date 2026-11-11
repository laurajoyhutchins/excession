# Conservative Git-native dependency discovery

`tools/excession_discover.py` constructs **declared-input** JSON for the Rust
`evidence contract-plan` command. It reads a pinned historical Git commit, not
the worktree, and runs no repository code. The same contract works for any
pair of revisions, including divergent branches and unmerged histories.

```sh
python3 tools/excession_discover.py "$REPO" src/main.rs \
  --language rust --revision "$OLD_SHA" > /tmp/old-declaration.json
excession evidence contract-plan --repo "$REPO" --rev "$OLD_SHA" \
  --declaration /tmp/old-declaration.json > /tmp/old-contract.json

python3 tools/excession_discover.py "$REPO" src/main.rs \
  --language rust --revision "$NEW_SHA" > /tmp/new-declaration.json
excession evidence contract-plan --repo "$REPO" --rev "$NEW_SHA" \
  --declaration /tmp/new-declaration.json > /tmp/new-contract.json

excession evidence contract-compare --repo "$REPO" \
  --source-contract /tmp/old-contract.json --target-contract /tmp/new-contract.json \
  --db /tmp/transport.sqlite
```

**Always regenerate the declaration separately at both revisions.** Reusing
only the old declaration can miss dependencies added at the target revision.
The Rust planner and SQLite auditor are authoritative for Git identity. This
adapter only discovers candidate paths and roles.

## Explicitly supported subset

* **JavaScript / TypeScript:** static local relative import/export and literal
  `require` edges, resolving unique tracked source files and index files.
  Includes root package and TypeScript configuration/lockfiles. Bare imports,
  dynamic imports, aliases, extends and references fail closed.
* **Go:** all regular `.go` source and test files in the entry's package, plus
  recursively imported packages under the declared Go module; includes
  `go.mod` and `go.sum`. External modules, replace directives, go.work,
  build tags, generated/embedded inputs and CGO require a native Go resolver.
* **Rust:** deliberately overapproximates a simple Cargo crate with every
  tracked `.rs` source under `src/`, `tests/`, `examples/`, and `benches/`.
  Includes Cargo manifests and optional lock/toolchain files. Workspaces, path
  dependencies, build scripts, `include!`, feature/target effects and advanced
  Cargo configuration require cargo/rustc metadata; unsupported cases refuse.

This is **not a general parser**. Language-specific syntax in strings/comments,
macro expansion, conditional compilation, environment variables, filesystem
reads, generated files and execution side effects are not comprehensively
understood. The current adapter is best used for bounded experiments with
explicit assumptions and independently checked closure; it does not certify
completeness or safe test skipping. In particular, a declared file set can
underrepresent dependencies for constructs the adapter does not recognize.

The output `coverage` is always `DECLARED_INPUTS_ONLY`, and Rust's strongest
positive transport decision remains `DECLARED_INPUTS_MATCH_ONLY`, not a
behavioral proof or independently established test pass. Re-execute when
uncertainty remains. A future native parser or compiler/LSP-backed adapter can
produce the **same** contract without changing the Rust transport core.

Run offline smoke tests:

```sh
python3 -m unittest discover -s tests -p 'test_dependency_discovery.py' -v
```
