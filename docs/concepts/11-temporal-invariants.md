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
  identified as source are considered. First-parent merge commits, empty
  scopes and source changes with more than 24 files are excluded and counted.
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
