# Resource budgets versus analysis completeness

Excession must separate **work performed in one invocation**, **how much is
returned in one response**, and **how much of the source has actually been
covered**. Git revision and blob identities remain authoritative. No budget
exhaustion may be reported as complete or as evidence that a relationship does
not exist.

## Implemented: configurable agent locate indexing

The native `agent locate` CLI accepts three positive indexing budgets, either
directly or from an optional, caller-selected JSON policy:

```sh
excession agent locate \
  --repo /path/to/checkout --rev HEAD \
  --task "trace the query compiler and virtual machine" \
  --budget-file config/agent-locate-budgets.example.json \
  --index-segments 8 --limit 32 --json

# Individual flags override the JSON policy:
excession agent locate \
  --repo /path/to/checkout --task "query planner" \
  --max-file-bytes 8388608 \
  --max-segment-bytes 33554432 \
  --max-segment-files 1000 \
  --index-segments 8
```

`--max-file-bytes` controls the maximum allocation for reading **one Git
source blob**. It defaults to 1 MiB; source files larger than the selected
budget remain explicitly unsupported (`LARGE`) until a larger budget is
selected. **Streaming/chunk indexing for files exceeding a bounded read remains
unimplemented.** The file budget cannot exceed `--max-segment-bytes`.

`--max-segment-files` (default 6000) and `--max-segment-bytes` (default
32 MiB) bound **new source work during one segment refresh**. Files that
exceed the current refresh budget are marked `BUDGET`, not permanently
excluded. The same invocation or a later invocation can reuse all already
indexed Git-authenticated files and continue with the deferred files.
A per-segment transaction commits after each refreshed segment, so a
subsequent invocation can resume without repeating completed segments.

`--index-segments` (default 8, maximum 256 logical hash partitions) selects
how many stale/incomplete segments to process per invocation. It is an
execution budget, not a claim that the entire repository fits in 256 work
units. A segment with deferred files can require multiple passes.

All three indexing limits are included in the segment fingerprint. A change
rechecks the same immutable Git revision rather than incorrectly reusing
an older `LARGE` or `BUDGET` decision. The report includes the effective
policy under `index.limits`.

The policy file must be explicitly selected by the caller. Excession does not
trust a policy discovered in an untrusted target repository; unknown JSON
properties, non-positive values and inconsistent budgets are refused.

## How to interpret coverage

- `index.total_source_files`: eligible source inventory for this locator.
- `indexed_source_count`: source blobs available in the search index.
- `skipped_source_count`: currently unavailable sources, including unsupported
  blob encodings or file-level budget exclusions.
- `index.pending_segments`: partitions that need additional work, including
  partially indexed partitions with deferred `BUDGET` files.
- `index.complete`: source indexing is complete only when none of those gaps
  remain; this is **not** semantic, compiler, runtime, or test completeness.
- `coverage.status=PARTIAL`: lexical matches and any observed references are
  still advisory, even after indexing is complete.

Neither a zero-result query nor a successful GitHub Actions workflow can prove
an implementation is absent.

## Existing boundaries outside source indexing

| Concern | Current behavior | Required next improvement |
| --- | --- | --- |
| Query results | `--limit` accepts 1–64; `reading_plan` contains at most 12 entries | Deterministic, snapshot-bound page cursors; no huge single response |
| Agent context | Up to 16 focus paths and 64 relationships per response | Continuation across pages with exact source and evidence identities |
| GitHub-native execution | 240-second native process and 8 MiB report caps; 30-day artifact retention | Explicit supported budgets, per-partition artifact checkpoints and continuation |
| Very large Git blobs | Can configure in-memory read cap but oversized files are not chunk-indexed | Authenticated streaming reads and byte/line-aligned chunks |
| Configured compiler analysis | Provider-specific limits and configured inputs | Native producer checkpointing with explicit per-configuration coverage |
| Repository scale | 256 logical hash groups, with resumable partial refreshes | Dynamic subdivision for pathological group distributions |
| External receipts | Type/size limits protect ingestion | Bounded, independently authenticated receipt pages |

The next work must implement **pagination and resumable execution** at
the relevant existing APIs, not create another global index, new parser or
second graph database. Budgets should be visible in every report and
tunable by the authorized caller or execution environment. Those promises
are *not* all implemented merely because the locator's budgets are
configurable.

## Security and reliability

Memory ceilings, CPU timeouts and report-size limits belong to the
execution environment. They are not assertions of source absence.
Untrusted repository files must not be allowed to raise worker privileges or
resource ceilings. Source analysis reads pinned Git objects, not source code
executed in the host process. `LARGE`, `NON_UTF8`, `BUDGET`,
`SEGMENT_INDEX_INCOMPLETE` and other gaps must remain visible.
