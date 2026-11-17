> **Reference for implemented commands.** The product is
> [investigation-first understanding and transformation](PRODUCT.md).
> This reference documents source, evidence and edit-building operations
> that support those workflows, not a separate graph-query product.
> There is no shipped `excession investigate` or `excession transform`
> command yet. See [current practical workflow](CODEBASE_UNDERSTANDING.md).

# Excession CLI

`excession` is the Rust command-line interface for Git-grounded code
intelligence. It indexes relationships at a pinned revision, assembles source
views, inspects evidence from other analysis tools, and audits recorded results
against their stated inputs.

One `clap` command tree defines named options and nested help. The opening
screen shows everyday commands; `excession commands` lists every specialist
family. Specialist commands remain callable even when omitted from the
opening help. There is no second positional dispatcher.

The command families correspond to different evidence layers:

| Work | Commands | Output |
| --- | --- | --- |
| Everyday source change impact | `discover`, `analyze`, `changes`, `impact`, `explain` | Git-pinned source witnesses, evidence and explicit unknowns |
| Static repository relationships | `index`, `inspect dependencies`, `view deps` | Source-linked module relations and navigable views |
| Syntax and native inputs | `syntax`, `semantic`, `inspect evidence` | Source-bound candidates and separately typed native observations |
| Refactoring investigation | `refactor suggest`, `refactor scip-suggest`, audit variants | Git-bound hypotheses, not source edits |
| Security evidence | `security ingest`, `security audit`, `view security` | Imported scanner results with source identities |

A successful audit establishes the reproducibility specified by its contract,
not the correctness or completeness of an external analyzer. The
[architecture](ARCHITECTURE.md) explains these boundaries.

## Install

Build from the repository with Rust 1.99.1, Cargo, Git (SHA-1 object
repositories), and an available dependency cache for offline installation.
Release archives bundle SQLite and do not require separately installed SQLite
or OpenSSL libraries; see [portability](PORTABILITY.md).

```sh
# From the repository root; one binary is installed.
cargo install --offline --locked --path core --bin excession
excession --version
excession --help
```

For development, `make cli` produces `core/target/debug/excession`. `make check` runs the existing source/evidence tests and the dedicated CLI tests.

See [resource budgets](RESOURCE_BUDGETS.md) for the configurable agent-locate
indexing policy, the distinction between processing and completeness, and
remaining pagination and streaming limitations.

## Ordinary repository analysis

Use a checkout containing a supported Python or JavaScript/TypeScript source
layout. Git reads are pinned to committed objects. The source checkout can be
dirty for these read-only operations; uncommitted edits are **not included**.

```sh
cd /path/to/repository
excession discover .
excession analyze .
excession analyze languages
excession analyze languages --roots src,tests --json
excession changes .
excession impact . --output /tmp/impact.json
excession impact . --json

excession explain . --report /tmp/impact.json --source src/pkg/handler.py --json
excession explain . --report /tmp/impact.json --source src/pkg/handler.py \
  --source-commit PINNED_COMMIT --audit
excession explain . --report /tmp/impact.json --all-sources --json
```

`impact` produces one `excession.impact.v3` report with a
`candidate_source_impacts` inventory. Each observed dependent source has
one rank-certified `next_hop` toward a changed source, preserving its
original Git identity and producer receipt. Alternative paths are not
enumerated. The complete report is saved to a new absolute path outside
the checkout; existing files are never overwritten. `--json` writes the
same canonical report to stdout.

The source path must be relative to the repository. If the same dependent
source appears in both revisions, specify `--source-commit`; otherwise
explanation refuses ambiguity. `--all-sources` replays the source once
and reconstructs at most 128 witnesses. The batch refuses a larger
result rather than truncating it.

Every result retains explicit partial coverage, and an empty source
inventory does not prove that tests are safe to skip. The source witness
forest is documented in [source impact](COMPACT_IMPACT.md).

The explanation command re-runs its source-bound impact query against pinned Git
objects and refuses mutated or stale reports. The audit reports Git
authentication and replay separately from semantic correctness, which remains
`NOT_EVALUATED`. The CLI uses no ontology inference to manufacture
dependency observations. The experimental concept stack may later provide
separately typed annotations.

Without `--base`, `impact` and `changes` compare `HEAD~1..HEAD`. For a
root commit, explicitly choose a valid base after another commit exists.

