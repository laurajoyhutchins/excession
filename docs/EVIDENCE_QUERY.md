# Inspect evidence for a source file

Use `excession inspect evidence` to see which relationships and observations
are associated with a file at a pinned Git revision. The result can include
static relationship candidates, compiler-declared inputs and recorded runtime
witnesses, each with its own source and evidence type.

The query keeps those categories separate rather than treating them as a
single dependency graph. This initial interface is read-only and path-scoped;
missing observations are reported as gaps, not inferred absences.

## Query

Generate the existing source-bound syntax and resolution receipts first:

```sh
excession syntax scan --repo "$REPO" --rev HEAD --roots src \
  > /tmp/excession-syntax.json
excession syntax resolve --repo "$REPO" \
  --report /tmp/excession-syntax.json \
  > /tmp/excession-resolution.json

excession inspect evidence --repo "$REPO" --rev HEAD \
  --focus src/main.rs \
  --syntax-report /tmp/excession-syntax.json \
  --resolution-report /tmp/excession-resolution.json \
  --declaration /tmp/compiler-declaration.json \
  --runtime-report /tmp/python-runtime-witness.json
```

`--declaration` and `--runtime-report` are optional. The compiler declaration
is an existing `excession.input-declaration.v1` receipt, e.g. from
`excession syntax cargo-depfile`. The runtime report is an opt-in
`excession.python-runtime-witness.v1` receipt from
`tools/python_import_observer.py`; no program is executed by the query.

Only a regular file in the selected SHA-1 Git tree may be the focus. This
initial query is **path-based**, not a claim to resolve all function/symbol
identities or all historical commits in one request.

## Result semantics

- **`static_candidates`:** source or target equals the focused path.
  Replays the original syntax parser and conservative resolution against Git
  before returning any relationship. Includes source blob IDs and the
  resolution receipt ID. An edge is still a *static candidate*.
- **`unresolved_imports` / `source_gaps`:** explicit partial
  observations for the focused source. Missing evidence is never a negative
  dependency conclusion.
- **`compiler_inputs`:** Git-checked paths in a scoped native input
  declaration, either all entries when focus is its subject or the relevant
  entry when focus is a declared input. The declaration must contain matching
  `source-commit` and `source-tree` assumptions; otherwise it is
  refused. It is **not** an independently attested compiler invocation and
  compiler input membership does not establish a direct import edge.
- **`runtime_witnesses`:** retained witness rows when the focused path
  is their Git-checked target, or when the focus is the original execution
  entrypoint. Entry-point matches are labeled `EXECUTION_CONTEXT_ONLY` and
  never promoted to a direct edge from that file. The raw caller module name
  and module origin are preserved. The report's self-hash, pinned entrypoint
  blob and Git-bound target blob identities are checked.
- **`runtime_gaps`:** the observer's explicit unmediated-load,
  pre-instrumentation, child/native process and truncation limitations are
  retained for relevant executions.

The response has its own canonical `query_id` digest, an explicit
`excession.evidence-query.v1` type and independent provenance receipts.
Its digest is **not** a signature. A compiler declaration can be fabricated,
and a runtime witness report can be fabricated and rehashed. Those records
are authenticated to the pinned source *paths*, not to an independent worker.
Where no source of evidence is supplied, its receipt field is JSON null.

## Intentional limits

- Read-only query, no subprocess observer, no hidden background work.
- No authoritative SQL mutation or security/test-skipping authorization.
- No semantic collapse of static candidate, build input and runtime observation.
- No assertion of complete imports, function calls, test impact or runtime
  dependencies across any configuration.
- No assertion that a source file currently matching Git must have been the
  bytes actually executed by an untrusted Python interpreter.
- `excession.semantic-observations.v1` currently represents
  **static candidates only**. The multi-source query has a separate envelope
  until distinct evidence variants are reviewed and versioned in JSON Schema
  and ODCS. This avoids silently reinterpreting its existing `CANDIDATE` state.

Run the focused integration:

```sh
python3 -m pytest -q tests/test_evidence_query_cli.py
```

The test uses a real clean Git revision, real syntax/resolution replay, a
trusted Python program with a dynamic import, and a synthetic native input
**declaration shape**. Its synthetic declaration does not claim actual
compilation. The Cargo depfile self-analysis workflow supplies separate real
compiler input evidence.

## Versioned wire and dataset contracts

The command preserves the existing `excession.evidence-query.v1` output format
and now checks the full result against
[JSON Schema Draft 2020-12](../contracts/evidence/evidence-query.v1.schema.json)
before printing it. The corresponding
[ODCS v3.2.0 dataset](../contracts/evidence/evidence-query.odcs.json)
lists every field and states the non-promotion and non-completeness rules.

The separate [semantic observations v1 contract](../contracts/semantic/semantic-observations.v1.schema.json)
continues to mean `CANDIDATE` or `UNRESOLVED` **static** imports.
The contract validator projects only the query's static import rows to that
vocabulary. Compiler source inputs remain `DECLARED_INPUTS_ONLY` with
`UNATTESTED_NATIVE_DECLARATION` provenance, and runtime records remain
`OBSERVED_RUNTIME_ONLY` with `SELF_HASHED_UNATTESTED_EXECUTION` provenance.
An absent optional receipt remains JSON `null` rather than negative evidence.

The Python contract validator checks consistency and versioning, not the truth
of a compiler or interpreter invocation. The Rust query's source replay remains
the required authority for pinned Git relationships. A query ID is a
reproducible unkeyed digest, not an attestation.

```sh
python3 tools/validate_evidence_contract.py /tmp/saved-evidence-query.json
```

