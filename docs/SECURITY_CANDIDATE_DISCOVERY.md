# Security candidate discovery contracts (v1)

Issue: [#128](https://github.com/laurajoyhutchins/excession/issues/128).
Stacked PR: [#129](https://github.com/laurajoyhutchins/excession/pull/129) on the
source-bound security validation baseline, [PR #87](https://github.com/laurajoyhutchins/excession/pull/87).

The historical Starlette/aiohttp experiment is **frozen**. We cannot tune
rules on its outcomes and then call a rerun independent validation.

## Existing authority: Git-bound report ingest

`core/src/security_ingest.rs` accepts CodeQL SARIF 2.1.0 and Semgrep JSON
as **untrusted scanner observations**. It verifies tracked source blobs and
line positions at the checked-out Git HEAD, stores source-linked result and
CodeQL flow evidence in SQLite, and independently replays the report against
the stored records on audit. Its run ID is computed as SHA-256 of
`relgit-security-v1\0<commit>\0<report SHA-256>\0<engine>`.
The audit receipt returns the source commit, report digest, engine, run ID
and `independently_replayed=true`. We reuse this run ID. We do **not**
introduce another security database or claim an audited static alert is
an independently confirmed vulnerability.

## Three independent record types

| Record | Schema | Meaning |
| --- | --- | --- |
| **Execution** | [scan](../contracts/security/scan.v1.schema.json) | Pinned commit, scanner/tool version and provenance, immutable ruleset selection, configuration digest, literal included/excluded source scope, errors, report digest, optional native ingest/audit run |
| **Coverage** | [coverage](../contracts/security/coverage.v1.schema.json) | One scan × language × framework × weakness family × literal source subtree and an evidence-backed applicability claim |
| **Candidate** | [candidate](../contracts/security/candidate.v1.schema.json) | One original scanner result with source blob/line, rule, native ingest candidate ID and raw result pointer; always `UNVERIFIED_STATIC_CANDIDATE` |

A scan **may have zero candidates** while still carrying coverage records.
A coverage record **never requires a filename or line number**. A candidate
may be preserved when its rule is inapplicable or applicability is unknown.
Neither statement establishes vulnerability detection.

### Execution states

- `NOT_RUN`: no scanner executed; no report, no ingest, no exit code.
- `FAILED`: explicit errors, not complete.
- `PARTIAL`: some output may exist, but the scan did not complete.
- `COMPLETE`: successful exit, no recorded execution errors, report bytes,
  and execution evidence. **Not an automatic assertion that a weakness was
  covered**, and not proof that scanner extraction was complete.

Ruleset reference, toolchain reference and config digest are *provenance
assertions*. Their bytes and authenticity must be checked by the executor.
The JSON Schema cannot attest a process or rule semantics.

### Coverage claims

- `NOT_SCANNED`: planned/not-run or the target language/subtree was out of scope.
- `INCOMPLETE`: failed/partial execution or an excluded/partially included subtree.
- `SCANNED_APPLICABLE`: completed scan, fully included subtree, relevant
  rule IDs and **independently justifiable rule-applicability references**.
- `SCANNED_NOT_APPLICABLE`: completed scan; external evidence says its
  rules do not apply to the target framework/weakness.
- `SCANNED_APPLICABILITY_UNPROVEN`: scan completed but the rule/family
  coverage could not be demonstrated.

`SCANNED_APPLICABLE` is a **documented rule applicability claim**, not an
assertion of full sensitivity or demonstrated detection. A false or vague
reference is not magically made trustworthy by a valid JSON record.
In particular, the Django rate-limit alerts reported in the aiohttp held-out
experiment must not count as evidence of CWE-22 discovery.

### Candidate states and identity

There is only one discovery claim: `UNVERIFIED_STATIC_CANDIDATE`.
The `candidate_id` and `logical_key` reuse the IDs from Rust ingest.
The candidate refers to `scan_id` and `coverage_id`, and locates the raw
alert with a JSON Pointer. No candidate field says `SAFE`, `VERIFIED`,
`CVE` or `REPRODUCED`. Behavioral assessment comes later, separately,
using the existing native oracle and assessor.

The Rust ingest candidate ID includes rule, source blob and raw evidence.
It can change across revisions even for a semantically identical issue.
Do not use equal line numbers to deduplicate defects across revisions.

### Semantic validation across records

JSON Schemas enforce shape and local conditionals.
[`tools/validate_security_discovery_contract.py`](../tools/validate_security_discovery_contract.py)
enforces joins and constraints the schemas cannot:

- Candidate and coverage must reference an existing scan.
- Covered languages/subtrees must be inside the scanner's declared scope,
  without intersecting exclusions; partial/failed scans cannot claim coverage.
- Inapplicable alerts are preserved as unadjudicated observations.
- No candidate may come from an unscanned/incomplete scope.
- Every candidate requires a completed scan and an ingest run *claiming*
  independent replay, with its ID matching the Rust run ID.
- Duplicate/contradictory coverage keys, reused candidate IDs, and
  decreasing source ranges fail closed.

This checker is **not** an audit implementation. It validates the
structure and internal consistency of the supplied receipt claims, but
does not independently fetch the Git objects, scanner binary, rules,
original SARIF/JSON, or Rust SQLite database. Only the existing Rust
`security ingest` and `security audit` operations establish the
source-bound replay claim.

## Fast TDD check

```sh
python3 -m unittest discover -s tests -p test_security_candidate_contract.py -v
python3 tools/validate_security_discovery_contract.py tests/fixtures/security-discovery.v1.json
```

All fixtures use **synthetic** identities. No scanner/network execution
is required and these tests make no assertion about detection accuracy.

## Next implementation boundary

Build one normalization adapter on the existing scanner ingestion output,
not a second scanner parser. Its inputs are the immutable report bytes,
Rust ingest/audit receipts, pinned scanner execution metadata, and
explicit rule applicability evidence. It produces execution/coverage/
candidate records as a projection, not as another source of authority.

Then freeze a **new prospective evaluation cohort** before invoking scanners.
Keep scanner runs out of ordinary commit CI; run fast schema and relational
tests there. Behavioral reproducibility and weakness-family sensitivity must
be established separately before computing accuracy estimates.
