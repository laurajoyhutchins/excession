# Native compiler and indexer observations

Excession delegates language semantics to existing native tooling. The common
`excession analyze native` command binds a requested source to a Git commit,
checks tool-specific configuration, and emits
`excession.native-analysis.v1`. It does **not** translate each language's
observations into an invented universal call graph.

| Language | Native tool | What is observed | What is not established |
| --- | --- | --- | --- |
| Go | `go list` | Compiler-selected source and configuration inputs | Symbol calls or complete runtime behavior |
| C / C++ | Clang with `compile_commands.json` | One configured translation unit: AST, direct calls, headers | Successful link, all build configurations or cross-TU call graph |
| Java | JDK `javac` APIs | Compiler-resolved references within a selected tracked source set | All dependency configurations, generated classes or runtime dispatch |
| Rust | `rust-analyzer scip` and `scip print` | Source-bound workspace symbol references | A `rustc` build, typecheck or complete reference closure |

Python's AST and TypeScript language-service module resolution are used by
the ordinary static source index. They do not become compiler-backed build
evidence merely because the repository contains those languages. Use
`excession analyze languages --json` to inventory detected languages,
parsing, and conservative resolution coverage.

## Invoking native producers

```sh
excession analyze native --language go --source cmd/service/main.go --trusted-local --json
excession analyze native --language c --source src/main.c --compile-db /tmp/compile_commands.json --trusted-local --json
excession analyze native --language cpp --source src/main.cpp --compile-db /tmp/compile_commands.json --trusted-local --json
excession analyze native --language java --source src/main/java/pkg/Service.java --source-prefix src/main/java --trusted-local --json
excession analyze native --language rust --source src/lib.rs --rust-analyzer rust-analyzer --scip scip --trusted-local --json
```

The requested Rust file is pinned as a Git subject. Its nested SCIP report
covers a workspace, potentially with more files than the subject. An index
does not prove that the requested file was indexed. Rust's pre-existing
`excession analyze rust` command still emits its native SCIP format directly
for consumers that require it.

Go and Java may have documented restrictions on workspace layout and external
inputs. Clang requires an existing compilation database, and its external
headers/configuration are not all Git-authenticated. No command executes
untrusted compiler flags inside an isolation boundary. `--trusted-local` is
required, and ordinary `excession analyze` does not launch any of these tools.

## Shared evidence replay

```sh
excession analyze native --language rust --source src/lib.rs --trusted-local --json > /tmp/native-rust.json
excession evidence native-import --repo . --rev HEAD --report /tmp/native-rust.json --db /tmp/excession-evidence.db
```

Import replays the producer receipt against the pinned Git tree. Rust SCIP
source-reference endpoints must match exact Git blobs. A rehashed forged
endpoint is refused. The same storage path already supports Go input
membership, Java compiler-resolved references, and Clang translation-unit
observations without equating these predicates. Querying an absent reference
is not evidence that the reference does not exist.

The native report and the persisted receipt retain their producer-specific
meaning, limits, and `TRUSTED_LOCAL_NOT_ATTESTED` execution status. Artifact
build provenance and REA obligations remain separate. A future independent
compiler/test execution receipt must be recorded independently rather than
fabricated from static indexing.

The common entry point does not currently run `rustc`, `tsc`, the Python
type checker, or the Go linker. Extending the execution matrix requires exact
project configurations, versioned toolchain identities, bounded execution,
independent acceptance fixtures, and explicit coverage accounting.
