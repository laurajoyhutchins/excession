# GitHub-native Excession analysis

Excession needs no GCP, long-running server, custom ChatGPT MCP connection,
extra service secret, or repository webhook for ordinary public-repository
analysis. Use GitHub issues as authorized requests, GitHub-hosted Actions as
ephemeral workers, Git commit and tree hashes as source identity, and workflow
artifacts as the report transport.

The request workflow is [.github/workflows/github-native-analysis.yml](../.github/workflows/github-native-analysis.yml).
It reuses the actual Rust CLI, not an approximation or generated summary.
Existing acceptance and benchmark workflows remain independent.

## From a ChatGPT conversation

The already-connected GitHub integration supports creating an issue, reading
workflow runs, listing artifacts, and downloading an artifact. It cannot
currently invoke workflow_dispatch directly, so a dedicated issue title is
the chat-native transport.

1. Find the exact 40-character Git SHA of the commit you want analyzed.
2. Create an issue in laurajoyhutchins/excession with the **exact** title
   `Excession analysis request`, authored by the repository owner.
3. Put **only** this JSON in the issue body (a fenced JSON block is also
   supported). This is an example; replace the revision with the target commit:

```json
{
  "schema": "excession.github-analysis-request.v1",
  "repository": "laurajoyhutchins/excession",
  "revision": "0df408fa1d0836dfac6c7f2ba93d305100d9ddfb",
  "operation": "locate",
  "task": "Find the source that ranks impact witnesses",
  "limit": 16
}
```

4. GitHub starts the **Excession agent analysis (GitHub-native)** workflow
   from the trusted default branch. Find the run whose event is `issues`,
   not an unrelated test or benchmark.
5. Inspect the run's result and download its
   `excession-analysis-<run-id>` artifact. The exact files are
   `receipt.json`, `report.json` (only on success), and `summary.md`.
6. Verify `receipt.status == "COMPLETED"`, its Git and report identities,
   and the report's native `coverage` and unresolved obligations. A successful
   Actions job never establishes semantic correctness or safe-to-merge.

A failed validation step creates no report. A native CLI refusal produces a
`REFUSED` receipt and no successful report.

A manual fallback is available through the workflow's `workflow_dispatch`
form, restricted to the same owner account. Do not interpolate untrusted
issues into shell commands or workflow expression arguments.

## Operations

| Operation | Additional fields | Native command |
| --- | --- | --- |
| `locate` | `task` (3–4000 characters) | `excession agent locate` |
| `context` | `focus` (1–16 relative Git paths), optional `task` | `excession agent context` |
| `verify` | `base` (full 40-character Git SHA) | `excession agent verify` |

All requests require the schema, exact allowlisted repository, lower-case
full `revision` SHA, and operation. `limit` defaults to 16, maximum 64.
An unknown field, branch name, tag, repository, Git path traversal, or
unsupported operation causes a refusal. `verify` uses `revision` as the
candidate commit and requires `base`.

Public source repositories are registered in
[config/github-analysis-targets.json](../config/github-analysis-targets.json).
Adding a new public repository is a reviewed source allowlist change, not an
issue parameter granting arbitrary cloning. No repository code, hooks, tests,
build script, or shell fragment from the requested checkout is executed.
The analysis worker itself is compiled from the trusted Excession workflow
checkout at the Actions revision.

## Evidence and retention

The receipt records the request, producer tool commit and version, workflow
run ID, pinned source tree, native schema, report digest and advisory status.
The report is the original native Excession JSON. It is accepted only when the
schema and commit/tree identities match Git. The receipt explicitly records
`safe_to_skip_unlisted_tests: false`; Excession's analysis remains advisory.

GitHub-hosted standard runners are used with read-only `contents` permissions
and no persistent cloud identity. Rust dependency compilation is reused by
the existing `Swatinem/rust-cache` action. Native SQLite caches and Git clones
are isolated to the individual job, never published as authoritative evidence.
The artifact retention is 30 days. For longer-term evidence retention, archive
the manifest and native report in a separate reviewed publication workflow;
a cache hit or expired workflow artifact must not replace source provenance.

This is **request → compute → artifact → inspect**, not a synchronous HTTP
service. Queue and build latency are expected. Add remote workers only after
measuring a genuine interactive need.

## Security checks

The workflow runs from the default branch for issue events; PR code cannot
replace its request validator. Only GitHub issues authored by the exact owner
account, with the exact title, are admitted. Manual dispatch is owner-only.
All other issues produce no analysis job. Checkout credentials are not persisted
and the workflow has no write permissions or cloud credentials.

Tests: `python3 -m unittest discover -s tests -p test_github_analysis.py -v`.
The request validator rejects duplicate JSON keys, unexpected arguments,
foreign sources, non-commits, invalid focus paths, and identity mismatches.
The analysis process is bounded to 240 seconds, and reports are capped at 8 MiB.
Any source fetch or producer failure is a refusal, not a substitute algorithm.

The older container/Cloud Run draft PR #247 is not required. This design
does not depend on the existing Hatchable-to-GCP bridge.
