# Investigation-first documentation cutover

**Decision in force:** [ADR 0001](DECISIONS/0001-investigation-first.md).

This page inventories what a **complete documentation cutover** means
without misrepresenting it as a completed runtime feature migration.

## Canonical replacements

| Previously central framing | Current canonical home |
| --- | --- |
| Graph/index query as product | [Investigations](INVESTIGATIONS.md) |
| SCIP coverage as product acceptance | [Evaluation](EVALUATION.md); SCIP recall is diagnostic |
| Standalone refactoring candidate detection | [Transformations](TRANSFORMATIONS.md) and preservation obligations |
| Unqualified "verified" result | [Verification](VERIFICATION.md) and explicit evidence states |
| Collections of producer documents as roadmap | [Evidence lifecycle](EVIDENCE_LIFECYCLE.md) and [reference](REFERENCE.md) |
| Modularization target branch as product architecture | [Architecture](ARCHITECTURE.md); old [plan](TARGET_ARCHITECTURE.md) is historical |

The README, documentation index, architecture, contributor guidance,
agent instructions, quick start, CLI/MCP context, CI policy, benchmark
introduction, issue template and specialist references must route to
these canonical documents. Retained legacy pages still describe real
native commands or historical experiments and must remain readable.

## Present interfaces remain intact

Do not rename, remove or pretend to have replaced existing CLI/MCP or
versioned JSON contracts as a side effect of documentation. Today, `agent locate` and `agent context` remain evidence-gathering tools;
`refactor` and `improve` remain bounded and advisory/native-edit tools.
The `excession investigation` append-only ledger now persists source
candidates, claims, open questions, attributed synthesis, preservation plans,
direct-source refresh and `agent verify` reports. Fully automated semantic
explanations, transitive impact re-establishment and the proposed autonomous
`investigate` / `transform` agent APIs still require implementation.

## Product/runtime follow-through (not shipped by this docs change)

- Extend the implemented persistent investigation/claim ledger with richer
  configuration, producer and inference lineage and source-linked follow-ups.
- Implement demand-driven expansion and claim invalidation.
- Connect transformation preservation obligations to the investigation.
- Expose validated agent/CLI entry points without breaking existing tools
  until migration tests demonstrate equivalent capability.
- Benchmark unfamiliar-repository understanding and safe refactoring against
  independent baselines before claiming user benefit.

These are **outstanding engineering requirements**, not a checklist
certifying runtime completion. The cutover establishes one authoritative
documentation architecture. It intentionally does not delete useful
history or promise a feature that cannot be executed.

## Suggested working artifacts

Use the [investigation note template](templates/INVESTIGATION.md),
[transformation plan template](templates/TRANSFORMATION.md) and
[verification report template](templates/VERIFICATION.md) until
versioned machine-readable artifacts exist.
