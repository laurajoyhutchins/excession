# Contracts

Excession uses JSON Schema for source-level JSON record contracts and
ODCS for producer/consumer dataset metadata. Do not conflate an ODCS
quality declaration with an executed test or a JSON digest with an
authenticated source revision.

The [semantic observation envelope](semantic/semantic-observations.v1.schema.json)
has an [ODCS dataset contract](semantic/semantic-observations.odcs.json).
See [usage and evidence boundaries](../docs/SEMANTIC_CONTRACTS.md).

The typed [evidence-query JSON Schema](evidence/evidence-query.v1.schema.json)
and [ODCS 3.2 dataset entry](evidence/evidence-query.odcs.json) describe
the read-only `excession inspect evidence` result. They preserve static
semantic v1 observation meaning rather than reusing `CANDIDATE`
for compiler inputs or runtime witnesses.

Security discovery has **three distinct v1 records**:
[scanner execution](security/scan.v1.schema.json),
[coverage and rule applicability](security/coverage.v1.schema.json), and
[raw source-bound candidates](security/candidate.v1.schema.json).
See the [security contracts and provenance boundaries](../docs/SECURITY_CANDIDATE_DISCOVERY.md).
These are internal JSON exchange records; an ODCS catalog is unnecessary
until a dataset is published across repository boundaries.