`excession analyze languages` performs a common source-bound Tree-sitter
scan for all selected recognized languages, followed by only the conservative
import resolvers Excession actually supports (Python, JavaScript/TypeScript,
Go). It reports parsed, unavailable and unresolved counts separately for
every language. Rust, C/C++, Java, and other languages remain syntax-only
unless their separate native tooling is invoked. Scans require a clean checkout,
work offline with installed grammars, default to `--roots .`, and refuse more
than 500 selected source files or 16 MiB of source: narrow `--roots` for
larger projects. `--json` emits `excession.language-analysis.v1` with
Git-bound candidates and explicit skipped sources. Without `--json`, the
default is a concise per-language summary. These candidates are **not**
proven build dependencies or validated program effects.

### Configured native Go, Java, and Clang evidence

```sh
excession analyze native --language go --source cmd/server/main.go --trusted-local
excession analyze native --language go --source cmd/server/main.go --trusted-local --json
excession analyze native --language java --source src/main/java/app/Service.java \\
  --source-prefix src/main/java --trusted-local --json
excession analyze native --language java --source src/main/java/app/Service.java \\
  --source-prefix src/main/java --classpath-manifest /tmp/jvm-inputs.json \\
  --trusted-local --json
excession analyze native --language c --source src/main.c \\
  --compile-db build/compile_commands.json --trusted-local --json
excession analyze native --language cpp --source src/main.cpp \\
  --compile-db build/compile_commands.json --trusted-local
```

The original producer reports can also be retained and composed without
re-executing their compiler:

```sh
excession analyze native --language go --source cmd/server/main.go \\
  --trusted-local --json > /tmp/go-native.json
excession evidence native-import --repo . --rev HEAD \\
  --report /tmp/go-native.json --db /tmp/excession.sqlite
excession analyze . --db /tmp/excession.sqlite --json
excession impact . --db /tmp/excession.sqlite --json
excession agent context --repo . --rev HEAD --focus cmd/server/main.go \\
  --db /tmp/excession.sqlite
```

Native import retains the **original report bytes** through the existing
immutable evidence store, rechecks all claimed source/configuration blobs
against pinned Git objects, and requires exact report identities. Automatic
analysis, impact, agent context, and agent verify reuse the same selected,
source-replayed receipts. They expose native Go input membership, Java compiler-resolved source
references and configured Clang AST observations as **configured observations**, not generic dependency
edges or verified runtime effects. Reports for different commits remain
separate; saved impact plans replay only the originally selected identities.
`evidence native-store-audit` rechecks an imported snapshot without executing
the compiler. Imported producer claims remain **unattested**, even when their
Git inputs are authentic.

Java uses the installed JDK's `JavacTask` parse/analyze API to resolve
references between **tracked Java source files**. An optional externally
supplied Maven/Gradle-claimed compilation-input receipt admits only
hash-checked JARs, exact Git source inventory and supported Java `--release`.
See [JVM resolved inputs](JVM_RESOLVED_INPUTS.md). It neither generates class
files nor runs project classes, annotation processors, Maven or Gradle. It
uses an empty classpath unless `--classpath-manifest` is supplied; it
refuses missing external dependencies in either mode.
The source set is limited to 128 Java files and 4 MiB of source. Select a
smaller module with `--source-prefix` when needed. Compiler-resolved
source-to-source references are kept as typed, unattested observations
rather than promoted to verified call edges. Its offline JDK configuration
is intentionally narrower than a full Java project build.

Go reuses the existing pinned-Git, offline `go list` producer with its
declared source/configuration inputs. Clang reuses the existing compilation
database AST adapter for one translation unit, with symbols, calls and
content-hashed headers. Both produce `excession.native-analysis.v1`,
including the selected Git commit/tree and source blob, producer evidence,
input inventory, an explicit partial-coverage status, and a consistent
human summary. Go's declared input membership is **not a symbol call graph**.
Clang's **tracked** header observations are admitted only when the
reported SHA-256 matches the historical Git blob. They appear as
configuration-scoped observations for impact and agent context, never as
invented direct-call edges. External/generated headers and the compilation
database remain **not independently Git-authenticated**.
Results are trusted-local observations rather than independently attested
build receipts. No source code or build process is executed without
`--trusted-local`; this is not a sandbox.

Go currently requires a tracked root `go.mod` without a `go.work`, uses
offline module resolution and disables CGO. Clang requires an existing
compilation database and selected HEAD in a clean checkout. A source
without a supported native producer is refused explicitly.
Use `analyze languages` for syntax-level coverage across those languages.

