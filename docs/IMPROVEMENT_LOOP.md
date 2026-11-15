# Evidence-bound improvement loop

The first improvement coordinator evaluates **structural observations**, not
automatically safe deletions. It reuses existing rust-analyzer SCIP evidence,
Git identity, and refactoring primitives. This command does not run compilers,
tests or an agent, write source files, or authorize a merge.

## Plan on a clean, pinned checkout

Index Rust with the existing native tools in an isolated, trusted environment.
The SCIP JSON must be decoded by the official `scip print --json` producer,
with `project_root` matching the source checkout.

```sh
excession refactor scip-suggest --repo /work/base --rev BASE_SHA \
  --index /tmp/base.scip.json --source-prefix core > /tmp/base-native.json

excession improve plan --repo /work/base --rev BASE_SHA \
  --index /tmp/base.scip.json --source-prefix core > /tmp/improve-plan.json
```

`improve plan` regenerates the Git-bound native report, retains exact
`candidate_id` and producer evidence, and orders review candidates before
likely crate-root module-declaration candidates. It does not declare any
candidate dead or automatically removable. The report contains a stable
`plan_id` binding the complete evidence and ordering.

## Preview or apply a single supported native extraction

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

`STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED` means the reported observations
became structurally simpler without those specific coverage regressions. It
does **not** establish semantic equivalence, dead code, preserved public API,
or source-to-binary provenance. Even this outcome always includes open
obligations, `ready_to_merge=false`, and
`automatic_deletion_authorized=false`.

For behavioral acceptance, independently run the relevant native build, tests,
public API checks, and fresh source analysis. Excession's existing
`agent verify` retains coverage and receipt-authenticity obligations. The
improvement coordinator must never treat a plain JSON receipt as independent
execution attestation.

## Next implementation boundary

To make this a full autonomous debt-reduction loop, add a bounded executor that
selects one mechanically supported native action, applies it to an isolated
worktree, and requests independent build/test/API verification. The result
should become an auditable candidate PR, not an automatic merge until execution
attestation, coverage and compatibility requirements are independently satisfied.

Avoid a second semantic parser, data store, or custom Rust refactoring engine.
Existing native transformation code should be the only edit producer.
