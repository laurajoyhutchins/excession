# Generic compiler-backed C/C++ observations

Excession has one **repository-independent** Clang translation-unit interface.
It reuses Clang itself, the standard JSON compilation database format, Git
object identities, and the existing source-bound evidence bridge. It is not a
second C/C++ parser, a replacement for a language server, or a build system.

The `excession analyze clang` command works on clean or partially checked-out
Git repositories when the *selected source file* exactly matches a commit
blob and its locally available compilation dependencies can be parsed. It
does not assume the repository is Linux.

## Usage

```sh
excession analyze clang \
    --repo /path/to/zlib \
    --rev 51b7f2abdade71cd9bb0e7a373ef2610ec6f9daf \
    --source test/example.c \
    --compile-db /tmp/zlib/compile_commands.json \
    --compiler clang \
    --trusted-local true > /tmp/zlib-clang.json

excession analyze clang \
    --repo /path/to/fmt \
    --rev 123913715afeb8a437e6388b4473fcc4753e1c9a \
    --source test/static-export-test/main.cc \
    --compile-db /tmp/fmt/compile_commands.json \
    --compiler clang++ \
    --trusted-local true > /tmp/fmt-clang.json
```

Support: C `.c` and C++ `.cc`, `.cpp`, `.cxx`, and `.C`. The CLI chooses
`clang` for C or `clang++` for C++ by default. `--compiler` may select a
specific Clang binary; only that selected binary is executed. An arbitrary
original compiler, including a target-triple-prefixed GCC, is retained as
descriptive metadata from the compilation database, never executed.

`excession kernel clang` is retained as a compatibility alias using the
exact same implementation and report schema; Linux Kbuild is only **one**
producer of supported compilation database entries.

## Reuse build-system outputs

Standard compilation databases can come from CMake
(`-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`), Meson/Ninja, Bear, or other build
systems supporting the JSON compilation database format. Linux has its own
`scripts/clang-tools/gen_compile_commands.py` over genuine `.cmd` records.
These producers remain outside Excession. The adapter consumes their
already-created output; it neither runs the build nor generates one.

A database entry may use either `arguments` (preferred) or `command`
(parsed as POSIX shell words without executing a shell). Exactly one entry
may resolve to the selected source file. Multiple configuration variants are
ambiguous and cause a refusal; choose a specific database first.

## Observable contracts

- Git commit and tree IDs, plus the exact checked-out selected source Git
  blob, independently verified before invoking Clang
- `language`, the selected original compilation entry's SHA-256, the
  compilation database's full SHA-256, and effective Clang arguments
- Actual Clang executable file hash, path and version
- `clang -M` dependency file names and SHA-256 hashes for current headers,
  including non-Git external or generated dependencies
- `clang -Xclang -ast-dump=json -fsyntax-only` source-file declarations,
  qualified namespace/class names, function definitions and direct call
  nodes, extracted from the real AST
- Optional caller-provided configuration file hash, repeat-run consistency
  checks, and canonical report SHA-256

The common report schema remains `excession.clang-tu.v1` and
`status=CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED`. It is an observation
**of one translation unit under one selected local compilation command**.
It does not establish an actual linkable build, call-graph completeness,
runtime reachability, matching behavior, or CVE exploitability. C++ call
targets are name-level observations, not unique overload-resolved identities
or linker symbols.

Other language work remains complementary: Tree-sitter is suitable for
multi-language syntax discovery without a build; Go/Rust/JS/TS use native
resolution tools. They should share the **Git/source identity and bounded
evidence contract** rather than being forced through Clang.

## Trust, budgets and deterministic replay

`--trusted-local true` is mandatory. A compiler processing repository source
and headers is executable local work, **not an untrusted sandbox**. Excession
does not invoke the compiler executable named in the original database and
does not run commands through a shell. It removes output/depfile flags and
rejects compiler plugins, response files and multiple source inputs, but
neither Clang invocation nor its flags are an isolation guarantee.

One source may be at most 1 MiB. The database is capped at 32 MiB.
Clang JSON AST output is capped at 192 MiB, dependency output at 2 MiB,
dependency content at 32 MiB, and declarations/call facts at 5,000. There is
a 30-second Clang timeout per parse. Oversized/unsupported translation
units are explicitly **unsupported** rather than silently truncated.

The report hashes relevant local inputs and refuses if the selected source,
database, provided configuration, or observed dependencies change during the
two Clang invocations. Repeated reports from unchanged local state have the
same `report_id`. Generated/system headers are hashed but **not authenticated
by Git**; neither compiler/toolchain identity nor the report is a remote
attestation.

## Independent regression

`.github/workflows/compiler-cross-repo.yml` uses pinned upstream zlib
(C) and fmt (C++) Git commits with constructed, clearly marked compilation
database fixtures. It verifies source Git identities, declarations/calls,
header observations, trust refusal, reproducible reports, and the legacy
Linux command alias. These fixtures test **parser interoperability, not
whether zlib or fmt was built**. A separate Linux workflow tests its upstream
compiler-database generator's `command` entries.

Generic compiler-backed evidence is a distinct layer from Excession's
source-only Linux C/Kbuild/Kconfig relationships, which do not require a
compiler or local build configuration.
