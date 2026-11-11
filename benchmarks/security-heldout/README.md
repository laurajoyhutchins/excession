# Pre-registered external-source validity experiment

This experiment is **not** the synthetic Go calibration in `benchmarks/security-validity`.
It pins independently documented upstream security fixes (and their first parents),
both source-file Git blobs, the Semgrep rules Git commit, and the CodeQL action,
CLI version and query suite **before running either scanner**.

The two source cases are:

- Starlette GHSA-v5gw-mw7f-84px (CVE-2023-29159): a `commonprefix`
  check permits access to a sibling with a similar textual prefix; the fix
  changes to `commonpath`.
- aiohttp GHSA-5h86-8mv2-jq9f (CVE-2024-23334): the
  `follow_symlinks=True` path resolution permits parent-component escapes;
  the fix validates the normalized path before resolving symlinks.

Each vulnerable and patched source revision is checked out at its exact commit.
`tools/heldout_native_oracle.py` extracts and executes only the original
upstream path-resolution method in a temporary directory, then verifies an
outside-root read and an allowed benign read. It does not import the vendor
package, launch an HTTP service, or execute arbitrary repository tests.
A failure to establish the expected behavior **fails** the job.

CodeQL and Semgrep scan full checked-out upstream source without rule tuning
against the observed outcomes. The raw SARIF/JSON, import/audit receipts,
Git coordinates, oracle output, and tool versions are stored separately.
The comparison report must distinguish:

- Scanner report containing at least one alert in the upstream changed file.
- Scanner report with no alert in that file.
- Alert on the patched revision, **not automatically** a false positive.
- Execution failure or incomplete scanner coverage.
- Repeated alerts for one source weakness, not independent vulnerabilities.

No aggregation into precision, recall, or CVE confirmation is valid for two
handpicked cases. Neither the method-level reproduction nor the scanner alert
proves application-level reachability in a deployment.

Frozen input: `benchmarks/security-heldout/manifest.json`.
Run with: `.github/workflows/security-heldout.yml` (branch-scoped opt-in).
