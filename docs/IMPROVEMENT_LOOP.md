# Evidence-bound improvement loop

The improvement coordinator is language-neutral. It takes independently
generated, Git-bound candidate observations from supported producers,
normalizes **source paths and symbols**, and compares exact revision evidence.
The first producers are:

- `scip`: source-bound SCIP symbol references (Rust indexing demonstrated;
  other SCIP languages are admitted only where real symbol observations exist)
- `indexed`: Excession's existing Python/JavaScript module dependency-cycle
  and same-language duplicate-body-fingerprint candidates

Candidate reports are never proof of semantic equivalence, dead code,
completeness, or safe test elision. No producer's missing edge establishes a
symbol as unused.

## Indexed Python or JavaScript repositories

First index each exact revision using Excession's existing static index
with language-appropriate roots, then plan and evaluate the results:

```sh
excession index --repo /work/repo --db /tmp/evidence.sqlite \
  --name application --rev BASE_SHA --roots src

excession improve plan --producer indexed --db /tmp/evidence.sqlite \
  --name application --rev BASE_SHA > /tmp/improve-plan.json

# After a separately produced and committed candidate:
excession index --repo /work/repo --db /tmp/evidence.sqlite \
  --name application --rev CANDIDATE_SHA --roots src

excession improve evaluate --producer indexed --repo /work/repo \
  --db /tmp/evidence.sqlite --name application \
  --base BASE_SHA --candidate CANDIDATE_SHA \
  --plan /tmp/improve-plan.json
```

The indexed producer detects supported Python and JavaScript relationships.
It can *propose* a consolidation but has no automatic edit producer in this
workflow. `improve trial --producer indexed` refuses: never silently substitute
a Rust-specific AST tool or a regex rewrite for another language.

## SCIP producer

SCIP reports and exact source identities are supplied by their existing
language-native indexers; the generic planner never translates source text
or invents cross-language call semantics.

### Plan on a clean, pinned checkout

Index Rust with the existing native tools in an isolated, trusted environment.
The SCIP JSON must be decoded by the official `scip print --json` producer,
with `project_root` matching the source checkout.

```sh
excession refactor scip-suggest --repo /work/base --rev BASE_SHA \
  --index /tmp/base.scip.json --source-prefix core > /tmp/base-native.json

excession improve plan --repo /work/base --rev BASE_SHA \
  --index /tmp/base.scip.json --source-prefix core > /tmp/improve-plan.json
```

`improve plan` regenerates the Git-bound producer report, retains exact
`candidate_id`, source locations and producer evidence, and orders native review
candidates before likely crate-root module-declaration candidates. It does not declare any
candidate dead or automatically removable. The report contains a stable
`plan_id` binding the complete evidence and ordering.

### Preview or apply the supported Rust extraction

An explicit trial selects one refactor hypothesis, then derives eligible Rust
function targets from its exact SCIP symbol references. The existing
`extract-shared-module` recipe still checks actual top-level syntax, public
visibility, local references and byte ranges. An ambiguous selection is not
silently resolved by arbitrary ranking.

```sh
excession improve trial \
  --repo /work/base --rev BASE_SHA \
  --index /tmp/base.scip.json --report /tmp/base-native.json \
  --plan /tmp/improve-plan.json --candidate NATIVE_CANDIDATE_ID \
  --ast-grep /opt/pinned/ast-grep --source-prefix core \
  --trusted-local true
```

The default is a read-only preview. Supply `--output /absolute/new/worktree`
to apply the *same* validated plan in a new detached worktree. If multiple
syntactically supported targets remain, use `--symbol exact_function_name`
to narrow the candidate; a missing or ambiguous action still refuses. The
native plan ID is checked across preview and application.

`--trusted-local true` explicitly authorizes execution of the installed
ast-grep binary. It is **not** a sandbox. Run unfamiliar repositories in an
external isolated worker. These trials do not run Cargo, execute tests, commit
the generated code, or assert that extracting a function actually improved
the architecture.

## Compare a candidate after a supported transformation

Use the existing `excession refactor rust-assist`,
`extract-shared-module`, or `rust-repair` for a pinned, bounded change.
Follow the relevant documentation for preview and detached-worktree application.
Commit the candidate in its isolated worktree before comparing. Regenerate a
**fresh** rust-analyzer SCIP index for that exact candidate checkout.

```sh
excession improve evaluate \
  --base-repo /work/base --candidate-repo /work/candidate \
  --base BASE_SHA --candidate CANDIDATE_SHA \
  --base-index /tmp/base.scip.json \
  --candidate-index /tmp/candidate.scip.json \
  --plan /tmp/improve-plan.json --source-prefix core
```

Both commits must be present in both checkouts. The native index roots must
match the respective checkouts. The original plan is recomputed and checked
against its full content ID. Git's exact change inventory must match the native
source commit/tree identities.

The comparison uses *source path pairs*, not `candidate_id`: source blob
changes legitimately cause native content IDs to change. It records observed
reciprocal SCIP pairs removed or introduced and whether the disappearance
touches an actual changed Git path. Unchanged paths losing relationships are
inconclusive. Fewer indexed documents or more unresolved references are also
inconclusive. This is a conservative coverage heuristic, not a completeness
proof.

`STRUCTURAL_RELOCATION_OBSERVED_UNVERIFIED` is emitted when a normalized
candidate finding disappears while a new language-recognized source file is
introduced. Moving a Rust function, Python definition, JavaScript module or
another recognized source location can erase observed graph relationships
without eliminating an implementation. `introduced_source_paths` records that
evidence instead of counting it as debt removal.

`STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED` means normalized findings decreased
without introduced source files or those specific coverage regressions. It still does **not** establish semantic equivalence, dead code,
preserved public API, fewer implementations, or source-to-binary provenance. Even this outcome always includes open
obligations, `ready_to_merge=false`, and
`automatic_deletion_authorized=false`.

For behavioral acceptance, independently run the relevant native build, tests,
public API checks, and fresh source analysis. Excession's existing
`agent verify` retains coverage and receipt-authenticity obligations. The
improvement coordinator must never treat a plain JSON receipt as independent
execution attestation.

## Pinned historical acceptance: why the classifier is conservative

The first real native run on historical Excession (pre-#161) used a
rust-analyzer SCIP index and an existing two-function extraction. Independent
Cargo compilation, the selected regression test, API differential and a fresh
SCIP run are exercised by the historical replay harness. The evidence
evaluation observed:

- 61 reciprocal source pairs at the base and 60 at the candidate
- one additional Rust source module
- regressed index coverage
- `INCONCLUSIVE_EVIDENCE_CHANGE`, with merge and automatic deletion refused

[Native replay run and archived receipts](https://github.com/laurajoyhutchins/excession/actions/runs/43706274651)
are pinned evidence for this negative case. A smaller reciprocal-reference
count is not itself maintenance savings, even when the refactor compiles.

## Next implementation boundary

The narrow native extraction trial is implemented, but independent
build/test/API verification and an objective maintenance-surface measurement
are still missing from the coordinator. The historical Excession native test
now runs a real SCIP plan, trial and before/after evaluation as an advisory
acceptance fixture. Future transformations should demonstrate eliminated
implementation ownership, not merely moved modules.

Only after reliable execution attestation, coverage and compatibility checks
should the coordinator emit an auditable candidate PR. Unattended merge is not
authorized by these structural measurements.

Avoid a second semantic parser, graph authority, custom regex rewriting
engine, or new persistent evidence store. Any new edit producer must declare
a language, supported action, native oracle, public-contract checks, and
independent verification capability before it is allowed to mutate source.
