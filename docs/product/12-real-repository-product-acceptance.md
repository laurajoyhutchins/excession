# Pinned real-repository product acceptance

Extends the existing real-repository projection suite and its **independent Git-source import oracle**. No duplicate parser, index or test runner is introduced.

The bundled h11 upstream snapshot and pinned Axios source are checked by the ordinary product CLI: `analyze` is compared with relationships independently derived from source Git blobs, then a deterministic local commit changes an independently selected imported source. `impact` must include the known importer as a candidate, `explain` must source-replay the claim, `--audit` must retain `NOT_EVALUATED` semantic correctness, and a tampered report must be refused.

Records exact upstream and local commits, scoped pair counts, unknown import sites, source/impact/explanation/audit wall times, and SQLite bytes. It **does not** establish production-repository usability, universal language coverage, source line correctness for all possible imports, full configured build semantics, runtime completeness, or peak RSS. The existing oracle's exact site-level comparison remains a separate arm.

This belongs in the dedicated `.github/workflows/real-fixtures.yml` (manual or suite-specific PR changes), **not** every quick commit. If upstream network access fails, the acceptance is blocked rather than silently counted as passed. Auto-ontology #88–#95 may provide optional concept annotations, not substitute for the source oracle.
