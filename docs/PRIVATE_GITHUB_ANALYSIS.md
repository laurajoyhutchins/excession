# Private GitHub repository analysis

Public Excession issue requests cannot read private Git objects. Private
repositories instead invoke the same Excession Rust analysis in their own
GitHub Actions jobs. The report and execution logs stay in the private
caller repository; no source file or Git object is uploaded to Excession.

## Install once per private repository

Create .github/workflows/excession-analysis.yml in the **private**
repository:

    name: Excession private source analysis
    on:
      issues:
        types: [opened]
    permissions:
      contents: read
    jobs:
      excession:
        if: >-
          github.event.issue.title == 'Excession analysis request' &&
          github.event.issue.user.id == 219002713
        uses: laurajoyhutchins/excession/.github/workflows/github-private-analysis.yml@PINNED_EXCESSION_SHA
        permissions:
          contents: read

Replace PINNED_EXCESSION_SHA with the full reviewed commit ID of Excession
containing the reusable workflow. A tag or branch is not an immutable
execution identity. GitHub Actions executes the reusable job **inside the
private caller repository's Actions context**. The job uses a read-only token
for the caller's Git checkout and a public immutable Excession checkout.
The workflow publishes its artifact to the private caller's run.

The GitHub integration can create an issue in that same private repository
with the exact title Excession analysis request and a JSON body:

    {
      "schema": "excession.github-analysis-request.v1",
      "repository": "laurajoyhutchins/arcata",
      "revision": "REPLACE_WITH_EXACT_40_CHARACTER_COMMIT",
      "operation": "locate",
      "task": "Find transformer sizing calculations",
      "limit": 8
    }

The GitHub issue must be authored by repository owner ID 219002713.
Supported operations are locate (task), context (focus of up to 16 tracked
relative paths, optional task) and verify (base full commit SHA, with revision
as candidate). Every request is bound to the *caller* repository: an issue
cannot choose another private checkout, command, remote, source directory,
or branch. The workflow refuses an absent commit instead of fetching an
arbitrary source from the network.

## Retrieve and verify

Find the Actions run named Excession private repository analysis associated
with the issue. Download artifact
excession-private-analysis-ISSUE_NUMBER-RUN_ID from the **private caller**
repository using the GitHub connector. It contains the same native
report.json, receipt.json and summary.md contract as the public issue
workflow, retained for seven days.

Check the request repository, exact revision, Git tree, producer commit,
status COMPLETED, report SHA-256, original native schema and coverage.
The analysis has no behavioral-test attestation or merge authority.
The runner's contents:read permission and its issue-author guard are distinct
from a proof of safety for every repository source file.

## Security model

- No GCP, service token, persistent agent, or public artifact host
- Never execute build commands, hooks, tests or scripts from the caller source
- Checkout at the trusted default branch, then analyze exact retained Git
  objects without checking out the requested candidate revision or updating
  its refs
- An owner-authored request selects one operation and exact source revision
- Tool revision must match job.workflow_sha; Excession is built from that
  immutable source with its locked Rust and TypeScript dependencies
- No repository write token, checkout credential persistence, or actions
  that comment on/modify the issue
- Fail-closed native JSON identities, explicit partial coverage, and bounded
  8 MiB output
- The adapter's source provenance is a Git observation, not a signed
  execution attestation or semantic correctness claim

The reusable job consumes standard GitHub Actions minutes. Use it on
explicit issue creation, not on every commit. A private repository with
additional policy requirements should review this workflow before calling it.

The public source workflow and its allowlist remain unchanged; see
[GITHUB_NATIVE_ANALYSIS.md](GITHUB_NATIVE_ANALYSIS.md). This path is
GitHub-mediated, not a synchronous remote MCP endpoint.
