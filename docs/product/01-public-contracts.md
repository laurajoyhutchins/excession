# Product CLI and evidence contracts (scaffold)

**Status:** draft specification, not implemented by this PR. Tracks #82 and establishes the shared contract vocabulary for #79–#86. All existing commands, evidence receipts and SQL stay unchanged.

## Contract boundaries

- Git objects and SQLite are Excession's only repository evidence authority. SHA-1 Git format only until separately supported.
- `common.v1.schema.json` contains small reusable **wire types**, not a second database, parser, semantic graph or ontology.
- `source_ref` binds a path, mode and blob to an exact commit/tree. Shape validation cannot authenticate Git bytes.
- `evidence_ref` names a producer's observation; `assessment` is not an overall verification flag. A compiler's declared input, static candidate and SCIP reference must retain their distinct meanings.
- `coverage.status=DECLARED_SCOPE_COMPLETE` is limited to the named producer/configuration/scope; it never proves universal runtime closure. An omitted relationship is unknown, not absent.
- Explanation and impact reports reference the original evidence, and audits separately report structural validation, Git replay and semantic verification.
- `ontology_link_ref` is **annotation-only**, anticipating the distinct semantic-intelligence scaffold #88–#95, especially #94's `contracts/concepts/analysis-link.v1.schema.json`. Its claim/subject/concept identities are **not** dependency graph edges, proof of behavior, or a reason to skip tests. No local copy of `contracts/concepts/*` is introduced.

## CLI contract (proposed)

- Human-readable text by default for new workflows; `--json` selects a versioned report. Existing specialized JSON commands retain their current wire semantics.
- Machine stdout contains one entire report, not log messages. Errors and progress use stderr. Deterministic ordering and canonical IDs must be tested.
- Exit 0: successful execution including explicit PARTIAL coverage. Exit 2: bad input; 3: unavailable requested capability; 4: evidence refusal; 5: internal failure; 6: resource-limit refusal. These are **proposed**, not implemented in the current CLI.
- A future `--require-complete` may return nonzero for declared incomplete scope, but normal partial observation is not an execution error.
- Existing `excession analyze clang` remains reachable when `excession analyze .` is introduced. The retired positional `excession research` interface must not return.

## TDD boundary

Run the **active schema tests** with:

```sh
python3 -m unittest discover -s tests -p 'test_product_*.py' -v
```

The behavioral tests are deliberately `skip`ped until the corresponding native implementation exists. A skipped test is **not** successful behavior verification.

## Compatibility

Changes to interpretation of evidence kinds, source identity or an analysis status require a new contract version. Existing `semantic-observations.v1`, `evidence-query.v1`, ODCS declarations and original stored receipts are not rewritten. Relational source facts and ontology claims have distinct IDs and cannot be implicitly interchanged.
