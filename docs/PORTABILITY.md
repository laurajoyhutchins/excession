# Portable Excession

Excession has one native Rust executable. Git remains required for
repository/evidence work, and SQLite is compiled into the executable through
the maintained rusqlite library. Python, Node, parsers, Go, CodeQL and Semgrep
remain optional and are not needed for `excession --help` or `excession doctor`.

## Distribution targets

The Portable CLI workflow builds and tests a release archive on:

| Runner | Native binary | Requirements |
| --- | --- | --- |
| Ubuntu 24.04 x86-64 | `excession` | Git and compatible glibc |
| Ubuntu 24.04 ARM64 | `excession` | Git and compatible glibc |
| macOS 15 Apple Silicon | `excession` | Git |
| macOS 15 Intel | `excession` | Git |
| Windows 2025 x86-64/MSVC | `excession.exe` | Git |
| Linux x86-64 musl | `excession` | Git for repository work; binary tested in network-isolated Alpine |

Archives are named using the actual Rust host target triple and include a
SHA-256 checksum file. The workflow builds with the pinned Rust toolchain and
uses the locked TypeScript 5.8.3 parser. It publishes archives on matching
version tags; pull requests retain CI artifacts.

Each archive contains the compiled binary, `lib/typescript.js` with its
upstream license, Excession's license, README, portability instructions, and
third-party notices. The JavaScript parser is selected by an explicit path,
**never by searching the target repository's node_modules**.

Archives do not bundle Git, Python, Node, Go, optional native scanners or
Tree-sitter grammars. SQLite is compiled in on all supported platforms.
The conventional Linux binaries still require a compatible libc. The musl
binary is validated inside a minimal container with networking disabled.
Portability is not the same as hermetic execution or a sandbox.

## Install

Extract the archive intact into a directory of your choosing. The executable
is at `excession/excession` (Unix) or `excession/excession.exe` (Windows).
Keep `lib/typescript.js` in the same `excession` directory as the binary's
`lib` subdirectory. Move that directory together if relocating the executable.

Verify the archive with its adjacent `.sha256` manifest before extracting.
A checksum detects corruption but is not an authenticated signature.

Check local availability (never downloads anything):

```sh
excession doctor
excession doctor --json
```

From any working directory:

```sh
excession --version
excession --help
excession index --repo /path/to/checkout --db /path/to/index.sqlite \
  --name project --rev HEAD --roots src,tests
```

The checkout must use the currently supported SHA-1 Git object format;
Git SHA-256 repositories are not yet supported. Committed-revision indexing
reads immutable Git objects and can run in a dirty checkout, but it does not
include staged, unstaged or untracked edits. Trusted-local compilers and
scanners retain their separate clean-checkout and isolation policies.

### Optional interpreters and adapters

- **Python:** Install Python 3.13 for Python indexing and the optional
  Tree-sitter bridge. The default executable is `python3` on Unix and
  `python` on Windows. `EXCESSION_PYTHON` may be set to an executable path,
  not a shell command.
- **JavaScript/TypeScript:** Install Node 22. The portable bundle carries
  a TypeScript 5.8.3 parser. `EXCESSION_NODE` may override the executable path.
- **Development/Cargo installation:** Rust alone builds the CLI. JS indexing
  additionally needs the adjacent parser sidecar, the locally locked
  `node_modules/typescript/lib/typescript.js` from the Excession checkout,
  or `EXCESSION_TYPESCRIPT_JS` naming an existing absolute, trusted
  TypeScript 5.8.3 JavaScript parser file.
- **Further analysis:** Go, Tree-sitter grammars, CodeQL and Semgrep require
  their native tools and separate trust/isolation decisions.

The doctor command uses bounded, two-second version probes and reports
detected tools, the bundled SQLite version, and available language adapters.
Its `excession.capabilities.v1` JSON output explicitly identifies unknown
grammar coverage and project-specific configuration requirements. Detection
is not proof of correctness, runtime isolation, or complete input discovery.

A missing parser is an error, not a silent fallback to less precise results.

## Evidence for portability

Ordinary pull requests exercise a Linux release smoke test. Pull requests
labeled `ci:full-portability` run the complete native OS matrix and musl/Alpine
validation. Version tags and manually dispatched release workflows also run
the complete matrix, preserving the fast default for unrelated changes.

The Portable CLI workflow checks SHA-1/SHA-256 standard vectors, runs a
Rust integration test from paths containing spaces and Unicode, builds the
release for Linux x86-64/ARM64, macOS Intel/Apple Silicon and Windows, then
extracts each archive *outside the build checkout*. The extracted binary runs a two-revision JavaScript fixture through
`analyze`, `impact`, `explain` and a replay audit using its bundled TypeScript
parser, then refuses a mutated report. This exercises Git access, compiled-in
SQLite and an actual static dependency; it does not establish completeness.
An additional musl binary is tested in network-isolated Alpine.

The existing Linux verify workflow remains authoritative for full Rust,
Python, JavaScript and historical suites. A green portability build proves
that the tested release subset operates on its host OS. It does **not**
establish every optional adapter's semantic parity or execute untrusted
programs safely.

## Limits

- The conventional Linux binaries require a compatible libc. The separate
  musl build addresses minimal x86-64 Linux environments. Non-x86-64 musl
  targets are not yet covered.
- Some optional Go effect-inference and native research adapters retain
  separately managed toolchain and runtime requirements.
- Windows has no POSIX file executable modes. Git object/mode identity is
  still authoritative. Materialized views use the host's read-only file flag
  and are not executable source checkouts.
- Paths that cannot be represented on a target filesystem may be refused
  during materialization; a Git view can remain auditable without copying it.

## Evidence and compatibility

The SQL schema and public persisted evidence format are unchanged by the
migration to rusqlite. Git SHA-1 remains necessary for reading existing SHA-1
repositories; the implementation now uses maintained RustCrypto crates rather
than handwritten hash algorithms. SHA-1 is not a secure collision-resistant
evidence signature and is never promoted to one by this change. SHA-256
continues to bind the unchanged canonical evidence bytes.
