# Clang-backed translation-unit observations

Excession reuses **Clang**, **JSON Compilation Database** and the Linux
kernel's existing `scripts/clang-tools/gen_compile_commands.py`, rather than
implementing a competing C parser. Its earlier `kernel scan` stays
configuration-free; `kernel clang` is an explicitly trusted local operation
over one compiler-configuration-specific translation unit.

## Prepare a real Linux compilation database

Build Linux in an isolated environment with the intended `.config`, arch,
compiler, generated headers and object directory. Then use the upstream tool
already contained in that *same* pinned Linux Git checkout:

```sh
# Representative example, for a previously configured and built Linux tree:
python3 scripts/clang-tools/gen_compile_commands.py \
    -d /path/to/kernel-build -o /path/to/kernel-build/compile_commands.json
```

This tool reads the kernel's actual `.cmd` files. The directory must contain
the necessary build results; the generator cannot invent a kernel build from a
Git source checkout. No third-party parser or vendored copy is needed.

For a particular tracked C source whose checkout bytes match the requested
Git commit:

```sh
excession kernel clang \
    --repo /path/to/linux \
    --rev HEAD \
    --source drivers/rtc/rtc-cmos.c \
    --compile-db /path/to/kernel-build/compile_commands.json \
    --config /path/to/kernel-build/.config \
    --compiler clang \
    --trusted-local true > /tmp/rtc-cmos-clang.json
```

The original compilation entry may name GCC. Excession records the original
executable, but uses the **selected Clang binary** to interpret the flags.
Some GCC-specific options will be unsupported and will produce a refusal
rather than an invented AST. Multiple compilation-database records matching
one source are refused; this adapter intentionally avoids guessing which
configuration is authoritative.

## What is observed

- Git source commit, Git tree, exact source blob and source path, verified
  against the physically checked-out C file
- Compilation database path and SHA-256, selected command entry hash, original
  compiler identity and effective Clang flags
- Actual Clang binary path, SHA-256 and version
- Content-hashed preprocessor dependencies observed via `clang -M`, including
  tracked source, generated headers and system headers
- Source-file declarations and direct function calls from
  `clang -Xclang -ast-dump=json -fsyntax-only`
- Optional `.config` content hash and compiler diagnostics

The report has `schema=excession.clang-tu.v1`, a canonical `report_id`, and
`status=CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED`. It does not claim compilation
or execution succeeded, that all builds contain a symbol, or that a function
is reachable. Header contents are hashed to capture concrete local
dependencies, but external/generated files are **not** authenticated by the
Linux Git tree and can change independently. Compilation database and
Clang binaries are also local execution inputs. The source Git commit alone
cannot replay a build.

The source-only Kbuild and Kconfig scanner remains independent. This command
does **not** select a Kconfig configuration or expand Make functions. It
reparses a single existing compilation command. The byte and flag provenance
is what permits later comparison against a distinct configuration.

## Security and limits

`--trusted-local true` is mandatory. Compiling a C translation unit can
load generated headers and compiler extensions and must not be offered as an
isolation boundary. We never execute shell commands or the compiler-wrapper
binary specified in a compilation database. We strip ordinary compiler
output flags and refuse plugin flags, response files, and multi-source
commands; these checks are **not a sandbox**.

The adapter is bounded to one tracked C file (maximum 1 MiB), one unambiguous
compilation entry (database maximum 8 MiB), Clang outputs of 32 MiB AST and
2 MiB dependencies, at most 2,048 tracked dependency paths and 32 MiB of
dependency contents, at most 5,000 symbols/calls, and a 30-second timeout per
compiler invocation. Unsupported flags, missing generated headers, incomplete
or inconsistent command databases, and changing working-tree source bytes
cause an error.

## Verified Linux demonstration

The independent CI workflow `.github/workflows/clang-compile-db.yml`
parses Linux v6.12's real
`tools/build/feature/test-hello.c` with an explicitly constructed Clang
compilation entry. This particular test requires only the platform's C headers,
so it exercises exact Git source identity, Clang AST declarations/calls,
preprocessor header hashes, refusal on mutated source and deterministic
report replay *without* a full kernel build. **The test's constructed entry
is not a Linux Kbuild-generated compilation command**, and it is not evidence
that any kernel configuration was successfully built.

The test additionally feeds a **synthetic `.cmd` fixture** for the pinned
`scripts/mod/empty.c` through Linux's unmodified
`gen_compile_commands.py`, then verifies that the generated `command`-form
database is accepted by Excession and parsed by Clang. This is compatibility
testing, **not evidence of a real Kbuild compilation**.

For configuration-sensitive kernel C sources, use the upstream
`gen_compile_commands.py` on real build outputs as shown above.
