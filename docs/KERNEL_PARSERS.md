# Linux C, Kbuild and Kconfig syntax observations

Excession can extract conservative, read-only facts from pinned Linux kernel
Git blobs. This is useful for navigation and scope discovery, **not** a C
compiler, GNU Make evaluator, complete Kconfig implementation, or build proof.

## Run

Use a clean SHA-1 Git checkout (a sparse/partial clone is supported when the
selected blob objects can be fetched by Git). Scanned sources are read from
Git objects, not from the worktree.

```sh
excession kernel scan \
  --repo /path/to/linux \
  --rev adc218676eef25575469234709c2d87185ca223a \
  --roots drivers/rtc > /tmp/linux-rtc.json
```

The JSON report binds `source_commit`, `source_tree`, selected roots, every
source path/blob/mode, source-line observations, diagnostic records and a
canonical SHA-256 `report_id`. A single `kernel scan --roots` run is constrained to 1,500 regular
source files, 1 MiB per source, 32 MiB total source input, 100,000
observations, and 64 input files per parser invocation. For a whole Git
tree, use the [bounded source inventory and partition audit](LINUX_WHOLE_SOURCE.md)
rather than increasing these budgets. The tool never runs
a Kbuild recipe, loads a compiled kernel module, or compiles source.

## Syntax observations

- **C:** `#include` with literal quoted or angle-bracket operands;
  conservative raw `#if/#ifdef/#ifndef/#elif/#else` condition strings;
  macro declarations; lexical `EXPORT_SYMBOL*` candidates. Macro-generated
  includes are diagnostics, not resolved targets. C symbols and type
  relationships are not comprehensively parsed without a compilation
  database and a real native C parser.
- **Kbuild:** straightforward `obj-$(CONFIG_*)`, `foo-y`,
  `foo-$(CONFIG_*)` and related Make assignments, including continued lines
  and simple make condition stacks. GNU Make functions, variable expansion,
  recipes, `eval`, and build selection are not executed or assumed.
- **Kconfig:** `config` and `menuconfig` declarations, `depends on`,
  `select`, `imply`, `default`, `source` variants and lexical
  `if/endif` conditions. Help prose is excluded. This is not complete
  Kconfig grammar or tristate semantics and does not evaluate any option.

`facts[].status` is one of `DECLARED`, `LEXICAL_CANDIDATE`,
`UNRESOLVED_INCLUDE`, `UNRESOLVED_CONFIG`, `UNRESOLVED_SOURCE`, or
`UNRESOLVED_MAKE`. No fact is a verified compile/run claim.

## Git-identity-backed candidate edges

`links` contains only relationships supported by an exact filename,
repository path, or Kconfig symbol-name match in the pinned Git tree:

- `candidate-header-include`: a literal C include matched a tracked
  source path, using the including directory for quoted includes or the
  repository's `include/` tree for angle-bracket includes
- `candidate-object-source`: a Make object filename such as
  `rtc-cmos.o` matched a same-directory tracked `rtc-cmos.c`
- `candidate-config-declaration`: a `CONFIG_RTC_DRV_CMOS` guard
  matched an observed `config RTC_DRV_CMOS` declaration
- `candidate-kconfig-source`: a literal Kconfig source directive
  matched a tracked Git path

The statuses `GIT_PATH_MATCH_ONLY` and `CONFIG_NAME_MATCH_ONLY` are
intentionally weak. Header-search precedence, generated headers, module
composition, recursive source lists, Kconfig dependencies, architecture,
compiler macros and the selected `.config` could invalidate a proposed
operational edge. The absence of an edge says nothing about safety or
inclusion in a real build.

## Test and reproduce

```sh
python3 -m unittest discover -s tests -p test_kernel_parsers.py -v
# GitHub workflow .github/workflows/linux-native-parsers.yml runs a pinned
# v6.12 integration probe through tests/kernel_linux_probe.py.
```

The regression checks connect Linux RTC's `CONFIG_RTC_DRV_CMOS` definition,
its `rtc-cmos.o` Make assignment, `rtc-cmos.c` source and C header paths;
check exact Git blob IDs and deterministic report identity; and ensure the
records never claim that configuration-dependent facts were verified.

## Future complete build analysis

A later **independent** adapter should consume an explicitly pinned,
configuration-specific `compile_commands.json`, use Clang AST and
preprocessor dependency outputs, and bind its results to the corresponding
compiler version, flags, generated headers and built objects. That
configuration-scoped evidence must not silently replace these source-only
facts or be presented as evidence for other kernel configurations.