`excession analyze rust --trusted-local` runs the installed rust-analyzer
and official SCIP decoder on a **clean** HEAD. It automatically selects a
sole tracked Cargo project, including a nested project. Multiple projects
require `--source-prefix`. Default output is a human summary; add `--json`
for the full versioned SCIP observations. `--trusted-local true` remains
accepted; the bare flag is preferred. The process is **not sandboxed**.

`analyze` auto-discovers supported source roots and creates its SQLite cache
outside the repository. Use `--roots` or `--db` when a project requires explicit
selection. `analyze clang` remains a distinct configured compiler workflow.

## One repository, many layouts

The following example uses an existing pinned h11 checkout and writes SQLite and manifests outside the repository. The indexer currently resolves static Python/JavaScript dependencies within configured roots. Use the actual root directories for your project.

```sh
REPO=/path/to/h11
OUT=/tmp/excession-h11
mkdir -p "$OUT"

excession index --repo "$REPO" --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --roots h11,tests

excession view deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._connection > "$OUT/connection.json"
excession view deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._headers > "$OUT/headers.json"

# Re-execute the original graph query against the saved manifest.
excession view replay deps --db "$OUT/index.sqlite" --name h11 \
  --rev HEAD --seed h11._connection --manifest "$OUT/connection.json"

# Mount separately to preserve both virtual focus groups.
excession view mount --repo "$REPO" --manifest "$OUT/connection.json" \
  --namespace connection > "$OUT/mounted-connection.json"
excession view mount --repo "$REPO" --manifest "$OUT/headers.json" \
  --namespace headers > "$OUT/mounted-headers.json"
excession view compose --repo "$REPO" --op union \
  --left "$OUT/mounted-connection.json" --right "$OUT/mounted-headers.json" \
  > "$OUT/combined.json"

# No source files are copied for any of these virtual operations.
excession view ls --repo "$REPO" --manifest "$OUT/combined.json"
excession browse --repo "$REPO" --manifest "$OUT/combined.json"
```

In the browser use `ls [directory]`, `cd directory`, `pwd`, `stat file`, `show file`, `help`, or `quit`. The browser reads Git blobs on demand and limits rendered text to 8192 bytes by default. Use `view cat` for exact raw bytes:

```sh
excession view stat --repo "$REPO" --manifest "$OUT/connection.json" \
  --path views/00-focus/h11/_connection.py
excession view cat --repo "$REPO" --manifest "$OUT/connection.json" \
  --path views/00-focus/h11/_connection.py
```

A virtual path must be obtained from the manifest or `view ls`. The code above is a representative h11 path, not a generic path for other projects.

## Optional physical materialization

```sh
excession view materialize --repo "$REPO" --manifest "$OUT/combined.json" \
  --output "$OUT/read-only-tree"
excession view audit --repo "$REPO" --manifest "$OUT/combined.json" \
  --output "$OUT/read-only-tree"
```

Materialization requires a new **absolute** destination outside the source checkout, produces mode-0444 regular files, and audits every file against its original Git blob. Materialized views are **not runnable checkouts** and cannot be used to claim test or security equivalence.

## Refactoring hypotheses

```sh
excession refactor suggest --db /tmp/index.sqlite --name project --rev HEAD > /tmp/refactor.json
excession refactor audit --db /tmp/index.sqlite --name project --rev HEAD --report /tmp/refactor.json
```

Both commands require an indexed Git revision. Results are unverified
refactoring hypotheses, not safe edits. The audit recomputes indexed relations
without rerunning the language parsers. See [refactoring hypotheses](REFACTORING.md).

## Scanner evidence

For an already installed Semgrep scanner and a reviewed, local ruleset, the CLI can execute the scanner without a shell, then import and independently replay the resulting evidence:

```sh
excession security scan --repo "$REPO" --report "$OUT/report.json" \
  --db "$OUT/security.sqlite" --engine semgrep \
  --config /path/to/reviewed-rules.yaml --trusted-local true
```

For CodeQL, use `--engine codeql --codeql-db /path/to/prebuilt/codeql-db` and a reviewed local query suite for `--config`. Use `--scanner /path/to/executable` when the engine binary is not on `PATH`. The default timeout is 300 seconds; `--timeout-seconds` supports 1 through 300. The source checkout must remain clean, the report must not already exist, and report/database files must be outside the checkout. Output paths are required to be absolute.

