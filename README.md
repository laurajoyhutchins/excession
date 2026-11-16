# Excession

**Git-grounded, auditable code intelligence.**

Excession turns a pinned Git revision into inspectable relationships between
source files, analysis results, and focused repository views. It records where
each observation came from and lets you query or replay the evidence against
the original Git objects.

The core is a Rust CLI with Git as source authority and SQLite for indexed
relations and evidence. It reuses language-native parsers, symbol indexers,
compilers, and scanners rather than replacing them.

## Get started

**Prebuilt releases:** When a published version is available, download its
platform archive and adjacent `.sha256` checksum from
[GitHub Releases](https://github.com/laurajoyhutchins/excession/releases).
Verify the checksum, extract the entire `excession/` directory, and keep
the executable next to `lib/typescript.js`. See the
[install, upgrade, and uninstall instructions](docs/RELEASE.md).
An unpublished crate version is not itself a release.

For Linux x86-64, after v0.2.0 is published:

```sh
VERSION=v0.2.0
TARGET=x86_64-unknown-linux-gnu
ARCHIVE="excession-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/laurajoyhutchins/excession/releases/download/${VERSION}"
curl -fLO "${URL}/${ARCHIVE}"
curl -fLO "${URL}/${ARCHIVE}.sha256"
sha256sum -c "${ARCHIVE}.sha256"
mkdir -p "$HOME/.local/opt"
tar -xzf "$ARCHIVE" -C "$HOME/.local/opt"
"$HOME/.local/opt/excession/excession" doctor
```

The checksum detects corruption, not publisher authenticity. Keep the
executable and bundled `lib/typescript.js` together. For other platforms,
select the matching archive from the release page and follow
[the platform instructions](docs/RELEASE.md).

Excession is a local Rust CLI. It reads pinned Git source objects, indexes
observed relationships, and explains *candidate* change impact with source
evidence. Git is required; Python and Node are needed for their respective
language adapters. Check the actual available tools first:

```sh
cargo install --locked --path core --bin excession
excession doctor
```

From a supported Python or JavaScript repository:

```sh
cd /path/to/repository
excession discover .
excession analyze .
excession impact . --output /tmp/excession-impact.json
```

The current `excession.impact.v3` report ranks **Git-bound source
witnesses**, not claim IDs or asserted path effects. `--output` saves the
exact replayable JSON outside the source checkout without overwriting an
existing file. Select a reported source path for evidence replay:

```sh
excession explain . --report /tmp/excession-impact.json \
  --source src/pkg/handler.py --json
excession explain . --report /tmp/excession-impact.json \
  --source src/pkg/handler.py --audit
```

Replace the example path with an actual `candidate_source_impacts` entry.
Explanations preserve partial coverage and do not prove behavioral impact.

The `--audit` result distinguishes source replay from independent semantic
validation. Source replay does **not** establish that an affected module is
broken or that a test can safely be skipped. Reports explicitly preserve
unsupported and unresolved coverage.

These workflows operate on committed revisions. They can read pinned Git
objects even when the working tree is dirty; they do not include uncommitted
edits. Default SQLite caches live outside the repository. Exact-output workflows
also support `--json` and explicit revisions/roots.

## Daily commands

```sh
excession                    # focused help
excession commands           # all specialist families
excession analyze .          # static Python/JS/TS candidates
excession analyze languages  # Git-bound syntax for all detected languages
excession impact .           # compare HEAD~1..HEAD
excession tests              # plan tests without running them
excession analyze rust --trusted-local    # execute installed native Rust indexer
excession analyze rust --trusted-local --json > /tmp/rust.json
```

The `analyze languages` command parses tracked sources using preinstalled
Tree-sitter grammars and reports each language's parsed, unavailable and
unresolved counts. Python, JavaScript/TypeScript and Go receive conservative
import candidates where supported. Rust and other languages retain syntax
observations without fabricated dependency edges. It requires a clean Git
checkout and accepts `--roots` to narrow the scan. Add `--json` for
Git-bound evidence. Missing grammars are reported rather than downloaded.

Rust analysis selects the sole tracked Cargo project, including a nested
project. For multiple projects, specify `--source-prefix`. It requires a
clean checkout and explicitly executes trusted local tooling; this is not a
sandbox. Specialist commands remain accessible through their own help screens.

## Plan a Git history consolidation

Excession can inventory a local Git SHA-1 history, audit the resulting read-only plan,
enumerate local refs, and verify that a separately reconstructed history has the
same exact source tree as a frozen architectural target:

```sh
excession history plan --repo . --rev HEAD --max-commits 10000 > /tmp/history-plan.json
excession history audit --repo . --plan /tmp/history-plan.json
excession history refs --repo . --max-refs 10000
excession history verify-target --repo . --target main --replacement curated-branch
```

These commands do not rewrite Git history or authorize a ref update. Adjacent
commit-message scopes are only proposals for review, not correctness proofs.
See [target architecture](docs/TARGET_ARCHITECTURE.md) and
[history safety contract](docs/HISTORY_REORGANIZATION.md).

## Test what changed

Run Excession inside a Git checkout. Its default is a read-only
test plan, not an unexpected execution of project configuration:

~~~sh
# Once per Linux machine, after installing rootless Podman:
sh tools/bootstrap-test-sandbox.sh

# From any directory: show runtime/image readiness without executing tests
excession tests --doctor

# From your project: plan first, then run the same source snapshot safely
excession tests --json
excession tests --run --json
~~~

For **uncommitted** changes, the one-time setup image is found locally by
its Excession label and resolved to its immutable SHA-256 ID. There is no
need to copy a digest into every command. The JSON plan's `next_argv`
pins both image and source fingerprint for agents. Use `--sandbox-image
sha256:<ID>` for a different preinstalled image. `--doctor --json`
lists explicit blockers and setup actions without requiring a Git checkout.
Provisioning downloads packages only when the developer explicitly invokes
the setup script; normal tests never download or install dependencies.

Excession chooses the revision comparison and detects installed Jest or a
Python pytest-testmon project. Jest discovers **and runs** related tests;
pytest-testmon selects and runs tests from a previous coverage baseline.
\`--run\` executes a dirty working tree only inside rootless Podman, without
modifying the original checkout. Clean committed checkouts retain trusted
native execution. A first-run Python baseline is built inside the sandbox. Failing tests return exit code 1, an inconclusive
run returns 3, and setup refusals return 2. JSON includes source identity,
executed test IDs, and optional verification obligations.

Use \`--base\`, \`--candidate\`, \`--provider\`, or \`--focus\` to override
discovery. Mixed frameworks require explicit provider choice.
See [native test workflow](docs/NATIVE_TEST_SELECTORS.md).

For advanced reproducible queries, the existing \`index\`, \`inspect\`,
`view`, `browse`, `graph`, `refactor` and `security` commands remain
available. See [CLI usage](docs/CLI.md), [release portability](docs/PORTABILITY.md), [release procedure](docs/RELEASE.md),
and [development setup](docs/DEVELOPMENT.md).

**Combining native evidence:** `analyze` and `impact` automatically
discover and compose existing SCIP symbol-reference and configured-build
observations in the selected SQLite store. Overlapping SCIP sources are
quarantined and build configurations remain independent. Explicit producer
selection still narrows scope. See
[observed relationship analysis](docs/OBSERVED_RELATIONSHIP_ANALYSIS.md).
Build membership is never silently promoted to a source dependency.

## Use Excession through GitHub

**ChatGPT or an agent with the GitHub connector:** request `locate`,
`context`, or `verify` against a full commit SHA by creating an owner-authored
[Excession analysis request](.github/ISSUE_TEMPLATE/excession-analysis.md).
GitHub Actions runs the actual Rust CLI and uploads its original JSON result,
checksum-bearing receipt, and coverage obligations. Retrieve the workflow
artifact through GitHub; no hosted server, Cloud Run instance, or custom MCP
connection is needed. See [request format and artifact validation](docs/GITHUB_NATIVE_ANALYSIS.md).

**Pull requests and other repositories:** use the separate
[reusable GitHub evidence workflow](docs/GITHUB_EVIDENCE.md). It produces an
auditable base/head evidence envelope at pinned source and producer revisions
without executing the target repository's tests or granting merge approval.

Both paths run on GitHub-hosted workers, use read-only contents permissions,
and treat caches as disposable. A green workflow means evidence was produced,
not that semantic correctness, test selection, or merge readiness was proved.
[CI execution policy](docs/CI.md) defines the narrow PR checks and deep gates.

For a *local* coding agent, Excession also has a separate, repository-scoped
[stdio MCP integration](docs/MCP.md). It does not need to be deployed to
ChatGPT to use the GitHub workflows.

## Coding-agent interface

For MCP-capable coding agents, configure Excession in the target repository
without replacing existing agent instructions:

```sh
excession agent init                 # inspect the proposed Codex/Claude config
excession agent init --write         # create only missing project config files
```

This registers `excession mcp`, a local, repository-scoped, read-only MCP
server with source search, context, impact, test planning, coverage and
verification tools. Existing `.mcp.json`, `.codex/config.toml`, `AGENTS.md`
and `CLAUDE.md` are preserved; merge into existing MCP configurations
manually. The installed binary must be on PATH. See [coding-agent MCP setup](docs/MCP.md).

Find candidate code without knowing its paths first:

```sh
excession agent locate --repo . --task "Fix worker lease recovery" --limit 20 --json
```

The locator now uses an incrementally cached, **256-segment** FTS5/BM25
index over authenticated Git sources and reuses `agent context` for
relationships and test suggestions. The default refresh budget is eight
segments per call; use `--index-segments 256` for an explicit full warmup.
Partial coverage is reported instead of silently searching stale partitions.
This is lexical discovery, **not semantic proof**, and does not include
uncommitted edits. Related Git-bound SCIP and static-import neighbors, when
available, appear separately with original producer evidence and uncertainty. See [agent commands](docs/AGENT_CLI.md).


`excession agent context` returns bounded Git source identities, candidate
relationships, coverage gaps and heuristic test suggestions for exact focus
paths. `excession agent verify` compares an isolated candidate commit with
its base, recomputes supported impact evidence and incorporates externally
supplied test/build observations without treating them as verified attestations.
Both emit versioned JSON and unresolved obligations. See
[agent-facing CLI](docs/AGENT_CLI.md) for commands and contracts.

## What you can investigate

| Question | Excession commands | Evidence |
| --- | --- | --- |
| What does this module relate to? | `analyze`, `index`, `inspect dependencies`, `view deps` | Python/JavaScript static candidate relationships at a Git revision |
| What might a committed source change affect? | `changes`, `impact`, `explain` | Exact Git diffs and bounded candidate relationship paths, with uncertainty |
| Can I see a smaller, useful slice of the tree? | `view select`, `compose`, `mount`, `browse` | Virtual paths pointing to original Git blobs |
| Where might refactoring help? | `refactor suggest`, `refactor scip-suggest` | Cycles, repeated bodies, and symbol-reference hypotheses |
| What syntax and imports can be observed? | `syntax scan`, `syntax resolve`, `semantic scan` | Tree-sitter and language-aware source observations |
| How do those observations compare with native tools? | `syntax native-run`, `syntax native-compare` | Compiler-declared inputs and source-bound differences |
| What source supports a scanner result? | `security ingest`, `security audit` | CodeQL SARIF or Semgrep findings tied to Git blobs |
| What Git inputs and artifact bytes can I replay? | `evidence build-record`, `inspect build-provenance` | Pinned source identities, declared toolchain/configuration and independently hashed artifact bytes; no invented build-execution proof |
| What does REA observe about a binary? | `evidence rea-import`, `inspect rea-record` | Original REA Evidence v2 predicates, raw results, limitations and native IDs bound to artifact SHA-256 |
| What evidence belongs to this exact artifact? | `inspect artifact-evidence` | A shared artifact subject joining replayed build records with source-free or source-linked REA observations |

Build provenance and REA are **core evidence capabilities**, not optional
standalone tools. A binary's typed artifact subject joins build-source evidence
to REA-native observations without asserting causal build execution or
semantic correctness. Use `inspect artifact-evidence --db /tmp/evidence.sqlite
--artifact /path/binary` to see both sides, adding an independently audited
`--repo --rev --build-snapshot` triple when source provenance is available.
See [build provenance](docs/evidence-core/build-provenance.md) and
[REA evidence](docs/evidence-core/rea.md).

These are different evidence layers, not interchangeable dependency graphs.
The built-in dependency index currently covers bounded Python/JavaScript
static relationships. Optional Tree-sitter grammars broaden syntax coverage;
SCIP brings in references from language-native indexers; native comparison
supports configured Go, Rust, and JS/TS workflows. Coverage and unresolved
relationships remain visible rather than being filled in by guesses.

### Find refactoring candidates

For an advanced, explicitly indexed checkout, choose a repository and external
output directory before querying its recorded module relations:

```sh
REPO=/path/to/h11
OUT=/tmp/excession-h11
mkdir -p "$OUT"
excession index --repo "$REPO" --db "$OUT/index.sqlite" \
  --name h11 --rev HEAD --roots h11,tests
```

Then inspect or audit the candidates:

```sh
excession refactor suggest --db "$OUT/index.sqlite" \
  --name h11 --rev HEAD > "$OUT/refactor.json"

excession refactor audit --db "$OUT/index.sqlite" \
  --name h11 --rev HEAD --report "$OUT/refactor.json"
```

The report includes source identities and reasons to investigate each
candidate. For resolved symbols, Excession can consume an index produced
by an existing SCIP indexer instead of implementing another language parser.
See [refactoring](docs/REFACTORING.md) and
[self-analysis](docs/SELF_ANALYSIS.md).

### Measure a refactor before trusting it

`excession improve plan` and `excession improve evaluate` share one
source-bound evidence contract across SCIP and indexed Python/JavaScript
observations. They compare normalized source locations, Git changes, and
producer-specific coverage without inferring behavioral equivalence. Rust
extraction remains a language-specific adapter. A separate TypeScript
language-service script can propose an unused JavaScript/TypeScript named-import
deletion; the shared `improve replay` command checks the exact Git source
and previews the edit without writing to the original checkout. Application
requires `--output` with a fresh external worktree on a supported Unix host.
The script requires Excession's source checkout and pinned Node dependencies.

No improvement report permits automatic deletion or merge. See the
[commands, native producer setup, and verification limits](docs/IMPROVEMENT_LOOP.md).

### Inspect scanner evidence

Given an existing CodeQL SARIF report:

```sh
excession security ingest --repo "$REPO" --report "$OUT/report.sarif" \
  --db "$OUT/security.sqlite" --engine codeql

excession security audit --repo "$REPO" --report "$OUT/report.sarif" \
  --db "$OUT/security.sqlite" --engine codeql
```

The importer records findings and reported flows against exact Git source
identities. The audit independently checks the imported records against the
report and source. Excession can also invoke an installed, trusted-local
scanner; see [CLI usage](docs/CLI.md).

## What an audit establishes

An Excession audit checks **source identity and reproducibility of recorded
evidence**. It does not prove a parser complete, a refactoring safe, or a
scanner finding exploitable. Compiler and scanner execution is not sandboxed
by Excession. Keep untrusted builds in separately isolated environments.

This distinction is part of the data model, not a warning repeated on every
command. See the [architecture](docs/ARCHITECTURE.md),
[repository views](docs/REPOSITORY_VIEWS.md), and
[evidence contract](docs/EVIDENCE_QUERY.md) for precise boundaries.

## Research direction

Vulnerability discovery is an application to investigate, not an established
capability of the code-intelligence layer. Excession already imports and audits
source-bound scanner evidence. Whether combining that evidence with repository
relationships, revision history, and runtime tests finds additional confirmed
vulnerabilities needs controlled comparison against existing tools.

See [architecture and security research](docs/ARCHITECTURE.md) for the
implemented boundary and the proposed experiment.

## Develop

```sh
mise install
mise run setup
mise run check
mise run cli
```

`make check` is the ordinary local validation path. Optional adapters,
JUnit output, grammar setup, and supported toolchains are documented in
[development setup](docs/DEVELOPMENT.md).

The old positional `excession research` entry point was retired; it is
not a supported command. See [migration notes](docs/MIGRATION.md).
Benchmarks and their measurement contracts live in
[benchmarks](benchmarks/README.md).

## Reference

- [Complete CLI](docs/CLI.md)
- [Composable views](docs/COMPOSABLE_VIEWS.md)
- [Syntax parsing and native comparison](docs/SYNTAX_PARSERS.md)
- [Semantic contracts](docs/SEMANTIC_CONTRACTS.md)
- [Architecture and security research](docs/ARCHITECTURE.md)
- [Third-party source licenses](THIRD_PARTY.md)
