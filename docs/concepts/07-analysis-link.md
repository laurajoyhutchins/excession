# PR 7: Attach semantic concepts to existing analysis

Status: additive annotation contract + TDD scaffold. Existing refactor, impact and view schemas remain authoritative and unchanged.

## Contract

A concept annotation references an immutable semantic snapshot, an original source-bound analysis report and existing candidate IDs. It must not replace, strengthen or silently rewrite the underlying analysis relationship. A static scanner result remains unverified even if its related concept is reviewed.

## Integrations after implementation

- Refactor: annotate potential shared abstractions, conflicting interfaces, caller/test constraints and independent preservation obligations.
- Impact: display concept context beside existing typed Git/SCIP/build edges, never treat concept membership as a compilation dependency.
- Views: select original Git-backed source identities using concept membership; preserve existing read-only view manifests.
- Security: optional investigative concept classification, not a vulnerability verdict.

## TDD fixtures

- The enriched and unmodified baseline report must have identical original source-edge IDs.
- A concept reference to the wrong revision or nonexistent claim is rejected.
- A proposed concept cannot authorize skipping a test or certifying a code edit.
- A scanner finding plus reviewed concept remains a static finding.
- Replaying an enrichment report must reauthenticate both source reports and the semantic snapshot.
- A normal repository-view read returns unchanged original Git blob bytes.

No new view engine, executable checkout, parser, or refactoring transformation.
