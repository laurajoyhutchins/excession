# Excession

**Understand software. Refactor it without losing what makes it correct.**

Excession is an evidence-driven **codebase understanding and transformation
system**. An investigation begins with an engineering question, finds the
governing implementation, explains how the relevant pieces behave together,
identifies tests and assumptions, and records what remains unknown. A
transformation uses that understanding to plan a structural change and
independently check the behavior it must preserve.

Git supplies source authority; SCIP, compilers, language servers, tests,
builds and runtime traces supply evidence. **The index and graph are not
the product.** An accurate, reviewable engineering decision is the product.

Read the [documentation cutover](docs/CUTOVER.md) and
[product contract](docs/PRODUCT.md),
[architecture](docs/ARCHITECTURE.md), and
[accepted decision](docs/DECISIONS/0001-investigation-first.md).

## What works today

The Rust CLI operates on exact Git revisions and uses SQLite for derived
indexes and evidence. It can locate candidate implementations, assemble
bounded context and reading plans, identify possible change impact,
inspect source-bound observations, audit report replay, and support
bounded refactoring proposals and native edit replay.

**Important distinction:** a first persistent investigation ledger is now
available through `excession investigation`. It records claims, behavior-
preservation obligations and existing `agent verify` outputs, but **does not
write behavioral explanations or certify equivalence**. A fully automated
investigation engine and transformation command remain target architecture. Current reports do
not themselves certify behavior, semantic equivalence, safe test selection,
or permission to merge.

| Engineering task | Start here |
| --- | --- |
| Understand an unfamiliar behavior | [Investigation workflow](docs/CODEBASE_UNDERSTANDING.md) |
| Evaluate a refactoring | [Transformation workflow](docs/TRANSFORMATIONS.md) and [current refactoring commands](docs/REFACTORING.md) |
| Understand evidence and uncertainty | [Evidence lifecycle](docs/EVIDENCE_LIFECYCLE.md) and [limits](docs/EVIDENCE_AND_LIMITS.md) |
| Check what was preserved | [Verification contract](docs/VERIFICATION.md) |
| Judge whether Excession helps agents | [Evaluation](docs/EVALUATION.md) |
| Integrate a terminal or coding agent | [Quick start](docs/QUICKSTART.md) and [local MCP](docs/MCP.md) |
| Request analysis through GitHub | [GitHub-native analysis](docs/GITHUB_NATIVE_ANALYSIS.md) |

## Record a persisted investigation

```sh
excession investigation start --repo /path/to/repository \
  --rev HEAD --question "How does recovery work?" \
  --db /tmp/excession-investigations.sqlite
```

This returns an `investigation_id` and first event. Use
[`investigation claim`, `plan`, `verify`, and `show`](docs/INVESTIGATIONS.md)
to record source-grounded findings, preservation obligations and scoped
verification. The ledger stores reports outside the source checkout;
it does not modify code or confer merge approval.

## Try a source investigation

Install a [verified release](docs/RELEASE.md) or build using
[development setup](docs/DEVELOPMENT.md). Git is required; some
language-native producers are optional.

```sh
excession doctor
excession agent locate --repo /path/to/repository --rev HEAD \
  --task "Find where a transaction becomes durable" --limit 12 --json
```

Read the returned `reading_plan` and the **actual Git-pinned source**.
It is a navigation aid, not an automatically produced behavioral explanation.
Use `agent context` for the selected paths, inspect tests and failure modes,
then record supported claims and unresolved questions. Follow
[the practical guide](docs/CODEBASE_UNDERSTANDING.md).

For a committed change:

```sh
excession impact /path/to/repository --output /tmp/excession-impact.json
```

The result is a bounded source-impact candidate report, not proof that a
behavior changed. See [quick start](docs/QUICKSTART.md),
[CLI](docs/CLI.md), and [agent CLI](docs/AGENT_CLI.md) for exact syntax.

## Available surfaces

- **Local CLI:** native analysis and specialist evidence/refactoring commands.
- **Local stdio MCP:** repository-scoped, read-only `search_code`,
  `get_context`, `explain_impact`, `verify_change`, `plan_tests` and
  `check_coverage`. No live `investigate` or `transform` MCP tool yet.
- **GitHub Actions:** owner-authorized on-demand public-repository source
  reports and separate PR evidence artifacts. They do not run unspecified
  target-project tests or authorize merging.

See [MCP](docs/MCP.md) and [GitHub-native analysis](docs/GITHUB_NATIVE_ANALYSIS.md).
This GitHub-based path does not require a custom ChatGPT MCP server,
Cloud Run or another deployment.

## Development

```sh
mise install
mise run setup
mise run check
```

Review [contributing](CONTRIBUTING.md), [agent instructions](AGENTS.md),
[CI](docs/CI.md), and the [documentation index](docs/README.md).
Producer experiments, graphs and historical designs remain available
through the [technical reference](docs/REFERENCE.md), but they do not
override the investigation-first direction.

Licensed under the [repository license](LICENSE).
