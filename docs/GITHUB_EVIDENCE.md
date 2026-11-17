# GitHub-native Excession evidence

This is the reusable PR-review evidence path. It uses GitHub Actions and
workflow artifacts, separate from [on-demand agent queries](GITHUB_NATIVE_ANALYSIS.md).
It does not deploy an MCP server, database, scheduler, external worker, or API.

## For a repository already using Excession

Add this job to a GitHub Actions workflow triggered by pull requests:

```yaml
name: Source evidence
on:
  pull_request:
    types: [opened, reopened, synchronize, ready_for_review]
permissions:
  contents: read
jobs:
  excession:
    # Replace the placeholder with the full, reviewed Excession commit SHA
    # containing .github/workflows/github-evidence.yml.
    uses: laurajoyhutchins/excession/.github/workflows/github-evidence.yml@<FULL_40_CHARACTER_EXCESSION_SHA>
    permissions:
      contents: read
    # Optional: bound Excession's native source analysis.
    with:
      roots: src,tests
```

Pin the reusable workflow to a **full commit SHA**, not a branch or floating
version tag. The workflow checks out its Excession source at the same immutable
workflow commit. It builds the Rust CLI using the locked Cargo dependency
graph, checks the source Git objects without reading uncommitted edits, and
retains a GitHub Actions artifact named
`excession-evidence-v1-<head-commit-prefix>`. This SHA is **the tool's
revision**, separate from the base and candidate SHA being evaluated.

Excession's own repository also exposes the workflow on pull requests and
manual dispatch. Manual runs accept optional base/head commit SHAs; the
defaults are HEAD^ and the workflow commit. A full-history checkout is
intentional: both revisions and their trees must actually be available.
There is no token with write permission or execution of target-repository
builds/tests by the evidence adapter.

## Artifact contract

Inside the artifact:

- `github-evidence.v1.json` is the machine-readable report.
- `summary.md` is the short human-readable CI summary.
- On refusal, `failure.json` replaces the report; there is no
  success-shaped placeholder.

The JSON contract is
[`contracts/agent/github-evidence.v1.schema.json`](../contracts/agent/github-evidence.v1.schema.json).
Check `schema` before consuming; major changes use a new schema and artifact
name. The top-level report contains repository identity; exact base/head
commit and tree OIDs; up to 16 Git-authenticated source witnesses; separate
verification and context coverage; suggested tests marked
`selection_verified: false` (either `PATH_NAME_HEURISTIC` or
`EXPLICIT_FOCUS_TEST`; explicit focus is not completeness verification); producer source commit; Excession's
original report IDs; the GitHub workflow run ID; and sorted unresolved
obligations. The entire bounded `agent verify` report is embedded so that
impact candidates, native/source witnesses, receipts, and original coverage
are not lost in translation.

`report_sha256` is SHA-256 of the canonical UTF-8 JSON object with that
field absent: sorted keys, compact separators, non-ASCII text preserved.
It detects a changed envelope but is **not** a signature or attestation.
An artifact acquired through GitHub has GitHub's run identity and retention
context, which should be checked separately. A run ID alone does not prove
tool execution or source semantics.

## Read through the GitHub connector

Open the exact workflow run for the target commit. List its artifacts,
locate the `excession-evidence-v1-…` artifact, and download it using the
existing GitHub connector. Parse `github-evidence.v1.json` and verify:

1. `schema` matches the consumer's supported contract.
2. `repository`, base/head commit and tree IDs, and
   `producer.source_commit` match the intended analysis.
3. `source_witnesses` contain the expected Git blob identities.
4. `coverage` and `unresolved_obligations` are interpreted before
   suggesting any test, approval, or merge.
5. The original `verification` and `context` entries remain available
   for audit.

The workflow runs no independent tests or builds for the analyzed repository.
A green Actions result means that evidence was produced and archived. It
does **not** mean a candidate is correct, all dependencies were discovered,
all relevant tests were selected, or the change is safe to merge.

## Deliberate limitations

`agent context` accepts up to 16 regular tracked source paths. The adapter
selects changed paths from Git's `--no-renames` diff and records partial coverage
when more are present, when a path uses the CLI's comma delimiter, or when
only deletions/submodules/symlinks are changed. It does not substitute a
heuristic source witness for an unsupported path. Underlying Excession
`agent verify` may also truncate its 64-item candidate arrays; the totals and
explicit obligations remain in the full verification report.

The adapter intentionally refuses if Git/Excession source identities disagree,
the actual change count disagrees with Git, JSON is malformed, source blobs
do not match, or Excession unexpectedly reports merge readiness. Failures
retain a diagnostic artifact and fail the job. Unknown coverage stays unknown.

The Linux CI job compiles Excession from the chosen commit, which costs more
than installing a prebuilt binary but does not rely on mutable release assets.
A future optimization can use signed provenance-bound binaries without
changing the report schema.
