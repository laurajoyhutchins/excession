# Contributing to Excession

Excession is an **investigation-driven codebase-understanding and
transformation system**. Read the [product contract](docs/PRODUCT.md),
[ADR 0001](docs/DECISIONS/0001-investigation-first.md) and
[agent instructions](AGENTS.md) before changing product behavior.

## Start

Use the [development setup](docs/DEVELOPMENT.md), [CI policy](docs/CI.md),
[documentation home](docs/README.md), and [contracts](contracts/README.md).
The pinned toolchain entry point is:

```sh
mise install
mise run setup
mise run check
```

Keep local checkpoint checks focused; run deep producer acceptance when
the relevant boundary changes.

## What a change must establish

- The user question, behavioral problem or structural improvement it serves.
- Exact Git identities, original source observations, producer/configuration,
  evidence gaps and the smallest useful explanation.
- For source changes: which public behavior, errors, ordering, side effects,
  state and compatibility guarantees must be preserved or intentionally changed.
- For assertions of correctness: independent tests, negative fixtures and
  receipts addressing the **specific** claim.
- For cost claims: total resource cost, including preprocessing, index
  creation, source reads, execution and verification.

Add a failing test or explicitly identified absent acceptance case before
implementation where practical. Skipped or unavailable tests are **not**
passing evidence. A graph edge, passing replay, or schema is not a proof
of behavioral correctness.

## Architecture contributions

Prefer using Git, SCIP, compilers, language servers, parsers and native
test tools as evidence providers. Do not create another full language indexer
or universal source rewriter without a demonstrated investigation benefit.
Keep derived knowledge and SQLite caches subordinate to Git source and
producer provenance. Prefer question-scoped work over global graph expansion.

Record proposed interfaces as **not implemented** until live CLI/MCP code,
versioned contract and independent acceptance exist. For refactoring,
use isolated worktrees and fail-closed native edit replay, never the original
dirty checkout.

## Evaluation and documentation

See [evaluation](docs/EVALUATION.md) and
[benchmarks](benchmarks/README.md): improve actual unfamiliar-repository
understanding and behavior-preserving refactoring, not primarily edge recall.
When modifying a public behavior, update the canonical task guide and
reference documentation, and run [documentation checks](docs/DOCUMENTATION.md).

Historical security, ontology, graph and migration experiments remain
in [technical reference](docs/REFERENCE.md); their presence does not
make them the product direction.

See [SECURITY.md](SECURITY.md) for security disclosure.
