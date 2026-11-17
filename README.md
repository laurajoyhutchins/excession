# Excession

Excession analyzes source code at exact Git revisions. It records observed
relationships, retains the identity and provenance of their sources, and
reports what remains unknown. The implementation is a Rust CLI with SQLite
indexes and integrations with language-native analyzers.

Its first job is to help an engineer understand unfamiliar code: find the
relevant implementation, work outward to callers and dependencies, inspect
tests and invariants, and identify what a change might affect. Git identities
and language-native observations support this investigation; they are not a
substitute for reading and explaining the source. See the
[codebase-understanding workflow](docs/CODEBASE_UNDERSTANDING.md).

It can help locate an implementation, examine change impact, choose code to
inspect, and audit the evidence behind a report. These are **candidates and
observations**, not proofs of behavioral correctness.

## Choose how to use it

| Where you work | Entry point | What it produces |
| --- | --- | --- |
| ChatGPT with GitHub access | [GitHub analysis request](docs/GITHUB_NATIVE_ANALYSIS.md) | A GitHub Actions artifact containing the native JSON report, pinned Git identities, and receipt |
| Pull requests and CI | [Reusable GitHub evidence workflow](docs/GITHUB_EVIDENCE.md) | Base/head evidence, coverage, original verification output, and unresolved obligations |
| Terminal or editor | [Native CLI](docs/QUICKSTART.md) | Source queries and local, replayable reports |
| Local coding agent | [Stdio MCP server](docs/MCP.md) | Read-only repository-scoped tools launched by the agent host |

The GitHub paths use hosted Actions runners and GitHub artifacts. They do
**not** require Cloud Run, another service, or a custom ChatGPT MCP connection.

## Local quick start

Install the [appropriate release archive](https://github.com/laurajoyhutchins/excession/releases)
and verify its adjacent `.sha256` file. Keep the executable and bundled
`lib/typescript.js` together. See [installation and supported platforms](docs/RELEASE.md).
Alternatively, build the CLI from the checked-out source using the pinned
Rust toolchain and [development instructions](docs/DEVELOPMENT.md).

From a repository containing supported source:

```sh
excession doctor
excession discover .
excession analyze . --json
excession impact . --output /tmp/excession-impact.json
```

`impact` produces an `excession.impact.v3` report containing
`candidate_source_impacts`. Select a **real path in that report**, then
inspect its evidence:

```sh
excession explain . --report /tmp/excession-impact.json \
  --source path/from/report.py --audit
```

The `impact` comparison defaults to the committed `HEAD~1..HEAD`
range. Use `--base` and `--head` to choose another exact comparison.
These commands do not incorporate uncommitted working-tree edits.
The [quick-start guide](docs/QUICKSTART.md) explains output interpretation,
partial coverage, and supported adapter boundaries.

## Request an analysis from GitHub

Create an owner-authored issue titled **Excession analysis request** in this
repository, using the [issue template](.github/ISSUE_TEMPLATE/excession-analysis.md).
Provide an allowlisted public repository, the full 40-character Git commit SHA,
and a `locate`, `context`, or `verify` request. The
[GitHub-native workflow](.github/workflows/github-native-analysis.yml) builds
and executes the real Rust CLI and uploads the original report and a
checksum-bearing receipt.

Use the existing GitHub connector to find the matching workflow run and
download its artifact. Verify the requested commit, Git tree, report schema,
receipt status, and hash before relying on its contents. See the
[request and validation contract](docs/GITHUB_NATIVE_ANALYSIS.md).

PR evidence is a separate path. The
[reusable workflow](.github/workflows/github-evidence.yml) preserves a
base/head comparison; it does not run independent behavioral tests or
authorize merging.

## What a report means

- **Git identities** bind the observation to a commit, tree, path, mode,
  and blob. They establish what source was examined, not what the program does.
- **Native producer outputs** such as SCIP, compiler declarations, and SARIF
  remain distinct from Excession's interpretations. A static import or
  symbol reference is not automatically a runtime dependency.
- **Coverage and unresolved obligations** are part of the result. A successful
  command or green workflow does not mean complete analysis, validated test
  selection, or safe-to-merge status.

See [architecture](docs/ARCHITECTURE.md),
[evidence and limits](docs/EVIDENCE_AND_LIMITS.md), and
[source-impact semantics](docs/COMPACT_IMPACT.md).

## Documentation

Start with the [documentation index](docs/README.md). The most useful routes
are [codebase understanding](docs/CODEBASE_UNDERSTANDING.md),
[CLI reference](docs/CLI.md), [agent commands](docs/AGENT_CLI.md),
[GitHub workflows](docs/GITHUB_NATIVE_ANALYSIS.md),
[local MCP](docs/MCP.md), [CI policy](docs/CI.md), and
[development setup](docs/DEVELOPMENT.md).

Experimental designs and dated research records remain accessible through
the [research and reference index](docs/REFERENCE.md). A proposed algorithm
or schema in those records is not necessarily implemented.

## Development

```sh
mise install
mise run setup
mise run check
```

See [contributing](CONTRIBUTING.md), [verification policy](docs/CI.md),
and [releases](docs/RELEASE.md). Git is the authority for source;
SQLite indexes and CI caches are reproducible accelerators, not substitutes
for source evidence.

Licensed under the [repository license](LICENSE).
