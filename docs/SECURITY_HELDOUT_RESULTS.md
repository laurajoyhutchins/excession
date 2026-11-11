# Historical source-revision security experiment: initial results

**Experiment completed:** 2026-11-11. **Execution:** [GitHub Actions #43115695557](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557), source revision `0f80dd4ebefa8e000988c7278c5a84086baf377a`.
The [frozen manifest](../benchmarks/security-heldout/manifest.json) was committed before the scans, with exact source commits/blobs, source-family labels and scanner dependencies.
All four source-revision scan jobs, the independent comparison job and the ordinary Excession verification passed.

## Controlled native behavior

The original upstream method was executed from each pinned Git blob in an isolated temporary filesystem. This is a bounded **method-level behavioral reproduction**, not execution of a full application or a network exploit.

| Upstream source | Vulnerable first-parent revision | Fix revision | Outside-root read on vulnerable | Outside-root read on patched | Benign read |
| --- | --- | --- | --- | --- | --- |
| [Starlette GHSA-v5gw-mw7f-84px](https://github.com/advisories/GHSA-v5gw-mw7f-84px) | `24c1fac62a80bb153c6548145334fc643991e35a` | `1797de464124b090f10cf570441e8292936d63e3` | observed | not observed | preserved |
| [aiohttp GHSA-5h86-8mv2-jq9f](https://github.com/advisories/GHSA-5h86-8mv2-jq9f) | `33ccdfb0a12690af5bb49bda2319ec0907fa7827` | `1c335944d6a8b1298baf179b7c0b3069f10c514b` | observed | not observed | preserved |

## Scanner observations

The scanners were not tuned in response to these source pairs. Both engines completed on all four revisions and produced audited report bytes. The Semgrep 1.145.0 rules came from `semgrep/semgrep-rules@311ca4e9ba59d700624539bf658e3d29b134ee77`; CodeQL used the action at `9418214df1f2c4b8128c770156c409013b07d6b9` with the bundled CLI and the `security-extended` query suite.

| Case | Engine | All alerts, vulnerable / fixed | Alerts in changed source file, vulnerable / fixed | CWE-22 detection by observed alert |
| --- | --- | ---: | ---: | --- |
| Starlette | Semgrep | 24 / 24 | 0 / 0 | No |
| Starlette | CodeQL | 0 / 0 | 0 / 0 | No |
| aiohttp | Semgrep | 108 / 108 | 12 / 12 | No |
| aiohttp | CodeQL | 1 / 1 | 0 / 0 | No |

The 12 Semgrep alerts in `aiohttp/web_urldispatcher.py` come from the unrelated `python/django/security/audit/django-ratelimit/missing-ratelimit` rule. CodeQL's one aiohttp alert in each revision is `py/log-injection` in `aiohttp/web_ws.py`. None of these results identifies the parent-component path-traversal behavior reproduced by the native oracle.

**Interpretation:** The experiment establishes **lack of observed detection of these two vulnerabilities under the frozen rule/query configuration**, not a general false-negative rate, scanner incompetence or proof of no vulnerability. Some baseline rules are not semantically applicable to these frameworks. An alert in a patched source file is not automatically a false positive, and equal alert counts across revisions do not imply equal semantics.

## Evidence and limitations

The [completed Actions run](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557) retains:
- [Comparison JSON and Markdown](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557/artifacts/13898377437).
- [Starlette vulnerable](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557/artifacts/13898391922) and [patched](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557/artifacts/13898043385) source-bound reports, receipts and oracles.
- [aiohttp vulnerable](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557/artifacts/13897748383) and [patched](https://github.com/laurajoyhutchins/excession/actions/runs/43115695557/artifacts/13898282539) evidence.

`security ingest` and `security audit` succeeded for both engines on all revisions; they verify preservation and source binding of *reports*, not the truth of the findings.

No application-level HTTP request was made, no arbitrary vendor tests were executed, and no production configuration or deployed reachability was examined. The aiohttp fix intentionally still permits some symlink traversal configurations; the negative oracle tests only the specified parent-component scenario. Artifacts have a 30-day retention period.

## Engineering implications

1. Keep **scanner coverage/observation** separate from **finding validity**. A known vulnerable behavior with no relevant scanner alert must remain `NOT_OBSERVED_UNDER_SCANNED_RULESET`, not `SAFE`.
2. Associate rules with **language/framework applicability and weakness family** before interpreting alert volume. Avoid counting irrelevant findings as vulnerability detection.
3. Preserve the initial pre-registration unchanged. Any new candidate-generation strategies, CodeQL suites, Semgrep path rules or semantic analyzers must be evaluated in a **new, separately frozen prospective experiment**, not scored retroactively on these same cases.
4. Expand to additional independently selected vulnerability/fix pairs, with native-oracle evidence and explicit scope, before reporting any aggregate accuracy estimates.

The result is a useful boundary for Excession: **auditable evidence transport and behavioral assessment worked, but scanner-driven candidate discovery did not detect either held-out defect**.
