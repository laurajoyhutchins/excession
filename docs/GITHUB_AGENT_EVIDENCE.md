# GitHub-native Excession evidence

The GitHub connector can inspect workflow runs and download their artifacts.
This workflow makes Excession useful from ChatGPT without a remotely hosted
Excession service. It is **advisory read-only source analysis**, not a new
source authority, execution attestation, test result, or merge decision.

## Run on a selected PR

The reusable workflow is in
.github/workflows/github-agent-evidence.yml. It also runs on Excession's
own PR opening/ready-for-review events, not every commit. From another
repository, add a small caller workflow like this:

    name: Excession evidence
    on:
      pull_request:
        types: [opened, ready_for_review]
    permissions:
      contents: read
    jobs:
      excession:
        uses: laurajoyhutchins/excession/.github/workflows/github-agent-evidence.yml@PINNED_SOURCE_COMMIT
        with:
          base_sha: ${{ github.event.pull_request.base.sha }}
          candidate_sha: ${{ github.event.pull_request.head.sha }}

Replace PINNED_SOURCE_COMMIT with a reviewed full Excession commit SHA
containing this workflow. Do not use a mutable branch or tag in a
security-sensitive caller. The reusable workflow builds Excession from the
fixed source SHA in the checked-in workflow. It executes no code from the
subject checkout. The repository's own tests and CI remain independent.

For an exact pair of retained commits, use the workflow_dispatch inputs
base_sha and candidate_sha. Both must be full lowercase 40-hex Git IDs.
Optionally supply up to 16 comma-separated tracked paths in focus and a
human-readable task for context. The workflow refuses missing commit
objects, identity mismatches and unsupported source shapes rather than
quietly analyzing the workflow's synthetic PR merge commit. For fork PRs
whose head commit is unavailable through checkout, run the workflow from
an authorized environment where both commits are retained.

## What GitHub stores

Artifact name: **excession-github-evidence** (retained seven days).

- report.json: versioned GitHub-oriented index with repository identity,
  base/candidate commit and tree, the exact Excession tool commit/tree,
  workflow run metadata, native verification status, source witnesses if
  available, heuristic test suggestions, coverage and unresolved obligations.
- verification.json: original unmodified output of Excession agent verify
  for the exact commits.
- context.json: original agent context output, when at least one eligible
  regular source path was selected.
- summary.md: short advisory summary also displayed in the workflow job summary.

Every native report is referenced by its file name, SHA-256 and byte length.
The runner independently resolves Git commit and tree IDs and refuses stale
or mismatched Excession reports. The report envelope never upgrades
semantic correctness, allows skipping unlisted tests, or authorizes merge.
GitHub workflow artifacts are retained execution outputs, not signed
attestations of their own provenance.

For ChatGPT through the GitHub connector, first obtain the PR's exact
base/head SHAs and locate the completed workflow run. List its workflow
artifacts, download excession-github-evidence, then check report.json.
Reject reports whose base/head do not match the PR being reviewed, whose
tool identity is unexpected, whose referenced native report hashes mismatch,
or whose coverage and obligations are incomplete for the task. Continue
reading GitHub source and CI evidence independently.

## Performance and limits

This is intentionally not a required PR admission check. It builds the
pinned Rust tool and runs bounded source analysis, so schedule it at PR
opening, ready-for-review or on demand rather than every intermediate
commit. Analysis uses the same Git/SQLite authority as the local CLI and
does not run the subject repository's tests, build scripts, or code.
For large repositories, source context covers at most 16 automatically
selected changed regular paths and preserves truncation information.
Missing context.json or an empty witness array is not evidence that
there are no affected files or no tests to run.

The parallel remote option uses the existing
deploy/remote-excession backend and Hatchable project. It must not be
described as live until a real authenticated Rust round trip succeeds.
