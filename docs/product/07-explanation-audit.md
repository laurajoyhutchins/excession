# PR 7: Explain original evidence and audit each validity domain

**Status:** typed draft schemas, documentation and TDD only.

`excession.explanation.v1` gives an ordered relationship path with original Git paths/blobs, producer evidence and relationship kinds. It should reuse `graph path`, `inspect evidence` and source replay, not create another graph or semantic index.

`excession.audit-result.v1` reports structural validation, source Git authentication, deterministic replay, and independent semantic checking separately. A passing schema/digest/SQLite replay never establishes semantic truth. Only independent oracle evidence may support `INDEPENDENTLY_TESTED`.

Ontology links from #94 may appear as annotations beside explanations but do not strengthen original dependency, scanner or refactor claims. Respect the auto-ontology #88–#95 snapshot/review contracts.

**Tests:** active wire and invalid promotion checks; future Git span/replay/stale blob/multi-hop validity tests deliberately skipped.

Tracks #81, #84, #94, #101.
