# Autonomous native improvement loop

**Implemented scope:** Excession has two guarded auto-integration lanes and
one review lane. Rustfmt-only changes and graph-preserving Cargo patch updates
can integrate **only after independent exact-head CI passes**. Broader native
Clippy and dependency updates remain review-required draft PRs. There is no
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
| `native-cargo-lock-refresh` | pinned Cargo update | Patch-only, same-dependency-graph updates are auto-integrate eligible after CI; all other resolution changes go to draft review |

A successful `Verify` workflow already enforces clean Rustfmt and Clippy,
so those two producers normally find **no new work** on a healthy `main`.
They remain defensive recipes, not evidence of a self-improving system by
themselves. Native Cargo resolution can discover genuinely newer dependencies;
the separate recurring engineering investigation must generate additional
source-grounded refactoring recipes over time.

The technical notebook supplies reasoning and evidence references. It is not
parsed as untrusted executable instructions and a prose suggestion cannot
grant a workflow token permission to merge arbitrary changes.

## Running the loop from a connected GitHub client

The GitHub connector may not expose GitHub Actions `workflow_dispatch`.
The native workflows therefore also accept one **owner-authored issue-opened**
event with a fixed title: `[Excession] Run Cargo improvement` or
`[Excession] Run Rustfmt improvement`. The workflow still checks the
canonical repository, `main` and GitHub's immutable issue author ID
`219002713`; it does **not** read the issue body as code, paths or commands.
An issue by another account cannot authorize the privileged job. Existing
scheduled and explicit `workflow_dispatch` paths remain unchanged.

A successful trigger is only a start. Review the actual run, candidate branch,
GitHub evidence, full Verify results and resulting PR/merge. A no-op formatter
is a verified **absence of an eligible edit**, not a completed improvement.
If a workflow creates a draft dependency PR, it observed work but did not
satisfy automatic patch-only policy.

## Production loop

The [auto integration workflow](../.github/workflows/quality-auto-integrate.yml)
runs weekly or by manual dispatch, only on the repository's `main` ref. The
[Cargo updater](../.github/workflows/quality-cargo-update.yml) runs weekly
on a separate day, also from main. Both use the shared
[exact-head merge gate](../tools/ci/await_verified_merge.sh):

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

For Cargo's independent lane, `cargo update --manifest-path core/Cargo.toml`
may produce real `core/Cargo.lock` changes even when formatting and Clippy
are already clean. The [Cargo patch auditor](../tools/ci/cargo_patch_audit.py)
allows unattended integration only if every package name/source and dependency
edge remains identical, every changed registry version increases strictly
within its existing major/minor, checksums are consistent, and no other file
changes. Major/minor updates, added dependencies, graph changes, nonregistry
replacements, and ambiguous versions are **draft PRs**, not silent failures
and not auto-merged. The auditor is a *lockfile-structure classifier*, not a
supply-chain malware detector.

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

**Scope:** The formatter workflow maintains Excession's own
Rust sources and the dependency updater maintains its Cargo lockfile. Applying it to other repositories requires their explicit
source/native-tool configuration, required checks and separately scoped
credentials. It must not silently interpret every connected repository
as authorized for unattended modification.

**Unverified:** Neither the scheduled main-workflow path nor an unattended
merge has been exercised end-to-end until a real non-empty candidate is
produced and both exact-head workflows pass. Cargo's patch gate has explicit
negative cases, but patch-only updates may still introduce malicious code,
unsupported runtime changes, or untested regressions. Passing fixture tests is not evidence that GitHub
will permit a bot merge.

## Live operational evidence (November 17, 2026)

[Owner invocation #319](https://github.com/laurajoyhutchins/excession/issues/319)
really started [native Cargo improvement run #44154374026](https://github.com/laurajoyhutchins/excession/actions/runs/44154374026).
The workflow generated nine updates and successfully pushed commit
`4d05cd0c099366fa362555a6c720a9558bab9736` to branch
`automation/cargo-refresh-44154374026`. Its native patch-only classifier
refused automatic integration because `rmcp-macros 3.10.0 → 3.11.0` is a
minor-version upgrade; that was correct behavior.

The attempted `gh pr create` then failed with
`GitHub Actions is not permitted to create or approve pull requests`.
This is an actual GitHub repository Actions-setting restriction even though
the workflow specifies `pull-requests: write`. The runner cannot override
it and must not push directly to `main` as a fallback.

**Administrator action:** In the Excession repository's **Settings →
Actions → General → Workflow permissions**, enable
**Allow GitHub Actions to create and approve pull requests**. Keep the
independent exact-head Verify/source-evidence requirements and normal
branch protection enabled. The recovery path is to open the already-pushed
bot branch through the connected GitHub account; [draft PR #320](https://github.com/laurajoyhutchins/excession/pull/320)
preserves the full native proposal, while [PR #321](https://github.com/laurajoyhutchins/excession/pull/321)
isolates the eight patch updates pending independent verification.

Until repository permission is enabled and a real bot-created PR passes
the full two-receipt merge gate, **unattended improvement integration has not
been observed**. A candidate push is useful progress, not a successful
autonomous merge.

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
