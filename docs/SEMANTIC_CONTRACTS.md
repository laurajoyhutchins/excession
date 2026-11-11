# Semantic relationship contracts

Excession's language-native resolvers feed one **versioned semantic observation
envelope**. This is deliberately narrower than a complete cross-language
symbol graph. The format is defined by [JSON Schema Draft 2020-12](../contracts/semantic/semantic-observations.v1.schema.json);
dataset metadata and quality obligations are declared with
[ODCS v3.1.0](../contracts/semantic/semantic-observations.odcs.json).

## Current producer

The Rust CLI converts existing source-replayed syntax-resolution reports:

```sh
excession syntax scan --repo "$REPO" --roots src,tests > /tmp/syntax.json
excession syntax resolve --repo "$REPO" --report /tmp/syntax.json > /tmp/resolution.json
excession semantic export --repo "$REPO" \
  --syntax-report /tmp/syntax.json \
  --resolution-report /tmp/resolution.json > /tmp/semantic.json
python3 tools/validate_semantic_contract.py /tmp/semantic.json
```

`semantic export` replays the pinned Git/Tree-sitter syntax scan and
conservative language resolver before emitting any record. The validator is
**schema + digest + count/uniqueness only**, not a substitute for that replay.

## Observation semantics

- `source_commit` and `source_tree` identify the pinned SHA-1 Git revision;
  each source or target has a tracked pathname and Git blob SHA-1.
- `kind` identifies a common vocabulary (`IMPORTS`, `DEFINES`, `CALLS`,
  etc.). In this first adapter **only `IMPORTS` is emitted**. Other vocabulary
  entries reserve typed representation, not implemented extractors.
- `CANDIDATE` has a tracked target and a named language-specific rule.
  `UNRESOLVED` has **no target**. No native compiler or dynamic semantics
  are implied by either record.
- `condition_evaluation: NOT_EVALUATED` is compulsory. The absence of a
  build condition is *not* an assertion that the relationship is unconditional.
- `gaps` contain excluded/unsupported sources and partial parse states.
  Even zero gaps does not establish completeness: syntax extraction,
  dynamic imports, generated sources, build flags, macros, aliases, and
  runtime dependencies can remain unknown.
- `observation_id` is SHA-256 of the canonical JSON observation **without
  its own ID**; `report_id` similarly covers the full report body. Both are
  reproducible, unkeyed digests, not signatures or independent attestation.
- Every report is permanently marked `PARTIAL_CANDIDATES_ONLY`; absence of
  an edge must not become permission to omit tests or declare safety.

## Quality and compatibility

The accompanying ODCS record describes the dataset and names three quality
checks: schema conformity, content identity/uniqueness, and independent Git
replay. ODCS is **metadata**, not an alternate structural validator. The
versioned JSON Schema is normative for the wire format.

Run offline:

```sh
python3 -m pip install -r requirements/research.txt
make contracts-check
```

The tests check the ODCS document against a pinned, unmodified upstream
ODCS 3.1.0 schema, and validate positive/negative semantic fixtures. The
Rust core retains its zero-third-party-Cargo dependency constraint.
New semantic claims (such as compiler-confirmed edges or execution
conditions) require a new explicitly reviewed contract version, not a
silent reinterpretation of `CANDIDATE`.

The pinned ODCS validator schema is copied from
[bitol-io/open-data-contract-standard](https://github.com/bitol-io/open-data-contract-standard/blob/main/schema/odcs-json-schema-v3.1.0-20260616.json)
(Apache License 2.0; Git blob SHA-1
`558bb0bc5f40714cef57698731a232b26d988a71`).
See [vendor licensing](../contracts/vendor/LICENSE-ODCS).
