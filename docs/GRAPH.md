# The Excession graph

The graph is an **evidence model**, not a CMake graph, a Kbuild graph,
or a C/C++ call graph. The same commands inspect observations produced by
any supported adapter. Adding a language/build system does not introduce a
new graph query API or a second database.

## Common workflow

Point Excession at an existing configured build. A SQLite database is
created at `BUILD/excession.sqlite` unless `--db` is specified.

```sh
# Build with CMake/Ninja first; request the CMake File API codemodel and
# enable compile_commands.json. No Clang receipts are required to start.
excession graph import --repo . --build build --target fmt

excession graph ls --build build
excession graph show --build build
excession graph deps --build build --node fmt
excession graph path --build build --from fmt --to src/format.cc

# A separate read of an existing graph does not execute any build tools.
excession graph deps --db /tmp/shared.sqlite --id GRAPH_ID --node fmt \
  --direction both --depth 3
```

`graph import` detects a CMake File API directory and uses its configured
target model. To add C/C++ AST observations, pass
`--observations /path/to/clang-receipts`. Without receipts, the graph
**still imports configured sources and targets**, but reports
`PARTIAL_TARGET_OBSERVATIONS`. No compilation is executed by the import.

For Linux Kbuild:

```sh
# Build init/built-in.a first, in a selected .config.
excession graph import --repo /path/to/linux --build /tmp/linux-build \
  --target init/built-in.a --trusted-local true

excession graph ls --build /tmp/linux-build
excession graph deps --build /tmp/linux-build --node init/built-in.a
excession graph path --build /tmp/linux-build \
  --from init/built-in.a --to init/main.c
```

The adapter detects a Kbuild `.config` and `.a` target. Native archive and
command-record extraction **executes trusted local tooling** and imports
Python code from the selected Linux source tree; it therefore requires
`--trusted-local true`. Use `--provider cmake|kbuild` if auto-detection is
ambiguous. Explicit `--db` shares multiple builds or configurations in one
database. When more than one graph matches, specify `--id GRAPH_ID` or
`--target TARGET` on query commands. Abbreviated graph IDs and unique
node suffixes are accepted; ambiguous matches are refused, never guessed.

The former `graph assemble` and `graph kbuild` commands remain compatible
provider-specific entry points, but are not the recommended interface.

## Neutral semantics

Every node has a common `role`, plus its source-native `kind`, stable
`id`, and payload. Roles include `target`, `artifact`, `source`,
`configuration`, `action`, `analysis`, `input`, `symbol`,
`reference`, and `observation`. Unknown provider kinds become
`role: unknown`, never a guessed category.

Each edge has a common `relation`, such as `depends_on`, `uses`,
`produces`, `contains`, `reads`, `declares`, `analyzed_as`,
`has_evidence`, or `may_refer_to`. It also preserves the exact original
`kind`, `status`, and `evidence` reference. No language-specific symbols
need to be interpreted by the query machinery.

| Original evidence | Neutral presentation | What can actually be claimed |
|---|---|---|
| CMake `TARGET_SOURCE` | `uses` | Configured target source membership |
| Clang `PREPROCESSOR_INPUT` | `reads` | Observed input bytes under one compiler action |
| Kbuild `CONTAINS_OBJECT` | `contains` | Archive membership from native archiver |
| Kbuild `COMPILES_SOURCE` | `reads` | Saved compiler command names that source |
| Clang `MAY_REFER_TO` | `may_refer_to` | Candidate name match, **not** resolved binding |

The storage tables remain `configured_graph`, `configured_entity`,
`configured_edge` and `configured_observation`; no parallel graph
database is introduced. Existing persisted snapshots missing neutral
`role`/`relation` fields are projected into the common vocabulary during
queries. Their original JSON bytes and SHA-256 graph IDs remain unchanged.

`graph deps` traverses **recorded relationships**, not dynamic call
reachability. It defaults to two outgoing hops and can traverse incoming
or both directions. Depth is limited to eight, and a bounded result reports
`PARTIAL_EDGE_LIMIT` rather than pretending completeness. `graph path`
returns a shortest recorded directed path and the evidence on each edge.
`NO_RECORDED_PATH_IN_SELECTED_DIRECTION_AND_DEPTH` is **not** proof that
a dependency does not exist.

All commands write machine-readable JSON; errors and ambiguous selection
go to stderr. This graph API deliberately has no source-language setting.
Support for another build provider requires a producer of authenticated
observations, not another query engine.

## Current limits

CMake's target adapter supports existing File API codemodel v2 and optional
Clang receipts. Kbuild's current adapter supports one locally built `.a`
archive with its direct object and `.cmd` observations. The neutral graph
does not magically add full transitive `vmlinux` closure, SCIP identity
resolution, all Kconfig variants, or runtime dependencies.

This PR does not modify the separate in-progress clap CLI migration. Once
the command grammar PR lands, the five neutral graph verbs should be
registered in the existing clap tree, without rebuilding a second parser.
