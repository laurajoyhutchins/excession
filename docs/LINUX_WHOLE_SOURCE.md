# Linux-wide source inventory and partitioned syntax coverage

Excession can index the **entire Linux Git tree** and now account for all
tracked regular `.c`, `.h`, `Makefile`, `Kbuild`, and `Kconfig*` files
without making one enormous compiler/parser invocation.

This does **not** imply a complete configured Linux dependency graph:
Kconfig expressions and GNU Make functions are not evaluated, candidate
cross-partition Kconfig relationships may remain unresolved, generated
headers are not present in a source-only inventory, and no Linux build is
performed by the scanner.

## Verified upstream source

- Upstream tag: Linux `v6.12`, commit
  `adc218676eef25575469234709c2d87185ca223a`.
- Independent Git source inventory: **64,927** eligible regular source
  files, including C and header files, Kbuild and Kconfig syntax.
- At a partition size of 32: **2,029** source partitions.
- SHA-256 of the ordered (path, mode, Git blob ID) sequence:
  `a00cf23ad92ae65917f9fa085fc56aba4ff5da0c6af5a13302bc6621ad4f1394`.
- The CI smoke test parses and authenticates a real 32-file partition
  containing `drivers/rtc/rtc-cmos.c`, not all 2,029 partitions.
  The first independent source inventory run is at
  [GitHub Actions](https://github.com/laurajoyhutchins/excession/actions/runs/43042066279).

The full sweep workflow is independent and may be much more expensive than
the normal Git-tree index, which requires no source blob hydration.

## First inspect the entire source inventory

```sh
excession kernel inventory --repo /path/to/linux --rev HEAD --size 128
```

The inventory works with a sparse checkout if the Git tree is present. It
records the exact commit/tree, eligible count, selected partition size, count,
and an inventory digest. **It does not read C source blobs or parse them.**

## Parse and verify all sources

```sh
OUT=/tmp/excession-kernel-parts
mkdir -p "$OUT"
python3 tests/linux_partition_sweep.py \
  --repo /path/to/linux \
  --binary /path/to/excession \
  --rev HEAD --size 128 --out "$OUT"
```

The script only orchestrates the Rust CLI: every
`excession kernel partition --repo ... --index N --size 128` call reads
Git-authenticated source bytes and invokes the existing bounded syntax
adapter. Every partition receipt records the original Git inventory digest,
selected source count, exact observed source blobs, parser facts, and
`unsupported` records for oversized, non-UTF8, or batch-budget-exceeding
sources. It does not count unsupported blobs as parsed.

Receipts are checkpointed as `part-00000.json`, etc. Re-running the
orchestrator refuses an incompatible inventory and skips existing receipts
only when their basic scope matches. Existing records are **not trusted
solely because they exist**: after all parts are present, the native
`kernel audit` command validates each canonical receipt digest and its
source/blob identities against the independently reconstructed Git inventory.

You can run the strict native audit independently:

```sh
excession kernel audit \
  --repo /path/to/linux --rev HEAD \
  --dir /tmp/excession-kernel-parts --size 128
```

If all selected source blobs are parsed, the status is
`ALL_ELIGIBLE_SYNTAX_OBSERVED`. If some are skipped for explicit unsupported
reasons, it is `ALL_ELIGIBLE_SOURCES_ACCOUNTED_PARTIAL_SYNTAX`. Either
status requires **every** partition receipt; otherwise audit refuses.

A bounded pilot uses `--max-partitions 8` with the Python orchestrator.
It produces an explicit `INCOMPLETE_PARTITIONS_UNAUDITED` summary rather
than a complete-coverage claim. Partitions are deterministic but depend on
the exact Git tree and partition size, so a changed commit creates a new
inventory, never silently reusing old receipts.

## Boundaries and further work

- Up to **128 Git source files per partition**, **1 MiB per source**, and
  **32 MiB** aggregate parsed bytes per partition. Individual unsupported
  blobs are included in coverage accounting with a reason.
- `kernel scan --roots ...` remains available for the older
  single-subtree experience, with its original source/fact semantics.
- Partition-local candidate edges are not globally resolved symbol
  references. A whole-source audit does **not** prove compile inclusion.
- A complete **specified** kernel build needs an actual Kbuild
  `.config`, source/generated-header receipts, compiler command records,
  link artifacts and configured target closure. The CMake File API assembler
  is not a drop-in replacement for Kbuild.
- We should compare the configured graph against Kbuild's native records
  independently before claiming its transitive dependency coverage.

The full-sweep GitHub workflow is
`.github/workflows/linux-full-sweep.yml`. Only compact summary JSON is
uploaded by default; retaining all source receipts is an explicit local or
artifact-storage decision.
