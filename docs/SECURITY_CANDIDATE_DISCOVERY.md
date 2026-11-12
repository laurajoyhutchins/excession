# Security candidate discovery contract (v1)

Issue: [#128](https://github.com/laurajoyhutchins/excession/issues/128). Stacked on [PR #87](https://github.com/laurajoyhutchins/excession/pull/87).

The historical held-out run in PR #87 remains **frozen**. Neither the Starlette nor aiohttp examples from that run may be used to tune rules and then counted as independent validation.

## Separation of concerns

1. **Coverage** is a source revision × language/framework × weakness family × scanner ruleset. Record whether that combination was actually scanned and whether the ruleset claims applicability. A scanner producing no alerts is not proof of safety.
2. **Discovery** produces source-bound candidates, not vulnerabilities. A candidate must include a pinned Git commit and blob, exact rule/ruleset provenance, report digest, source location, weakness family and an applicability basis.
3. **Assessment** separately replays report ingestion, source matching and, where possible, independent behavior. Only the assessment layer may assign `REPRODUCED` or `NOT_REPRODUCED`, with an evidence reference. Do not infer either from an unchanged alert count across a patch.
4. **Prospective evaluation** freezes an independent cohort and configuration before running scans. Include relevant negative controls, changed-source and unrelated-alert controls, scanner failures, and explicit unsupported cases.

## Candidate states

`NOT_SCANNED`: no completed scan for this source/ruleset; `UNSUPPORTED`: scanner or framework not supported; `NO_CANDIDATE`: completed applicable scan without an alert for this source; `CANDIDATE`: applicable, evidence-bearing alert; `INCONCLUSIVE`: missing or ambiguous applicability, evidence or execution. The `disposition` field is orthogonal and defaults to `UNJUDGED`.

The schema intentionally requires an explicit source even for non-candidate records. Aggregate coverage should be a separate record keyed by repository/commit and ruleset, not a fabricated source location.

## Invariants

- Validate Git commit/blob hashes and report SHA-256 before storing a candidate. A source path must be repository-relative, never absolute or parent-traversing.
- Rule applicability is a *claim with evidence*, not guessed from an alert's presence. Framework-specific rules must not silently apply to unrelated frameworks.
- Stable identity should hash canonical source commit/blob, rule/ruleset identity and normalized source span. Identity across revisions requires semantic mapping evidence, not equal line numbers.
- Duplicate alerts can share one candidate identity; multiple scanner reports are separate provenance observations.
- No generalized accuracy metrics until the prospective cohort, scanner coverage and independent behavioral truth are sufficiently defined.

## First implementation slice

Build a pure adapter for existing Semgrep JSON and CodeQL SARIF using their own rule metadata, preserving raw reports unchanged. It should fail closed on incomplete coverage and emit versioned candidate/coverage records. Use deterministic fixtures and sub-30-second contract tests on ordinary commits. Full scanners run only via manual/scheduled experimental workflows.

**This PR supplies the contract and fixtures only.** It does not claim an implemented adapter or an improved detection rate.
