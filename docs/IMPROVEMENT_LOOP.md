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

## Language-independent native edit replay

The core has a bounded, **language-independent** byte-edit replay path.
Language-native tooling can propose exact source edits, which Excession then
replays against authenticated Git blobs without interpreting the syntax itself.

The producer supplies an `excession.improvement-native-edits.v1` object:
`schema`, `native_producer`, `source_commit`, `source_tree`,
`improvement_plan_id`, `candidate_id`, `edits`, and an `edit_plan_id`
computed from the canonical unsigned JSON object. Each edit includes `path`,
`original_blob`, `start_byte`, `end_byte`, and `replacement`.
All offsets refer to **original Git source bytes**, not previous edits.

```sh
excession improve replay --repo /work/repo --rev BASE_SHA \
  --plan /tmp/improve-plan.json --edits /tmp/native-edits.json
# Explicit application in a NEW detached worktree, never the original:
excession improve replay --repo /work/repo --rev BASE_SHA \
  --plan /tmp/improve-plan.json --edits /tmp/native-edits.json \
  --output /work/new-candidate
```

An unsigned native edit plan **does not authenticate its named producer**.
Source authenticity means exact Git blobs and source-bound candidate paths are
checked, not that Excession independently observed the compiler deciding the
edit is safe. The preview and applied report retain
`NATIVE_PRODUCER_AUTHENTICITY_UNVERIFIED`, independent build/test/API and
provenance/REA obligations, and `ready_to_merge=false`.

The initial narrow contract allows only existing, regular Git-tracked UTF-8
source files already present in the chosen candidate. It refuses traversal,
symlinked source blobs, stale Git hashes, out-of-candidate edits, oversized
plans or sources, ambiguous byte ranges, ineffective edits, and reused or
in-checkout worktree destinations. New files or non-UTF-8 source require
explicitly designed extensions, not silent coercion.

This replay API works for a native Python or JavaScript edit just as for Rust.
**It does not generate that edit.** The language-specific producer must supply
its own legitimate syntax/semantic action, including configuration-dependent
usage and public API checks. Tests demonstrate the shared replay mechanism
using deliberately *untrusted* edit plans, not automatic safe deletion.

## Native TypeScript unused-import producer

The first edit producer using the shared replay contract is the existing,
pinned TypeScript 5.8.3 language service. It works on JavaScript and TypeScript
source files. Excession does not implement JavaScript syntax, dependency
resolution or import-rewriting rules in Rust.

```sh
node core/adapters/improve_typescript.cjs \
  --repo /work/repo --rev BASE_SHA --source src/main.ts \
  --symbol unusedBinding \
  --plan-out /tmp/improve-plan.json \
  --edits-out /tmp/native-edits.json

excession improve replay --repo /work/repo --rev BASE_SHA \
  --plan /tmp/improve-plan.json --edits /tmp/native-edits.json

excession improve replay --repo /work/repo --rev BASE_SHA \
  --plan /tmp/improve-plan.json --edits /tmp/native-edits.json \
  --output /work/new-candidate
```

The producer reads the source blob from Git, not from the working tree. It
uses TypeScript's *unused identifier* diagnostic and its own
`unusedIdentifier` code fix. To keep the first experiment bounded, it accepts
only **pure deletions of an unused binding in an existing multi-binding named
import**, with no rewriting of the import module or side-effect imports.
Ambiguous fixes, whole-import deletion, arbitrary code changes, non-UTF-8
source, or nonexistent Git source are refused. Offsets are converted from
TypeScript UTF-16 positions to Git UTF-8 source bytes.

This is a single-file diagnostic with deliberately incomplete project and
configuration coverage. An unused binding diagnosis does not establish the
absence of side effects, dynamic uses, transpiler behavior or public-contract
regressions. Its output continues to require independent compiler, runtime,
type-checking, configuration, API, and provenance checks.

The CI acceptance fixtures exercise both languages, check that the used
binding and module initialization survive, compile TypeScript independently,
run the resulting code, and reject stale source identities and unsafe cases.
The native tool never commits the edited worktree or sets `ready_to_merge`.

### Held-out independently authored repository acceptance

The [third-party held-out acceptance](https://github.com/laurajoyhutchins/excession/actions/runs/43755236879)
uses **[piyushrav1/ai-interview-v.01](https://github.com/piyushrav1/ai-interview-v.01)**,
pinned at `78c6a78b8a367c2c19398b05d39274ea3bcacadb`.
This repository is not owned by the Excession author. The selected revision
predates upstream's own [unused-import cleanup](https://github.com/piyushrav1/ai-interview-v.01/commit/dea7976fcf620fc19c9731edef06a3d294f0451c);
no synthetic unused source is inserted.

The native TypeScript 5.8.3 producer identifies the pre-existing unused
**type-only** `Turn` import in `packages/core/src/machine/types.ts`
using diagnostic `TS6196` and TypeScript's own `unusedIdentifier` fix.
The generated exact-blob edit deletes **6 source bytes (840 → 834)** from
one existing file, with **zero new modules**. Its diff matches the upstream
author's later change to that import.

The independently executed before/after checks passed for both source trees:
a full core TypeScript typecheck using the upstream tsconfig and compatible
**TypeScript 5.6.3** (separately pinned from the producer), followed by
the upstream's **Vitest core test suite** from the workspace root. The
historical upstream lock resolves TS5.9.3, which rejects the original
`baseUrl` setting before any edits; the acceptance therefore installs a
compatible compiler from upstream's declared `^5.6.0` range without
modifying the lockfile or either source tree. Separate pnpm installations
are used for each tree to prevent workspace-link cross-contamination.

The workflow archives native edit plans, the exact Git diff, compilation
and test logs, and the measurement report. Neither upstream repository nor
its default branch is modified. The result demonstrates real source reduction,
**not** universal behavioral equivalence, completeness of all configurations,
independent native-producer attestation, REA provenance, or merge authorization.

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
