# Six-case prospective scanner evaluation: observed results

**Protocol registered before scanning:** [manifest](../benchmarks/security-prospective/manifest.v1.json), first committed at `5ca7f543b3a631ba278a4428915d8f1332e34a88` (Git blob `210bca7047cef7d757472eec03e4fbfe83ddb8ff`).

**Original execution:** [GitHub Actions run #43149355892](https://github.com/laurajoyhutchins/excession/actions/runs/43149355892).

**Artifact-only reconciliation:** [run #43150431421](https://github.com/laurajoyhutchins/excession/actions/runs/43150431421), [machine-readable results and Markdown](https://github.com/laurajoyhutchins/excession/actions/runs/43150431421/artifacts/13913063235).

This was **not** a test for discovering unknown CVEs. It was a previously unexamined, preregistered cohort of six published vulnerability/fix pairs from three upstream repositories, across CWE-78, CWE-22 and CWE-601. The scanner was **Semgrep 1.145.0**, using the pinned `semgrep-rules` Python subtree at commit `311ca4e9ba59d700624539bf658e3d29b134ee77`. Each revision was authenticated to its Git commit and changed-source blob before scanning. No scanner rules, source cases or scoring predicates were tuned based on the run.

## Measured outcomes

| Published advisory / source | CWE | CWE-tagged same-file scanner alerts vulnerable / patched | Bounded native oracle | Observation |
| --- | --- | ---: | --- | --- |
| GHSA-rpm5-65cw-6hj4 / GitPython | 78 | 1 / 1 | Both revisions expected | General subprocess-use alert persists after fixing the unsafe-options bypass |
| GHSA-r9mr-m37c-5fr3 / GitPython | 78 | 1 / 1 | Both revisions expected | Same general subprocess-use alert on both releases |
| GHSA-9277-mp7x-85jf / Dulwich | 78 | 2 / 2 | Both revisions expected | Shell-use alerts persist after the corrected argument quoting |
| GHSA-8w8g-wq8h-fq33 / Dulwich | 22 | 0 / 0 | Both revisions expected | No relevant candidate in either release; six unrelated findings each |
| GHSA-c2m8-h5v5-343r / Tornado | 22 | 0 / 0 | Both revisions expected | No relevant candidate in either release; ten unrelated findings each |
| GHSA-pw9p-v7r6-gx4p / Tornado | 601 | 0 / **incomplete** | Both revisions expected | Frozen Semgrep parser fails on patched Tornado 6.6.0, `tornado/web.py:1732` |

**Totals:** 6/6 source pairs passed their method-level behavioral control (vulnerable adverse behavior observed, patched behavior rejected, benign operation allowed in both). Exactly **5/6** paired scanner comparisons completed without errors; **11/12** source revisions produced accepted and independently replayed Rust ingest/audit receipts. No completed pair exhibited a relevant alert that disappeared after the upstream fix.

The original experiment workflow and comparison job **failed** because Semgrep's parser produced a syntax error in the sixth pair's patched source. That is a real execution/coverage gap, not an all-clear and not a vulnerability miss to be included in a denominator of fully scanned pairs.

The artifact-only reconciliation repaired **reporting**, not scanner behavior. It examined the archived unchanged raw Semgrep error and retained the valid patched method-level oracle, yielding `SCANNER_ERROR` for that phase. It performed **no rescans and no rule or cohort changes**.

## Why the CWE-78 hits are not successful defect detection

The GitPython finding is a general `dangerous-subprocess-use` rule at the **same source line 286** in both vulnerable and patched releases. The Dulwich merge-driver findings similarly originate at the `subprocess.run(..., shell=True)` call, which remains present after the upstream fix, shifted from line 124 to 139. None establishes the particular missing validation or unquoted filename vulnerability.

Consequently, treating `same-file + matching CWE` as an exact true positive overstates performance. That criterion is recorded because it was fixed *before* the run. A stricter **post-run diagnostic** identifies **zero patch-discriminating alerts** among the five complete pairs, without retroactively changing the preregistered metric.

## Claim boundaries and reproducibility

1. The oracles execute the **original upstream method bodies**, isolated with local temporary files or subprocess mocks. They do not demonstrate a full HTTP service exploit, network reachability, arbitrary command execution, or every deployment configuration.
2. Source Git commits and blob hashes, exact scanner version/ruleset, original Semgrep JSON (including its errors), SQLite tables, Rust ingest and independent audit receipts, and per-case oracle output were retained as six GitHub Actions artifacts under the [original run](https://github.com/laurajoyhutchins/excession/actions/runs/43149355892).
3. The scanner only scanned the specified changed source file for each case, not the whole repository. Other weaknesses, code paths and dependencies were not evaluated.
4. There was no CodeQL analysis in this experiment. The earlier two-case experiment is not counted toward these six results.
5. This small, deliberately selected corpus does not support a general precision, recall, false-positive rate, or vulnerability-discovery claim.
6. Evidence artifacts have a 30-day retention period. The manifest, oracle implementation, scoring code, negative tests and this summary remain in Git; raw scanner artifacts need separate durable preservation if later verification must outlive GitHub retention.

## Consequences

- **Coverage first:** a successful command exit is not sufficient. Parser diagnostics must invalidate coverage and remain visible beside the native oracle.
- **Candidate semantics:** framework-aware rule metadata and source anchoring are useful for audit but insufficient to identify a concrete defect; compare behavior and patch-sensitive evidence independently.
- **Rule identity:** Semgrep emitted rule IDs prefixed with runner-specific absolute rule paths. Normalize those only in a future candidate-identity version, while preserving raw IDs in archived reports.
- **Future evaluation:** hold a new, independently frozen corpus for changes to candidate generation, rules or scanner versions. Do not modify this cohort and count the improvement as a prospective result.
