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
excession impact . --base HEAD~1 --output /tmp/excession-impact.json
```

The default impact output lists candidate dependent paths and their full
`claim_id` values. `--output` saves the exact replayable JSON without
overwriting an existing file or writing into the source repository. Use an
absolute path in an existing external directory. If a candidate is listed,
copy its claim ID to inspect its evidence:

```sh
excession explain . --report /tmp/excession-impact.json \
  --id YOUR_CLAIM_ID
excession explain . --report /tmp/excession-impact.json \
  --id YOUR_CLAIM_ID --audit
```

The `--audit` result distinguishes source replay from independent semantic
validation. Source replay does **not** establish that an affected module is
broken or that a test can safely be skipped. Reports explicitly preserve
unsupported and unresolved coverage.

These workflows operate on committed revisions. They can read pinned Git
objects even when the working tree is dirty; they do not include uncommitted
edits. Default SQLite caches live outside the repository. Exact-output workflows
also support `--json` and explicit revisions/roots.

For advanced reproducible queries, the existing `index`, `inspect`,
`view`, `browse`, `graph`, `refactor` and `security` commands remain
available. See [CLI usage](docs/CLI.md), [release portability](docs/PORTABILITY.md), [release procedure](docs/RELEASE.md),
and [development setup](docs/DEVELOPMENT.md).

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
