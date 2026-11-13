# External pytest change-impact validation (2026-11-13)

## Experiment and independent control

An independently maintained real repository: https://github.com/pytest-dev/pytest

- Parent: `bc14038c406563b18a03e15453a10b9eee969e42`
- Head: `dcd0e636b4ce69021a9f607b060ea0056c339fd2`
- Changed paths: `src/_pytest/nodes.py` and `testing/test_nodes.py`
- Bug: `Collector.repr_failure` indexed the first exception argument even when `CollectError` had no arguments; the historical fix returns `str(excinfo.value)`.
- Reproduction: a probe running against the real upstream source raises `IndexError: tuple index out of range` before the fix and exits successfully afterward.

The executable experiment is in
[the independent pytest workflow](../../.github/workflows/pytest-independent-impact.yml).
It compiles the Rust Excession CLI and downloads exact pinned pytest Git objects.
The oracle in [impact_heldout.py](../experiments/impact_heldout.py)
uses Git and CPython AST, not Excession's SQLite dependency table, to derive
the literal in-tree Python import graph for both revisions.

## Results

| Baseline or observation | Unique files |
| --- | ---: |
| Changed files from Git diff | 2 |
| Direct importing files from independent AST | 31 |
| Full transitive affected files from independent AST | 151 |
| Excession head-revision affected files | 151 |
| Missing Excession files relative to independent oracle | 0 |
| Unexpected Excession files relative to independent oracle | 0 |
| Oracle affected paths under `testing/` | 108 |

Within the `src,testing` scope, the oracle observed **238 Python modules,
866 static import edges and 14 unresolved import sites**. The 108 paths
under `testing/` are candidate source files, **not 108 proven relevant
runnable tests**. The graph makes no claim of behavioral dependency or
safe test skipping.

## Fault exposed in Excession itself

The first run revealed that Excession exhausted its 512-effect limit on
alternative base-revision paths and did not inspect the head revision
sufficiently. It returned **0 of 151** independently expected
head-revision paths. [Initial run 43483964174](https://github.com/laurajoyhutchins/excession/actions/runs/43483964174).
That run incorrectly had an overall success conclusion because the oracle
evaluation only printed `MISMATCH` instead of failing the step.

The repaired Rust traversal prioritizes distinct dependent files, interleaves
base/head results, then emits alternate path evidence only with remaining
capacity. The workflow now requires `MATCH` or fails. Its behavioral
regression probe is also a required step, not a silently permitted failure.

[Validated run 43486404246](https://github.com/laurajoyhutchins/excession/actions/runs/43486404246):
the independent source oracle reports `MATCH`, and the old source
fails (exit 1 with `IndexError`) while the repaired upstream source passes
(exit 0). The branch also adds a Rust regression test using 300 importers
plus repeated alternative paths to exercise the finite result budget.

## Cost and limitations

| Metric | Observed on a successful Ubuntu-hosted run |
| --- | ---: |
| Fresh end-to-end Excession impact invocation | 3.85 seconds |
| Three repeated invocations | 3.84–3.85 seconds |
| Peak measured process RSS | approximately 38 MiB |
| SQLite index bytes | 6,266,880 |
| JSON impact output | approximately 1.07 MB |

The measurements include CLI execution and index work but exclude building
Excession and fetching the repository. They do not account for the entire
runner process tree. An earlier successful run measured 3.0–3.2 seconds.
Neither run is a controlled speed comparison against the independent AST
oracle, which performs additional independent verification and uses different
subprocess patterns. The 1 MB JSON output warrants optimization.

**Established:** The source-bound candidate impact set agrees exactly with
an independent static oracle on a substantial upstream repository. The
independent experiment exposed a severe Excession traversal defect and
provided a reproducible check of the repair. It also reproduced a real
historical pytest behavior change.

**Not established:** Excession did not find additional paths beyond the full
independent AST analysis. Candidate imports do not prove runtime effects;
paths under `testing/` do not certify test selection. We have not shown
that a coding agent avoids a mistake that a full native dependency analyzer,
CodeQL, or a real test-impact tool would miss. The reported cost is acceptable
for advisory review on this example, but not a proof of superior efficiency
or broad scaling.

Machine-readable receipts are retained in the GitHub Actions run artifacts.
Review the runnable workflow, source commit IDs, raw independent oracle
comparison, timing outputs, regression logs, and executable digest rather
than relying on the workflow badge alone.


## Stronger native test-selection control and agent-receipt check

A controlled one-file implementation change reintroduced the historical
`CollectError` crash without changing `testing/test_nodes.py`. Excession and
the independent AST oracle both selected the unchanged test source in the
faulty revision: **152/152** affected paths. The new test was not chosen
by looking at Excession's report. This is a seeded agent-style defect, not
an incident involving an independently operating agent.

A direct third-party baseline used `pytest-testmon==2.3.0`, a mature
dynamic test-selection plugin, with an actual executed-test baseline
*before* checking the injected regression. In the passing
[external run 43491166705](https://github.com/laurajoyhutchins/excession/actions/runs/43491166705):

| Measured step | Wall time | Reported process RSS | Exit |
| --- | ---: | ---: | ---: |
| testmon baseline (focused historical test) | 1.40 s | `52.6 MiB | 0 |
| testmon unchanged-source check | 0.28 s | `39.8 MiB | 0 |
| testmon changed-source regression test | 1.11 s | `52.2 MiB | 1 (expected `IndexError`) |
| Excession complete impact, cold | 2.85 s | `37.7 MiB | 0 |
| Excession complete impact, repeated | 2.78–2.83 s | `37.8 MiB | 0 |

testmon also caught the defect, so **Excession does not outperform
native executed-test selection on this witness**. Importantly, these
timings are not comparable operation-for-operation: testmon runs one
prewarmed focused test, while Excession traverses repository-scope static
dependencies and retains source-bound candidate explanations. The
full testmon setup and the Rust build are excluded from the command timings.

The experiment also runs `excession agent verify` with a caller-supplied
failed-test receipt built from the actual behavioral witness at the
faulty commit. Excession reports `REPORTED_FAILURE`, leaves
`ready_to_merge=false`, and refuses the same receipt after changing
its source commit. These are source-binding checks, **not**
cryptographic producer attestation. The passing-test receipt from the
fixed revision is separately tested against both the correct and the
incorrect candidate. Correct-source import must preserve open
obligations; applying that pass to the broken candidate must fail.

The real differentiation demonstrated is therefore **revision-bound,
auditable evidence and conservative agent integration**, not superior
impact recall versus full AST or mature test-selection tools. This
can prevent accidental cross-revision receipt reuse *if* the coding
agent or its orchestrator honors the verification refusal.
