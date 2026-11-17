# ADR 0001: Investigation-first product and evidence lifecycle

- **Status:** Accepted product direction, November 2026
- **Scope:** Canonical product and documentation architecture
- **Runtime status:** Migration incomplete; historical/native commands retained

## Context

SCIP and language-native indexes already supply most conservatively labeled
calls and type references in the recent experiments. Additional edge coverage
has diminishing value if an agent cannot locate governing behavior, explain
interactions, find relevant tests or assess uncertainty. Existing Excession
material described many useful components but no single primary product unit.

## Decision

1. Make an **investigation** the unit of understanding and a **transformation**
   the unit of architectural change.
2. Retain Git, SCIP, compilers, tests, build receipts and traces as distinct
   evidence providers. Never merge their authority or invent a universal
   language indexer.
3. Organize internal evidence flow using generation, ingestion, storage,
   transformation and serving. Transformation is question-led, not global ETL.
4. Represent claims, supports, assumptions, counterevidence and open
   obligations explicitly; establish independent behavior separately.
5. Make the main acceptance test an independently scored understanding or
   safe-refactoring task, not index coverage or graph size.
6. Preserve supported CLI and MCP contracts until a replacement is
   implemented, tested and explicitly versioned. Documentation must not
   present target interfaces as shipped.

## Rejected alternatives

- Expanding SCIP coverage as the principal roadmap.
- Treating a whole-repository graph as the primary user interface.
- Introducing a universal rewrite engine or a separate Git authority.
- Treating deterministic replay or successful tests as universal proof.
- Replacing working commands with a promised investigation CLI before
  acceptance exists.

## Consequences

The canonical routes are [product](../PRODUCT.md),
[investigations](../INVESTIGATIONS.md),
[transformations](../TRANSFORMATIONS.md),
[evidence lifecycle](../EVIDENCE_LIFECYCLE.md) and
[verification](../VERIFICATION.md).
Legacy graph, ontology, indexer and research pages remain technical
references, not alternative product strategies. User-facing claims must
state current support and explicit gaps.

## Migration acceptance

Docs are cut over when README, documentation navigation, agent instructions,
architecture, contributor guidance and evaluation policy all point to this
decision, and retained specialist pages are not described as the product
goal. Runtime migration requires separately demonstrated investigation
artifacts, persistence and MCP/CLI support. A documentation cutover alone
does not imply that second milestone.
