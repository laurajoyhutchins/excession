# Autonomous native improvement loop

**Shipped scope:** Excession can continuously generate two kinds of native
source improvements on a trusted `main` checkout. Rustfmt-only changes can
be automatically integrated **only after independent exact-head CI passes**;
machine-applicable Clippy edits remain review-required draft PRs. There is no
general code rewrite or language-independent semantic-equivalence prover.

## From learned principle to active recipe

A lesson is actionable only when it has a concrete native producer,
a measurable changed-source scope, a reproducible negative or positive
acceptance test, an explicit risk class, and an evidence reference.

`quality/improvement-recipes.json` records that association. It is validated
by `tools/ci/improvement_cycle.py` and tested on every PR before installing
dependencies. The registry is **not executable arbitrary-shell configuration**.
A new entry cannot self-assign unattended integration authority: the Python
runner separately hard-allowlists only `native-rustfmt`. Unknown recipes
must be review-required. The initially admitted recipes are:

| Recipe | Producer | Action |
| --- | --- | --- |
| `native-rustfmt` | pinned cargo fmt | Apply and auto-merge *only if* all independent receipts succeed |
| `native-clippy` | pinned cargo clippy --fix | Apply and open a draft PR; no unattended merge |

The technical notebook supplies reasoning and evidence references. It is not
parsed as untrusted executable instructions and a prose suggestion cannot
grant a workflow token permission to merge arbitrary changes.

## Production loop

The [auto integration workflow](../.github/workflows/quality-auto-integrate.yml)
runs weekly or by manual dispatch, only on the repository's `main` ref:

1. Validate the recipe registry and adversarial Git-fixture suite.
2. Run the exact pinned native Rust formatter on a clean checkout.
3. If nothing changes, report **no opportunity** and exit without making
   an empty PR. If something changes, commit one candidate on a bot branch.
4. Reconstruct the edit in a **fresh detached worktree of the original base
   commit**, rerun native Rustfmt and require byte-for-byte agreement of
   every changed Git blob. Reject additions, deletions, symlinks, changed file
   modes, non-Rust paths, unrelated changes, or self-edited recipe authority.
5. Run the existing quality ratchet, native formatting check, Cargo library
   tests, and the fixture suites before publishing a PR.
6. Explicitly dispatch the existing `Verify` and `Excession GitHub evidence`
   workflows at that candidate branch. A PR created with `GITHUB_TOKEN`
   cannot be presumed to have started normal PR-triggered workflows.
7. Require **both distinct exact-head workflow runs to complete successfully**
   and confirm that neither `main` nor the PR head has advanced.
8. Merge the PR with normal GitHub branch protection and permissions, never
   with an admin bypass. If rights or checks are missing, stop and leave the
   candidate unmerged for review.

The workflow uploads its deterministic native replay receipt. The tool's own
report is deliberately `REPRODUCED_PENDING_INDEPENDENT_CI` with
`ready_to_merge=false`. Reproduction does not attest the GitHub action
or prove general semantic equivalence; the separate Actions check receipts
and the protected merge are required.

The [existing Clippy workflow](../.github/workflows/quality-improvements.yml)
is kept distinct: it generates real compiler-suggested code edits and opens
draft PRs for review. It does not run the formatting-only recipe on its own,
and it does not automatically merge diagnostic fixes.

## Safety and reliability constraints

**Mutation authority:** Only native producer commands from trusted `main`
can run with the write-enabled bot token. Neither loop has a
`pull_request_target` nor pull-request trigger.

**Source identity:** Native replay checks exact base and candidate commits,
direct parentage, Git regular blobs, unchanged recipe catalog, and exactly
reproduced changed bytes. The parentage restriction means a rebase or
concurrent main update must trigger a fresh candidate instead of reusing
stale test evidence.

**Independent verdict:** Compiler formatting and library tests run before
publication. `Verify` and `Excession GitHub evidence` must both succeed
at the exact PR head before merge. A cancelled, skipped, pending or unknown
check does not count as a pass.

**Protection:** GitHub branch-protection policy and token settings remain
outside this tool's control. If the organization forbids bot-created PRs
or bot merges, automation refuses instead of changing security settings.

**Scope:** The formatter workflow currently maintains Excession's own
Rust sources. Applying it to other repositories requires their explicit
source/native-tool configuration, required checks and separately scoped
credentials. It must not silently interpret every connected repository
as authorized for unattended modification.

**Unverified:** The scheduled main-workflow path and unattended merge have
not been exercised end-to-end until a real non-empty native formatting
candidate is produced. Passing fixture tests is not evidence that GitHub
will permit a bot merge.

## Promoting further learned improvements

For each new engineering lesson, first capture a real failure, counterexample
or expensive maintenance pattern in the notebook. Build a **language-native
producer** and a narrow acceptance fixture. Prefer deterministic source
transformations and exact replay. Start its recipe in the *review-required*
class and collect independent regression receipts on real commits. Only
consider automatic integration after the producer's authority, edit scope,
resource policy and cross-configuration checks are justified.

Do not automatically raise confidence because the number of successful
transformations, passing tests or graph edges increases. Keep failed
experiments and unsupported cases visible.

Related: [quality ratchet](QUALITY_RATCHET.md),
[transformations](TRANSFORMATIONS.md),
[verification](VERIFICATION.md),
[structural preservation receipts](STRUCTURAL_PRESERVATION.md).
