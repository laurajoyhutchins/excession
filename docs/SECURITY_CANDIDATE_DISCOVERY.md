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

## Projection adapter: implemented

[The projection adapter](../tools/project_security_discovery.py) consumes
the **unchanged raw scanner report**, the **original Rust ingest SQLite file**,
the Rust ingest receipt, and the independently replayed Rust audit receipt.
It does not reimplement the scanner parsers or perform source authentication;
Rust ingest/audit must first run on the exact checked-out Git source.

The adapter additionally consumes:

- A JSON `--config` object with
  `schema=excession.security.projection-input.v1`,
  `repository`, `scanner`, `ruleset`, `scope`,
  `execution_evidence_refs`, and `coverages`.
  The scanner/ruleset/scope entries use the same shapes as the scan schema.
  Each coverage entry supplies `target`, `claim`, `rule_ids` and
  `evidence_refs` in the coverage schema's shapes.
- Exact `--ruleset-configuration` bytes whose SHA-256 equals the
  `ruleset.configuration_sha256` recorded in the config. This binds
  the supplied configuration *descriptor* to its digest. **It does not
  independently prove that a scanner executed those bytes.** Scanner
  execution provenance remains an input assertion and must be checked
  against the original job's trustworthy logs/artifacts.

Example for a completed **Semgrep** ingest. For CodeQL, set
`scanner.engine` in the config to `codeql` and supply its own
SARIF, SQLite database, ingest receipt and audit receipt:

```sh
python3 tools/project_security_discovery.py \
  --config scan-configuration.json \
  --ruleset-configuration scanner-selection.json \
  --report semgrep.json \
  --database semgrep.sqlite \
  --ingest-receipt semgrep-ingest.json \
  --audit-receipt semgrep-audit.json \
  --output projection.json
python3 tools/validate_security_discovery_contract.py projection.json
```

The output is exactly `{scan,coverages,candidates}`, and passing the
relational validator is a required part of projection. The adapter checks
the raw report SHA-256, report length, the Rust NUL-delimited run ID,
candidate logical keys and evidence IDs, all SQLite row counts and
foreign keys, the separately replayed audit receipt, and exact raw
report-result correspondence. Original SARIF code flows remain in
the original report and SQLite, not a newly interpreted model.

**No scanner alert is dropped for lack of declared rule applicability.**
An unmatched rule becomes a source-file-scoped
`SCANNED_APPLICABILITY_UNPROVEN` coverage record, with
`UNMAPPED_SCANNER_RULE` family. Framework-specific rules can be recorded
as `SCANNED_NOT_APPLICABLE` only when the caller explicitly supplies
that claim and its supporting references. Source-bound results remain
`UNVERIFIED_STATIC_CANDIDATE`; none is adjudicated by this adapter.

The first adapter version deliberately handles **completed, single-language
scans**. It refuses incomplete report/receipt joins, inconsistent source
scope, ambiguous coverage mappings and duplicated original results.
Multi-language applicability, rule-pack execution attestation and
cross-revision semantic identity are future work.

### Actual artifact replay (separate from prospective validation)

A one-time [archived-run replay #43132217257](https://github.com/laurajoyhutchins/excession/actions/runs/43132217257)
successfully projected the **actual historical aiohttp reports** from
[held-out source run #43115695557](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557):

- **Semgrep:** 108 audited candidates, including 80 Django-specific
  findings classified as framework-inapplicable (79 rate-limiting
  and 1 HTML formatting) and 28 retained as applicability-unproven.
- **CodeQL:** 1 audited `py/log-injection` candidate, retained with
  unproven applicability to the recorded target.
- **CWE-22:** neither scanner supplied a candidate for the known
  parent-traversal behavior. Both receive an explicit
  `SCANNED_APPLICABILITY_UNPROVEN` coverage record, not a safe verdict.

The [replayed JSON output and configuration inputs](https://github.com/laurajoyhutchins/excession/actions/runs/43132217257/artifacts/13905108716)
are retained as an Actions artifact. They demonstrate compatibility
with actual scanner output, **not prospective detection accuracy**.
The replay used metadata from an already completed scan, so it cannot
be counted as a new held-out detection experiment.

The temporary replay workflow was intentionally removed after success
to avoid a permanently wired CI dependency on 30-day artifact retention.

## Fast verification

```sh
python3 -m unittest discover -s tests -p test_security_candidate_contract.py -v
python3 -m unittest discover -s tests -p test_security_projection.py -v
python3 tools/validate_security_discovery_contract.py tests/fixtures/security-discovery.v1.json
```

Synthetic tests exercise both report formats and SQLite ingest identities;
they do not invoke scanners or establish source-level vulnerability behavior.
The real replay above verifies compatibility on actual archived scanner
outputs. Ordinary commit CI runs the pure tests only.

## Next independent experiment

Freeze a **new** prospective evaluation cohort before invoking or tuning
scanners. Preserve the historical Starlette/aiohttp negative result.
Keep scanner execution and independent behavior checks out of fast commit CI.
No precision, recall, reachability or CVE-confirmation claim follows
from either a successful projection or the archived held-out experiment.
