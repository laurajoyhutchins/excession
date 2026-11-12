# PR 8: Independent product acceptance and falsification protocol

**Status:** preregistered acceptance schema, no executed result claimed.

Reuse the already established `benchmarks/schemas/benchmark-report.v1.schema.json`, `docs/DEPENDENCY_COVERAGE.md` and independent import oracles. The new case manifest identifies pinned source, a **separate** expected source of truth, scope, known unknowns and adversarial changes. SQL-to-SQL comparison is not an independent semantic oracle.

Acceptance covers the actual product path: installed binary → analyze → change inventory → impact → explain → audit. Every expected relation must be independently checked against Git source and/or a language-native producer. A source-bound SCIP reference is not automatically a call or import; success counts remain source-relation-specific.

Include positive and negative changes, false and missing edges, stale blob, unmatched configuration, limits, unavailable adapter, and coverage denominator. Collect wall time, peak RSS and artifact sizes via existing benchmark mechanism, including startup cost.

**Auto-ontology:** reuse semantic-intelligence evaluation scaffold #95 for held-out concept precision/recall rather than creating another concept benchmark. A concept-enrichment result may be linked only by its evaluation report ID. It cannot change this product oracle's expected dependency edges or validate scanner findings.

**TDD:** shape tests now; held-out Git/native evaluations remain explicitly skipped. Keep deep external fixtures manual/scheduled, not per-commit heavyweight CI.

Tracks #84, #95, #101.
