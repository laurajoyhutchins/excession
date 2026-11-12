# Contract boundaries

Contracts are for durable **cross-process or externally consumed JSON**.
Rust types and SQLite constraints implement internal invariants; they do not
require another serialized contract for every pipeline stage.

Retained boundaries:
- Git-bound source identity, producer provenance and scoped coverage.
  [Evidence identity](evidence-core/identity-receipt.v1.schema.json),
  [coverage](evidence-core/coverage.v1.schema.json), and exported
  [snapshot manifests](evidence-core/persistence.v1.schema.json) have
  distinct trust limitations.
- [Semantic observations](semantic/semantic-observations.v1.schema.json)
  and [evidence queries](evidence/evidence-query.v1.schema.json) keep
  static candidates, compiler inputs and runtime witnesses separate.
- [Scanner runs, coverage and candidates](security/) represent the
  scanner's own vocabulary, not verified vulnerabilities.
- [Public command reports](product/) use shared source/evidence/coverage
  references. [Release manifests](product/release-manifest.v1.schema.json)
  describe shipped artifact digests, not signed publisher attestations.

**Not contracts:** acceptance-case manifests, installed-smoke transcripts,
contributor onboarding examples and generic CLI-summary scaffolds. They were
test-only proposed JSON shapes, not consumed public wire formats. Their
behavioral tests were skipped; executable Git/native acceptance, portable
archive tests and documentation now own those responsibilities.

A JSON Schema validates shape, not the authenticity of Git objects, correctness
of producer interpretation or test results. ODCS is dataset metadata, not an
executed quality check; do not add it to internal Rust/SQLite records by default.
Existing v1 evidence formats and native scanner payloads remain unchanged.
