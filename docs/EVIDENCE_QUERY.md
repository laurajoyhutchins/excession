# Read-only multi-evidence source query

Excession can now answer **one pinned source-path query** using separate static,
compiler-declared and runtime-observed receipts. It does not merge these into
one authoritative edge table or claim complete runtime dependency coverage.

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
