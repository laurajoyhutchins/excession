# Temporal change-companion hypotheses

This experiment proposes **testable associations**, not verified architecture or causal rules.

`excession concepts invariant-propose` reads a pinned first-parent Git history
ending at `--rev`. It finds repeatedly modified file pairs, requires minimum
support, 70% conditional co-change (default), and at least 1.5× lift above the
independent file-frequency baseline. At most one companion is selected per
changed trigger, with a 64-rule bound and explicit omitted-rule count.

```sh
excession concepts invariant-propose --repo . --rev BASE \
  --prefix core/src --train 48 --support 3 --confidence 70 > proposed.json
excession concepts invariant-propose --repo . --rev BASE \
  --candidate CANDIDATE --prefix core/src --train 48 > prospective.json

excession concepts invariant-backtest --repo . --rev HEAD \
  --prefix core/src --train 48 --holdout 16 > heldout.json
```

For a prospective review, BASE is the training cutoff; CANDIDATE must be
a later descendant. Only modifications between BASE and CANDIDATE are
checked against hypotheses trained before BASE. For backtesting, the last
16 first-parent events are held out **before** model fitting, with no
refitting on later evidence.

Every rule provides the two exact paths and cutoff Git blobs, its cutoff
commit and tree, training exposure counts, co-change event count and
individual historical witness commits. Reports carry a canonical SHA-256
identity and a JSON Schema shape check.

## Controls and limitations

- The comparator is a **file popularity baseline** trained on the identical
  earlier events. For each triggered rule it gets exactly one prediction,
  excluding the trigger itself, so compared exposure counts are equal.
  This is not a SCIP/static call graph, test selector or LLM baseline.
- Modified files only, not adds, deletions or renames. Only file extensions
  identified as source are considered. First-parent merge commits are skipped during sampling and their number
  reported; each selected non-merge commit is diffed against its actual Git
  parent. Empty scopes and source changes with more than 24 files are
  excluded and counted.
  Evidence for unsupported languages and configurations remains unavailable.
- No claim of causation, correctness, semantic identity, or runtime
  dependency is licensed by recurring co-changes. If a predicted file is
  not modified, the report explicitly says **NOT A PROVEN BUG**.
- No independent behavioral oracle, compiler check, regression test or
  human architectural adjudication is run by this command. No concept is
  promoted into a verified model, no automatic source modifications occur,
  and absence of a candidate does not authorize skipping tests.
- Only pinned Git history is consulted. The current checkout may be dirty
  without changing the result. No source repository writes or LLM calls.
- A passing synthetic fixture shows mechanical chronology and comparator
  correctness, not a useful real-world discovery. Real heldout performance
  needs multiple external repositories and independent review.

## Why this is part of the ontology pipeline

PR #154 currently accepts LLM-authored source-backed architecture proposals.
This deterministic producer offers **temporal counterfactual questions** to
those authors: which source changes have historically required co-editing a
different path, and does the proposed responsibility explain the association?

The next value gate is to take a frozen hypothesis, identify a failing
behavioral oracle, and demonstrate that the ontology predicts the defect
before the fix or test is revealed. It must beat both the existing SCIP/impact
and mature test-selector baselines under equal budgets.

The current result is a proposal and discovery/backtest input only. See
[issue #105](https://github.com/laurajoyhutchins/excession/issues/105).

## Fair frozen-policy comparison (new)

The rule list above is a hypothesis feed and is allowed to be sparse. It is
**not** the primary performance score. A separate source-companion prediction
comparison now counts **every** eligible heldout modified source present at the
training cutoff, including triggers for which no exact historical rule exists.

The experiment evaluates six predetermined policies: global popularity
(simple baseline), same-directory popularity, exact normalized source/test
stem, conditional co-edit, recently observed conditional co-edit, and a
hybrid of these hints. A fixed first 75% of the training history fits all
policies. The last 25% of training is an internal chronological validation
window; the policy is selected there using paired improvements over the
global baseline, requiring at least eight eligible predictions and a
strictly positive improvement. Popularity wins ties. No future holdout
observation influences policy selection.

Once selected, the policy is re-fit on **all** training events and compared
on the untouched holdout at exactly one prediction per eligible modified
source. The report preserves the internal calibration candidates, selected
policy, prediction differences, non-predictions, and model-only/baseline-only
successes. Same-directory frequency is reported as an additional
more demanding comparator. The original stricter cochange-rule metric is
retained separately for backward comparability.

The name and directory heuristics are **unverified lexical hints**, not
authenticated semantic relationships. A successful co-edit prediction is not
a demonstrated architectural invariant or a prevented regression. The
strictly pinned seven-repository exploratory matrix is documented in
`.github/workflows/concepts-crossrepo.yml`; definitive generalization needs
an independently selected, later frozen cohort with no parameter tuning.
