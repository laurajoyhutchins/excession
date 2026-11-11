# Configuration-scoped graph assembly (v1)

`excession graph assemble` is a native Rust/SQLite evidence join. It
**does not parse source languages or build scripts**. CMake File API codemodel
v2 supplies configured targets, selected sources, artifacts and declared target
dependencies; `excession analyze clang` supplies previously generated C/C++
source observations; Git identifies the historical source snapshot.

## Prepare independent observations

The producer build must be **trusted local**. Build a pinned checkout with
CMake/Ninja, leaving the build directory outside the source repository:

```sh
REPO=/path/to/fmt
BUILD=/tmp/fmt-debug
OUT=/tmp/fmt-observations
mkdir -p "$BUILD/.cmake/api/v1/query" "$OUT"
touch "$BUILD/.cmake/api/v1/query/codemodel-v2"

cmake -S "$REPO" -B "$BUILD" -G Ninja \
  -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_BUILD_TYPE=Debug \
  -DFMT_DOC=OFF -DFMT_TEST=OFF -DFMT_INSTALL=OFF
cmake --build "$BUILD" --target fmt

# One source-bound receipt for *each* selected C/C++ source file.
excession analyze clang \
  --repo "$REPO" --rev HEAD --source src/format.cc \
  --compile-db "$BUILD/compile_commands.json" \
  --compiler clang++ --trusted-local true > "$OUT/format.json"
excession analyze clang \
  --repo "$REPO" --rev HEAD --source src/os.cc \
  --compile-db "$BUILD/compile_commands.json" \
  --compiler clang++ --trusted-local true > "$OUT/os.json"
```

The source names above reflect a representative fmt target; check the
actual CMake File API reply for the selected target instead of assuming these
are always the complete source set.

Then assemble without invoking any compiler or build process:

```sh
excession graph assemble \
  --repo "$REPO" --rev HEAD \
  --build "$BUILD" --target fmt \
  --observations "$OUT" \
  --db /tmp/excession-configured.sqlite > /tmp/graph.json
```

## Reused authorities and evidence boundaries

- **Git:** validates every selected tracked source blob against the pinned
  commit and tree. Only source input blobs are Git-authenticated.
- **CMake File API:** validates codemodel v2 build/source paths; imports
  configured target dependencies, source membership and expected artifacts.
  `CMAKE_CONFIGURED` never means a target actually built successfully.
- **Clang receipts:** verifies schema, canonical report SHA-256, source
  commit/tree/blob and the full compilation database SHA-256. Re-hashes
  observed dependency files on import, refusing stale evidence.
- **SQLite:** stores immutable graph snapshots, entities and typed
  relationships, with the edge's status and producing evidence receipt.
  Importing the same snapshot again reuses its identity.

Edges include `DEPENDS_ON_TARGET`, `TARGET_SOURCE`,
`PRODUCES_ARTIFACT`, `OBSERVED_AS_TU`, `PREPROCESSOR_INPUT`,
`DECLARES`, `CONTAINS_CALL`, and optional `MAY_REFER_TO` for
name-only within-TU candidates. **No cross-TU link resolution is
claimed**. C++ overloads, virtual dispatch, generated sources and
noncompiler build actions require more evidence.

## Completeness and refusal

`TARGET_TUS_OBSERVED` means every tracked C/C++ source *listed by the
selected CMake target* has a compatible receipt. Missing receipts instead
yield `PARTIAL_TARGET_OBSERVATIONS`, with exact missing paths. That
status does not imply that all transitive target dependencies are analyzed,
all link/compile actions executed, or every runtime edge is covered.

Assembly refuses ambiguous receipts, conflicting Git identities, modified
source files, a changed dependency, mismatched compilation databases,
unsupported codemodel major versions, or a codemodel for a different
checkout/build directory. Only a single configured target and CMake
configuration are accepted per report. The scope is bounded to 128
translation units, with explicit memory/edge/JSON budgets.

The graph is configuration-scoped: selecting Debug versus Release yields
different graph identities even when the source Git blobs are identical.
The build directory is retained in the receipt so local generated-file
provenance cannot be accidentally conflated across builds.

## Independent acceptance

`.github/workflows/configured-graph.yml` fetches pinned fmt 11.1.4,
builds the target **for real** in separate Debug and Release directories,
and uses CMake's own compilation database. It derives the complete
selected-target C/C++ source list from the File API, obtains a Clang receipt
for each, assembles the graph twice to test stable identity, compares
source/artifact closure against Ninja's `-t graph`, verifies SQLite rows,
and ensures missing or tampered receipts cannot masquerade as complete.

This proves configured build structure and local compiler observation
interoperability for the selected target. It is **not** independent
compiler-level symbol resolution, dynamic reachability, or proof of the
build's operational correctness.
