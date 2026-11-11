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
REPO=/path/to/zlib
BUILD=/tmp/zlib-debug
OUT=/tmp/zlib-observations
mkdir -p "$BUILD/.cmake/api/v1/query" "$OUT"
touch "$BUILD/.cmake/api/v1/query/codemodel-v2"

cmake -S "$REPO" -B "$BUILD" -G Ninja \
  -DCMAKE_C_COMPILER=clang \
  -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_BUILD_TYPE=Debug \
  -DZLIB_BUILD_EXAMPLES=ON
cmake --build "$BUILD" --target example

# The same test/example.c is also compiled into example64 on platforms
# with off64_t. Select the specific object action, not a guessed first entry.
excession analyze clang \
  --repo "$REPO" --rev HEAD --source test/example.c \
  --compile-db "$BUILD/compile_commands.json" \
  --object-prefix CMakeFiles/example.dir/ \
  --compiler clang --trusted-local true > "$OUT/example.json"
```

The selected target must have one source-bound receipt for every Git-tracked
C/C++ translation unit listed in its CMake File API reply. The optional
`--object-prefix` selects an unambiguous output action when multiple
configured compilation commands name the same source; Excession refuses
ambiguous matches rather than picking the first entry. The selected action
is a local compiler observation, not proof that a linker's symbol identity
has been resolved.

Then assemble without invoking any compiler or build process:

```sh
excession graph assemble \
  --repo "$REPO" --rev HEAD \
  --build "$BUILD" --target example \
  --observations "$OUT" \
  --db /tmp/excession-configured.sqlite > /tmp/graph.json
```

**Upstream source-tree caveat:** zlib 1.3.1's CMake build renames its tracked
`zconf.h` while preparing generated headers. The CI acceptance test clones
the pinned checkout into a **separate throwaway source tree for each
configuration**. Excession authenticates the selected source's Git blob and
hashes local generated dependencies; it does not assert that the entire
compiler-time working tree remained clean.

## Reused authorities and evidence boundaries

- **Git:** validates every selected tracked source blob against the pinned
  commit and tree. Only source input blobs are Git-authenticated.
- **CMake File API:** validates codemodel v2 build/source paths; imports
  configured target dependencies, source membership and expected artifacts.
  `CMAKE_CONFIGURED` never means a target actually built successfully.
- **Clang receipts:** verifies schema, canonical report SHA-256, source
  commit/tree/blob and the full compilation database SHA-256. Re-hashes
  the selected Clang executable, optional configuration file and observed
  dependency files on import, refusing stale evidence.
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

`.github/workflows/configured-graph.yml` fetches pinned zlib 1.3.1,
builds its `example` executable and linked `zlib` library **for real** in
separate Debug and Release source/build directories, and uses CMake's original
compilation database. It selects the exact `example` compile action,
assembles the graph twice to test stable identity, checks direct CMake
`example → zlib` target linkage and source/artifact paths against Ninja's
`-t graph`, verifies SQLite rows and distinct configurations, and ensures
missing or tampered receipts cannot masquerade as complete.

This proves configured build structure and local compiler observation
interoperability for the selected target. It is **not** independent
compiler-level symbol resolution, dynamic reachability, or proof of the
build's operational correctness.
