# Evidence lifecycle and lineage

This page defines the evidence-system architecture behind investigations
and transformations. The five lifecycle responsibilities borrow from the
data engineering lifecycle described in *Fundamentals of Data Engineering*
by Joe Reis and Matt Housley: generation, ingestion, storage, transformation,
and serving. The analogy is about **trustworthy information flow**, not
about adopting a warehouse stack or running unconditional ETL jobs.

## Responsibilities

| Stage | Inputs and outputs | Acceptance / refusal |
| --- | --- | --- |
| Generation | Git objects, compiler/SCIP reports, test/build results, runtime traces | Identify producer, configuration, scope and origin; never assign borrowed semantics |
| Ingestion | Raw producer bytes -> normalized, typed observations | Verify requested Git source identity and parser/schema boundaries; preserve raw receipt and gaps |
| Storage | Immutable/versioned observations and derived indexes | SQLite is rebuildable; original Git and external receipts retain their own authority |
| Transformation | Evidence -> question-scoped relationships, hypotheses, claims, behavioral models | Every derived claim names supports, assumptions, transformations, and counterevidence |
| Serving | Investigations, plans, verification reports, CLI/MCP/Actions artifacts | Report identity, revision, coverage, unknowns, and any expired or unavailable evidence |

These are conceptual responsibilities. Existing Rust/SQLite paths implement
parts of the flow, including an append-only
[`investigation` ledger](INVESTIGATIONS.md) with candidate source discovery,
attributed synthesis, open questions, direct-source refresh and
independently replayed Git source-line witnesses for proposed control,
data and state operations. A trusted-local differential runner now records
one-invocation exit/output observations and optional best-effort CPython
line/local-value event traces under exact base/candidate source identities
and a witness-backed plan. Witness interpretation, runtime
attestation and all-input equivalence remain unproved.
Full dependency-aware invalidation and autonomously verified behavioral
models remain targets, not implemented guarantees.

## Authority and identity

**Source identity:** repository, commit OID, root tree OID, path,
entry mode, blob OID. The exact identity rules and unsupported objects are
documented in [evidence and limits](EVIDENCE_AND_LIMITS.md).

**Producer observation:** record producer/version, input source OIDs,
configuration, scope, raw content digest, original claim type and receipt.
A compiler declaration is not a runtime trace. SCIP references are not
necessarily calls. Test receipt success is not test-suite completeness.

**Derived claim:** record the question, exact source scope, derivation,
supporting observations and source windows, assumptions, conflicting
observations, unknowns, and status. A derived ID must depend on its relevant
inputs. A semantic claim cannot be made true by hashing its JSON.

**Engineering decision:** a transformation links to claims and explicit
preservation obligations. Its candidate revision and validation receipts
have independent identities. No candidate may inherit a passed verification
state merely because its base revision was checked.

## Lineage and invalidation

When source or a producer configuration changes:

1. Compute exact changed Git identities and configuration differences.
2. Invalidate *dependent observations* whose source/configuration bounds
   no longer hold. Retain original receipts for historical replay.
3. Mark derived claims supported by invalidated evidence **stale** or
   **requiring re-examination**. A lack of known dependency does not imply
   an unaffected behavioral claim.
4. Refresh only relevant evidence when a new investigation asks for it.
5. Re-run independent checks needed by any claim the system intends to
   re-establish at the new revision.

Cross-revision reuse is allowed only for explicitly authenticated, unchanged
inputs and compatible provider/tool configurations. Reusing the same source
blob does not prove the same behavior when a caller, build flag, environment,
generated code or dependency changed.

## Consistency and trust

- Preserve raw observations and separately store normalized projections.
- Use explicit `PARTIAL`, `UNSUPPORTED`, `REFUSED`, and `OPEN_OBLIGATIONS`,
  with reasons and affected scope.
- Unknown evidence is not evidence of absence.
- Independent oracles and reproductions must name exactly which property
  they check; parser self-replay does not validate parser semantics.
- Avoid running project code just to populate an index. Trusted-local or
  isolated execution must be a separate, authorized operation.
- Do not move user source into a service or retain private code by default.

Versioned JSON Schema should describe wire-level artifacts when implemented.
A draft schema does not confer an implemented API. For cross-repository
contracts, use the repository's stated JSON Schema/ODCS policy where
applicable, without fabricating an ODCS registry or duplicate catalog.

## Economics

Cache only what is reused. Record ingestion work, source bytes read, number
of files inspected, analysis wall time, peak memory, and independent
verification cost. Avoid global preprocessing whose cost exceeds the benefit
for actual investigations. See [evaluation](EVALUATION.md) and
[architecture](ARCHITECTURE.md).
