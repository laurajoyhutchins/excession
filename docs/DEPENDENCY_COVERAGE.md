# Dependency recovery: independent acceptance boundary

The view-parity suite alone cannot establish dependency completeness: both
views and graph retrieval consume the same SQLite `dependency` relation.
`benchmarks/experiments/dependency_oracle.py` independently reads **pinned Git objects**,
derives module identities from file paths, parses import declarations, and
resolves in-repository targets without consulting Excession's parser output or
stored edges. A separate Node script uses TypeScript syntax nodes and a
file-set resolver to inventory JS/CJS/MJS/TS imports. The JS oracle shares
TypeScript's grammar implementation with production; its *resolution logic*
is independent.

For each fixture the oracle compares four fields per direct edge:
`(source_name, target_name, source_line, syntax)`. Missing *or unexpected*
edges fail the fixture. Source OID, complete tree reconstruction, view ID,
graph parity and corruption refusals are still checked separately. JSON
receipts preserve counts, missing/extra examples and unproven import sites.
The suite runs h11 and four immutable upstream Git commits, plus one tiny
generated adversarial source tree.

## What the green check establishes

Exact recovery of the **observed statically resolvable, indexed,
in-repository** Python import declarations and JS import/export/require/
literal-import() references for these particular fixtures. Python static imports within observed regular packages are resolved from
tracked `__init__.py` package boundaries rather than from a hard-coded `src/`
prefix. SQLite's source-path-based module identifiers remain unchanged. See
[Python importable names](PYTHON_IMPORT_NAMES.md) for the trust boundary. JS `.cjs`, `index.cjs`,
and `index.ts` candidates are considered.

The verifier also records unproven sites: computed/dynamic imports, wildcard
imports, parse errors and unresolved relative paths. None may be silently
presented as a complete runtime dependency graph. External/stdlib modules,
runtime package initialization semantics, Python namespace packages, JS
bundler aliases, TypeScript project references, conditional behavior,
code-generated files and general module-loading hooks are **not** covered by
the in-index edge denominator.

Python's `from package import name` can select a submodule or an attribute;
the statically resolvable graph includes matching package and module
candidates, not a guarantee that both were executed. Import sites nested in
functions are syntactic observations, not unconditional execution edges.

## Run

```sh
# Rust 1.99.1, Git, CPython 3.13, Node 22 and lockfile-installed TypeScript 5.8.3
cargo +1.99.1 build --offline --release --manifest-path core/Cargo.toml
python3 benchmarks/experiments/real_fixtures.py \
  --bin core/target/release/excession \
  --manifest benchmarks/fixtures/real-repositories.json \
  --out real-fixture-results.json
```

A missing or invented edge is a test failure. Each fixture creates two
damaged copies of its dependency table, deleting one true edge and adding
one false edge, and requires the independent oracle to reject both. This
demonstrates the failure mode that a view-versus-graph comparison cannot
detect when both consume the same damaged table.

In the 2026-11-11 pinned fixture run, the six fixtures recovered 568/568
independently enumerated in-scope edges, with zero extras. Three import sites
remain **outside** the claim: one Python wildcard import in h11 and two
name-computed dynamic imports in Requests, both selecting third-party
modules. Those are reported as unproven, not silently counted as successes.

A green run supports a bounded fixture result, not an assertion that all
possible dependencies in an arbitrary repository have been recovered.
