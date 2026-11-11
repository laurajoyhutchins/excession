# Kbuild archive graph (configured local observations)

Excession consumes **real** Linux Kbuild archive members and saved compiler
records. It does not reimplement GNU Make, decide which CONFIG guards hold,
or infer Kbuild's object selection from lexically parsed Makefiles.

## Reused native authorities

- Linux's own `scripts/clang-tools/gen_compile_commands.py` supplies
  `.cmd` parsing and C/assembly source extraction, imported from the selected
  Git checkout. The script's bytes must match the requested Git commit.
- `ar t` or `llvm-ar t` supplies the archive's actual member listing.
- Rust checks the Git commit/tree and selected source blob identities; SHA-256
  binds `.config`, archive bytes, object bytes, `.cmd` files, generated
  sources and extracted compiler command identities.
- Excession's existing configured-graph SQLite schema stores the native
  export receipt, entity records and typed evidence edges. Both the CMake
  and Kbuild adapters use the same SQLite persistence routine.

## Reproduction with Linux v6.12

```sh
LINUX=/path/to/linux
BUILD=/tmp/linux-build
# Repository must be an ordinary Git checkout. Use a pinned commit or tag.
make -C "$LINUX" O="$BUILD" x86_64_defconfig
make -C "$LINUX" O="$BUILD" -j2 init/built-in.a

excession graph kbuild \
  --repo "$LINUX" --rev HEAD \
  --build "$BUILD" --archive init/built-in.a \
  --ar ar --trusted-local true \
  --db /tmp/excession-configured.sqlite > /tmp/init-graph.json
```

Select `--ar llvm-ar` when the archive was constructed with the LLVM
toolchain. The importer never executes the saved compiler command or runs
Make. It only reads the selected archive with native archiver tooling and
loads the pinned Linux `.cmd` parser. This **does execute local Python
source from the selected Linux checkout**: `--trusted-local true` is
mandatory; it is not an isolation sandbox.

## Observations and limits

The v1 graph contains `CONFIGURATION_SNAPSHOT`,
`CONTAINS_OBJECT`, `HAS_BUILD_RECORD`, `COMPILES_SOURCE`, and
`RECORDS_COMPILER_COMMAND` relationships. Edge statuses distinguish
Git-authenticated sources, locally hashed generated files and compiler
commands extracted from saved records.

Unsupported assembler actions, absent `.cmd` files and source paths
outside the selected Git checkout/build directory remain explicit
`unresolved_objects`, never silently converted into compiled-source
facts. Even `ALL_ARCHIVE_MEMBERS_HAVE_COMPILER_RECORDS` asserts only
record coverage for **one archive**, not a complete configured kernel.

Limits: 10,000 members per archive, 256 MiB per hashed artifact, 2 MiB per
individual command record, 24 MiB native exporter JSON. Larger scopes must
be partitioned into separate archives.

The `.config` is hashed as a **local observed configuration snapshot**.
Because saved archive and compile records are not a trusted build
attestation, merely finding a configuration file does not independently
prove it produced those objects. For the same reason, observed archive
membership does not establish a successful final kernel link, correct
behavior, or runtime dependencies.

## Acceptance

`.github/workflows/kbuild-native-graph.yml` checks out Linux v6.12 at
`adc218676eef25575469234709c2d87185ca223a`, configures
`x86_64_defconfig`, builds `init/built-in.a`, compares object membership
against a separate `ar t`, and validates the graph/receipt SQLite rows,
determinism, source-Git mutation rejection and trusted-local opt-in.

### Next: configured vmlinux and module closure

Extend the same native producer to process `vmlinux.a`,
`modules.order` and per-module `.mod` membership, keeping only actual
configured build records. Import `clang-scan-deps` or compiler-index
references where applicable instead of guessing cross-translation-unit
symbol bindings. Compare archives and final link outputs independently
before claiming whole-build target closure.
