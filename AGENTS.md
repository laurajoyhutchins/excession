# Excession agent instructions

Excession is an **investigation-driven codebase understanding and
transformation system**. See [product contract](docs/PRODUCT.md) and
[ADR 0001](docs/DECISIONS/0001-investigation-first.md) before changing
architecture, APIs or product documentation.

## What to optimize

Answer concrete questions about unfamiliar software with minimal relevant
source, accurate behavior, useful tests and explicit uncertainty. For
refactoring, preserve a documented behavioral contract and show independent
verification. Do not optimize primarily for SCIP edge count, index size,
graph density, or novel parser coverage.

## Work discipline

1. Pin exact Git source identities and read the governing code.
2. Use SCIP/compilers/builds/tests/traces as scoped, typed evidence providers.
3. Separate observations, source-derived claims, hypotheses and unknowns.
4. For a change, document allowed differences and preservation obligations.
5. Prefer native compiler/LSP edits. Use isolated worktrees and explicit
   authorization for mutation; do not overwrite dirty user work.
6. Re-run relevant checks and preserve failing or unsupported evidence.
7. Report only capabilities that exist at the inspected source revision.

**Current interface:** `excession agent locate`, `agent context`,
`agent verify`, `excession investigation start/claim/plan/verify/show`,
`impact`, `explain`, specialist `refactor`/`improve` commands, stdio MCP
and GitHub Actions evidence. The implemented investigation family is an
evidence ledger; it does not autonomously explain code or transform it.
See [CLI](docs/CLI.md), [MCP](docs/MCP.md), and
[refactoring reference](docs/REFACTORING.md).

## Documentation placement

- Product semantics: [docs/PRODUCT.md](docs/PRODUCT.md).
- Evidence contract and provenance: [docs/EVIDENCE_LIFECYCLE.md](docs/EVIDENCE_LIFECYCLE.md).
- Investigation: [docs/INVESTIGATIONS.md](docs/INVESTIGATIONS.md).
- Transformation: [docs/TRANSFORMATIONS.md](docs/TRANSFORMATIONS.md).
- Verification: [docs/VERIFICATION.md](docs/VERIFICATION.md).
- Experimental acceptance: [docs/EVALUATION.md](docs/EVALUATION.md).
- Historical/producer detail: [docs/REFERENCE.md](docs/REFERENCE.md).

Use the [investigation](docs/templates/INVESTIGATION.md),
[transformation](docs/templates/TRANSFORMATION.md) and
[verification](docs/templates/VERIFICATION.md) note templates until
machine-readable reports are implemented.

Follow [contribution guidelines](CONTRIBUTING.md) and [CI](docs/CI.md).
Changes must pass repository checks and must never relabel proposed
designs as implemented behavior.
