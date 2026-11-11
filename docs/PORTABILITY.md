# Portable Excession

Excession has one native Rust executable. The core works independently of its
source checkout. Git and SQLite remain required for repository/evidence work.
Python, Node, parsers, Go, CodeQL, and Semgrep are optional capabilities, not
prerequisites for `excession --help` or commands that never call them.

## Distribution targets

The Portable CLI workflow builds and tests a release archive on:

| Runner | Binary | Native requirements |
| --- | --- | --- |
| Ubuntu 24.04, x86-64 | `excession` | Git, system libsqlite3, compatible libc |
| macOS 15, runner architecture | `excession` | Git, system SQLite |
| Windows 2025, x86-64/MSVC | `excession.exe` | Git; SQLite statically linked |

Archives are named using the actual Rust host target triple and include a
SHA-256 checksum file. The workflow builds with the pinned Rust toolchain and
uses the locked TypeScript 5.8.3 parser. It publishes archives on matching
version tags; pull requests retain CI artifacts.

Each archive contains the compiled binary, `lib/typescript.js` with its
upstream license, Excession's license, README, portability instructions, and
third-party notices. The JavaScript parser is selected by an explicit path,
**never by searching the target repository's node_modules**.

Archives do not bundle Git, Python, Node, Go, optional native scanners or
Tree-sitter grammars. Linux/macOS archives also rely on system libraries.
Portability is not the same as hermetic execution or a sandbox.

## Install

Extract the archive intact into a directory of your choosing. The executable
is at `excession/excession` (Unix) or `excession/excession.exe` (Windows).
Keep `lib/typescript.js` in the same `excession` directory as the binary's
`lib` subdirectory. Move that directory together if relocating the executable.

Verify the archive with its adjacent `.sha256` manifest before extracting.
A checksum detects corruption but is not an authenticated signature.

From any working directory:

```sh
excession --version
excession --help
excession index --repo /path/to/checkout --db /path/to/index.sqlite \
  --name project --rev HEAD --roots src,tests
```

The checkout must be a clean SHA-1 Git repository. Git SHA-256 object
repositories are not yet supported. The index executes locally installed
language parsers, not the target project's code.

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

A missing parser is an error, not a silent fallback to less precise results.

## Evidence for portability

The Portable CLI workflow checks SHA-1/SHA-256 standard vectors, runs a
Rust integration test from paths containing spaces and Unicode, builds the
release, then extracts the archive *outside the build checkout*. The extracted
binary indexes a Git fixture in JavaScript using its bundled TypeScript
parser, exercising Git access, SQLite linkage, and language adapter discovery.

The existing Linux verify workflow remains authoritative for full Rust,
Python, JavaScript and historical suites. A green portability build proves
that the tested release subset operates on its host OS. It does **not**
establish every optional adapter's semantic parity or execute untrusted
programs safely.

## Limits

- Linux and macOS dynamically link system SQLite and platform libc. Older
  or minimal systems may need compatible libraries or a local build.
- Some optional Go effect-inference and native research adapters retain
  separately managed toolchain and runtime requirements.
- Windows has no POSIX file executable modes. Git object/mode identity is
  still authoritative. Materialized views use the host's read-only file flag
  and are not executable source checkouts.
- Paths that cannot be represented on a target filesystem may be refused
  during materialization; a Git view can remain auditable without copying it.
