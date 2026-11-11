# Compiler-assisted input discovery and measured retrieval scopes

`tools/excession_native.py` is an **optional, trusted-local compiler-assisted
adapter** to the existing language-independent Rust evidence contract. It
converts historical Git objects into a bounded temporary source tree, asks an
installed native resolver for dependencies, emits
`excession.input-declaration.v1` and destroys the temporary tree. It does not
change or check out the user's worktree and never invokes project tests.

```sh
REPO=/path/to/repository
REV=$(git -C "$REPO" rev-parse HEAD)
python3 tools/excession_native.py "$REPO" src/main.ts \
  --revision "$REV" --language typescript --trusted-local \
  --stats /tmp/excession-native-stats.json > /tmp/native-declaration.json
excession evidence contract-plan --repo "$REPO" --rev "$REV" \
  --declaration /tmp/native-declaration.json > /tmp/native-contract.json
```

To transport evidence, generate **separate** native declarations and Rust
contracts for source and target revisions, then use the existing
`excession evidence contract-compare` and `contract-audit` commands. Always
compare both revisions: an old closure alone cannot detect a new import.

## Resolution backends

| Adapter | Native source of dependency information | Explicit restrictions |
|---|---|---|
| Go | `go list -deps -test -json` with no network, toolchain upgrades, workspaces or cgo. Includes same-package build alternatives, tracked test files, reachable packages, embeds, `go.mod`/`go.sum` | External modules outside the pinned Git snapshot, cgo/SWIG and toolchain errors refuse. Generated testmain code belongs to the Go toolchain and is not a Git input. |
| JavaScript | Node CommonJS `createRequire(...).resolve` for `require`, with TypeScript compiler API parsing and ESM module resolution | Resolved packages must be tracked inside the pinned snapshot, including vendored `node_modules`. Dynamic imports, out-of-tree modules and ambiguous declaration-only runtime resolution refuse. |
| TypeScript | TypeScript compiler API with effective `tsconfig` compiler options, aliases and static module references | `extends`, project references, config plugins, ambient type references and unresolved imports refuse. Does not typecheck or execute user code. |
| Rust | `rustc --emit dep-info` on dependency-free crate roots, including tracked `include_str!` and `include_bytes!` assets | Workspaces, external dependencies, build scripts, arbitrary `env!`/`option_env!`, dynamic `include!`, and non-root entries refuse. This mode does not include `#[cfg(test)]` test compilation. |

The adapter returns `coverage: DECLARED_INPUTS_ONLY`. Native parsers improve
resolution fidelity **but do not certify full effect or input closure**. For
example, runtime `fs.readFile`, environment variables, generated code and
nondeterministic inputs are not automatically modeled. Version/byte-hash of the
native tool, relevant resolution options and host platform are recorded in
assumptions; these are **not full hermetic toolchain attestations**. Go's
standard library and TypeScript's builtin libraries are toolchain-owned, not
individually Git-bound. Never interpret a matching input set as proof that
target tests passed or as permission to skip CI.

### Security and resource boundaries

Native resolution is opt-in with `--trusted-local` because compiler parsers
have attack surfaces. This is **not a sandbox**. No network is permitted in Go
module resolution; Node package resolution must stay within staged files.
The snapshot is read-only, source bytes come from pinned Git blobs, and
staging is bounded to 4,096 regular files / 96 MiB. Contract outputs are
limited to 512 input paths / 32 MiB. Symlinks are never materialized. Refuse
missing paths, unsupported language semantics or toolchain errors rather than
substituting a smaller guessed closure. A partial observation is not a
successful contract.

## Measurable comparison

`tools/benchmark_native_resolution.py` compares the legacy static declaration,
the native declaration, and optionally Excession's indexed dependency view:

```sh
python3 tools/benchmark_native_resolution.py "$REPO" src/main.ts \
  --language typescript --graph-roots src \
  --excession-bin core/target/debug/excession --trusted-local
```

The output records wall-clock nanoseconds, declared files/bytes, staged files,
explicit refusal reasons, and symmetric path differences. **Different sets
are not a correctness ranking:** Excession's graph view may deliberately
include `.d.ts` files while a JavaScript runtime closure includes vendored
`.js`/`.cjs` files. Go/Rust indexed graph comparisons are reported unsupported
where the existing graph does not index those languages.

### Reproduced baseline (local Linux; one observation per run)

| Source corpus and entry | Static declaration | Native declaration | Indexed Excession view | Measured native time |
|---|---:|---:|---:|---:|
| Retained PostCSS source fixture (`core/fixtures/postcss-crossrepo.bundle`, commit `3f1d3ae7`), `postcss/lib/postcss.js` | Refused bare imports | **46** files / 224,679 bytes | **43** view entries | 1.87 s |
| Go 1.23.2 toolchain's vendored `golang.org/x/tools` source, copied into a reproducible local Git snapshot, `go/analysis/unitchecker/unitchecker.go` | Refused build constraints | **34** files / 733,293 bytes | Not indexed | 0.73 s |

For PostCSS the native set has **18** paths absent from the indexed view,
including vendored `nanoid`, `picocolors`, and `source-map-js` packages. The
indexed view includes **15** paths absent from the native result, chiefly
TypeScript declaration files. These are **scope discrepancies**, not proof
that either resolver fully captures execution dependencies. The Git commit
for the Go local snapshot is generated by the benchmark and is not claimed
to be an upstream x/tools commit. Timing is environment-dependent and includes
reconstructing Git blobs, tool initialization, and dependency discovery.

### Regression checks

```sh
python3 -m unittest discover -s tests -p test_native_dependency_resolution.py -v
python3 -m unittest discover -s tests -p test_dependency_discovery.py -v
```

The native suite includes an end-to-end transport/audit through the Rust CLI
across three Go revisions, TypeScript aliases, Node CommonJS resolution,
Rust `include_str!` depfiles, dynamic-import refusals, an explicit trust gate,
and the retained real PostCSS fixture. It tests **declared source coverage**,
not equivalent machine behavior or complete verification preservation.
