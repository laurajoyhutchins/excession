# Semantic relationship contracts

Excession gives language-specific relationship observations a **shared,
versioned format** so they can be inspected alongside one another without
losing their producer, source revision or uncertainty. The contract is an
interchange format, not a claim that all languages have been resolved into a
complete cross-language graph.

The envelope uses [JSON Schema Draft 2020-12](../contracts/semantic/semantic-observations.v1.schema.json);
dataset metadata and quality obligations use
[ODCS v3.2.0](../contracts/semantic/semantic-observations.odcs.json).
The current resolver emits import candidates or unresolved observations; SCIP
symbol references remain a separate evidence path.

## Producers and source authority

The **default entrypoint** produces the shared contract without saving syntax
or resolution intermediates:

```sh
excession semantic scan --repo "$REPO" --rev HEAD --roots src,tests \
  > /tmp/semantic.json
excession semantic audit --repo "$REPO" --roots src,tests \
  --report /tmp/semantic.json
python3 tools/validate_semantic_contract.py /tmp/semantic.json
```

`semantic scan` reads pinned Git blobs and invokes the installed Tree-sitter
language pack. The Rust-side Python, JS/TS and Go **relationship
resolvers** emit candidate or unresolved observations directly into one shared
sink at the point of each resolution decision. Excession's Rust core adds the
Git-bound source identities and common JSON Schema structure; native syntax
parsers never invent commit or blob identities. The JSON report is
`PARTIAL_CANDIDATES_ONLY`, never a complete graph.
Rust syntax remains visible, but Rust semantic resolution belongs to
rust-analyzer/SCIP and is not synthesized by these syntax adapters.

`semantic audit` independently repeats the same language parsing and
relationship resolution against the saved report's pinned revision and
explicit root selection. It compares the entire canonical report, rather than
accepting an embedded SHA-256 alone.

For existing pipelines, the previous reports remain compatible:

```sh
excession syntax scan --repo "$REPO" --roots src,tests > /tmp/syntax.json
excession semantic resolve --repo "$REPO" \
  --syntax-report /tmp/syntax.json > /tmp/semantic-from-syntax.json

excession syntax resolve --repo "$REPO" --report /tmp/syntax.json > /tmp/resolution.json
excession semantic export --repo "$REPO" \
  --syntax-report /tmp/syntax.json \
  --resolution-report /tmp/resolution.json > /tmp/semantic-from-saved.json
```

`semantic resolve` independently audits the saved syntax report before
running the language resolvers; `semantic export` additionally checks the
saved resolution report against independent regeneration. All three paths
should emit byte-identical semantic receipts for the same pinned inputs.
The historical `excession.syntax-dependencies.v1` report and its
`resolution_id` remain unchanged, including for callers that do not use
semantic contracts.

The direct syntax scan is a source-bound **observation**, not independent
execution attestation. Its replay command is required to claim an independent
Git/source match. The Python JSON Schema validator checks shape, canonical
digests and counts; it cannot authenticate source objects on its own.

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

ODCS 3.2 adds a scoped `context` to document how consumers should interpret
candidate-only results. Its `constraints` prohibit treating missing edges as
absence evidence, upgrading candidates to verified relationships, or trusting
self-consistent digests without independent source replay. These are contract
metadata for consumers, not executable authorization or a security policy
enforcement mechanism. The `completeness` property uses ODCS 3.2's
enumerations to state its only permitted value, `PARTIAL_CANDIDATES_ONLY`.

The **ODCS standard version** is 3.2.0; the **dataset contract version** is
1.1.0; the **JSON Schema observation wire version** stays v1. The changes to
metadata do not rewrite or reinterpret existing observation receipts.

Run offline:

```sh
python3 -m pip install -r requirements/research.txt
python3 -m unittest discover -s tests -p test_semantic_contracts.py -v
```

The tests check the ODCS document against a pinned, unmodified upstream
ODCS 3.2.0 schema, and validate positive/negative semantic fixtures. The
Rust core retains its zero-third-party-Cargo dependency constraint.
New semantic claims (such as compiler-confirmed edges or execution
conditions) require a new explicitly reviewed contract version, not a
silent reinterpretation of `CANDIDATE`.

The pinned ODCS validator schema is copied from
[ODCS v3.2.0 upstream schema](https://github.com/bitol-io/open-data-contract-standard/blob/v3.2.0/schema/odcs-json-schema-v3.2.0.json)
(Apache License 2.0; Git blob SHA-1
`f934075459fce0a20a3c8260dd2076872f91e2b9`).
See [vendor licensing](../contracts/vendor/LICENSE-ODCS).

## Typed cross-source queries

The existing v1 semantic observations remain **syntax candidates only**. The
merged `excession inspect evidence` command retains its v1 wire shape and now
enforces [a separate closed JSON Schema](../contracts/evidence/evidence-query.v1.schema.json)
before it returns JSON. An [ODCS 3.2 contract](../contracts/evidence/evidence-query.odcs.json)
describes this query dataset and its three evidence categories.

The [evidence validator](../tools/validate_evidence_contract.py) checks
shape, counts, self-hash, receipt-source consistency and compatibility of every
static candidate/unresolved import with the semantic v1 observation vocabulary.
It **never** maps compiler-declared inputs or runtime witnesses into
`CANDIDATE` or `UNRESOLVED` syntax observations. Only the runtime execution
collector can supply a witness, and the returned self-hash is not a trusted
execution signature. No source claims runtime completeness.

The query validator is integrated into [the real CLI fixture](../tests/test_evidence_query_cli.py)
and its [adversarial contract tests](../tests/test_evidence_contracts.py).
Run both catalog checks offline once pinned dependencies have been installed:

```sh
python3 tools/validate_semantic_contract.py
python3 tools/validate_evidence_contract.py
```


## Indexed JavaScript and TypeScript imports

The ordinary `analyze` and `index` commands use the pinned TypeScript
compiler's module resolver instead of custom filename-extension guesses.
The compiler sees a virtual snapshot of tracked JS/TS and configuration
blobs (`package.json`, `tsconfig.json`, `jsconfig.json`), never an
uncommitted checkout. Resolved aliases and package entry points are candidates.
Unsupported compiler configuration inheritance, project references, plugins,
and absent or out-of-index targets produce explicit unresolved observations.
No result establishes runtime dependency completeness or permission to skip
verification. The separately versioned `semantic scan` resolver is unchanged.