**This is trusted-local execution, not a sandbox.** In particular, CodeQL database creation can execute repository build steps; build databases in a separately reviewed, isolated process. Scanner rule packs and executables must also be trusted. The CLI neither installs tools nor downloads rules. Report provenance is not a signed attestation. A controlled fake-scanner integration test verifies orchestration; it does not establish that CodeQL or Semgrep is installed here.

After obtaining an actual scanner report through an authorized workflow:

```sh
excession security ingest --repo "$REPO" --report "$OUT/report.sarif" \
  --db "$OUT/security.sqlite" --engine codeql
excession security audit --repo "$REPO" --report "$OUT/report.sarif" \
  --db "$OUT/security.sqlite" --engine codeql

# The run ID comes from the ingestion receipt.
excession view security --db "$OUT/security.sqlite" --repo "$REPO" \
  --run YOUR_64_HEX_RUN_ID > "$OUT/security-view.json"
excession view replay security --db "$OUT/security.sqlite" --repo "$REPO" \
  --run YOUR_64_HEX_RUN_ID --manifest "$OUT/security-view.json"
```

The `tools/pipeline.py` scanner runner uses the dedicated CLI by default. CodeQL and Semgrep remain optional external programs. An imported report records **unverified static candidates**, not confirmed CVEs. Before trusting a security view, independently audit the underlying scanner ingestion as shown above.

## Agent-facing commands

`excession agent locate --repo REPO --task TEXT [--rev HEAD] [--limit 20] [--json]`
finds ranked lexical source candidates from pinned Git blobs using an
segmented, incremental, external SQLite FTS5 index (up to eight
segments refreshed by default, configurable via `--index-segments`), with exact
source identities, bounded excerpts, and an embedded agent context report.
Candidates are **unverified text matches** and uncommitted edits are excluded.
No model or external index service is required. See [agent locate](AGENT_CLI.md).


`excession agent context --repo REPO --rev COMMIT --focus PATH[,PATH...] [--task TEXT] [--limit 32]`
returns versioned JSON with exact source identities, existing candidate
relationships, coverage gaps, and explicitly heuristic test suggestions.

`excession agent verify --repo REPO --base COMMIT --candidate COMMIT
[--receipts FILE] [--limit 32]` recomputes supported Git-bound impact evidence
and reports outstanding obligations. It does not execute supplied commands,
trust unsigned execution receipts, or claim safety to merge. Both optionally
accept `--roots` and `--db` for the underlying analyzer. See
[the agent CLI contract](AGENT_CLI.md).

## Complete command families

- `excession discover|analyze|changes|impact|explain`: discover supported roots, index static candidates, compare exact Git revisions, traverse bounded candidate paths, and authenticate explanations. `explain --audit` does not independently prove semantic correctness.
- `excession index`: index an exact Git revision and configured dependency roots.
- `excession refactor suggest|audit`: emit or replay bounded source-bound refactoring hypotheses.
- `excession inspect dependencies|manifest|impact|review`: query indexed relations. Revision flags accept Git refs such as `HEAD`, resolved to commit IDs.
- `excession security scan|ingest|audit`: run an installed trusted scanner, or import and independently recheck CodeQL SARIF or Semgrep JSON.
- `excession view deps|security`: derive deterministic, source-bound views.
- `excession view compose|select|mount`: combine and filter read-only views.
- `excession view replay deps|security|compose|select|mount`: regenerate the result from the original inputs and compare with the saved manifest. For composition, selection and mounting the saved result is named by `--output`.
- `excession view ls|stat|cat`: list, inspect or read exact virtual source entries on demand.
- `excession view materialize|audit`: generate a read-only physical tree or verify it.
- `excession browse`: terminal browser, without a Python process or copied checkout.

All file contents and view identities remain grounded in Git objects. The CLI does not silently turn a candidate into a vulnerability, execute untrusted code, install scanners, or claim that unindexed edges do not exist. JSON commands write canonical JSON to standard output; `view cat` writes raw bytes. Errors and refusals go to standard error and return a nonzero exit code. The CLI uses one `clap` grammar with strict named options and refuses unknown or duplicate flags.

**Historical experiments:** The positional `excession research` command family
is retired. Reproduce an earlier protocol from its pinned historical Git
revision and environment; modern `view` and `inspect` commands are not assumed
to be byte-for-byte replacements. The source history, versioned on-disk
protocols, and exact-replay fixture identities remain retained. See
[migration notes](MIGRATION.md).
