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

`--max-file-bytes` is retained as the CLI name for the **per-source-chunk
buffer ceiling**, not a maximum source-file length. Large Git blobs are now
read in bounded, verified UTF-8 windows and stored as FTS5 chunks. The effective
chunk size is at most 64 KiB, the configured `--max-file-bytes` (default
1 MiB), and the per-refresh segment budget. Source sizes do not determine
index eligibility. Text-index byte budgets must be at least 256 bytes, allowing the
indexer to preserve all supported search identifier tokens across chunk
boundaries even for long one-line sources.
A source remains unavailable to search until all its chunks are authenticated
and indexed. That avoids presenting a partial source as the complete file.

`--max-segment-files` (default 6000) and `--max-segment-bytes` (default
32 MiB) bound **new source work during one segment refresh**. Files or file chunks that
exhaust the current refresh budget are marked `BUDGET`, not permanently
excluded. The same invocation or a later invocation reuses previously
verified chunks and resumes from the next missing chunk. The original Git blob
is re-streamed and its complete identity verified before any additional
chunk checkpoint is committed.
A per-segment transaction commits after each refreshed segment, so a
subsequent invocation can resume without repeating completed segments.

`--index-segments` (default 8, maximum 256 logical hash partitions) selects
how many stale/incomplete segments to process per invocation. It is an
execution budget, not a claim that the entire repository fits in 256 work
units. A segment with deferred files can require multiple passes.

All three indexing limits are included in the segment fingerprint. A change
rechecks the same immutable Git revision rather than incorrectly reusing
an older `BUDGET` decision. Completed, verified chunks remain reusable
regardless of the budget used to read them. The v3 chunk cache has a separate
filename, leaving older per-file caches untouched. The report includes the effective
policy under `index.limits`.

The policy file must be explicitly selected by the caller. Excession does not
trust a policy discovered in an untrusted target repository; unknown JSON
properties, non-positive values and inconsistent budgets are refused.

## Remote GitHub-native continuation

A successful, index-producing owner-originated `locate` job publishes a
derived SQLite index to GitHub Actions cache. A later **sequential** request
at the same source and Excession tool commit and identical indexing budgets
can restore that data and continue pending partitions; use `"enrich": true`
to require indexing when Git-only retrieval would otherwise suffice.
The indexer independently checks Git object and partition fingerprints.
The cache is never a proof of semantic completeness and may be evicted or
fork under parallel calls. Each report remains explicitly partial when any
source partition is pending.

This is reusable **best-effort continuation**, not durable artifact retention
with a guaranteed lease or cross-tool-version migration. CI and native process
timeouts are still enforced; missing checkpoints cannot be reported as
complete.

## Remote GitHub-native requests

The owner-originated `Excession analysis request` issue body can include
an optional `budgets` object for a `locate` operation, with
`max_file_bytes`, `max_segment_bytes`, `max_segment_files` and
`refresh_segments`. The workflow passes these as validated CLI arguments,
and verifies that the returned report lists the same values. See the
[GitHub-native request example](GITHUB_NATIVE_ANALYSIS.md#configurable-indexing-budgets-for-locate-requests).
This does not relax the worker's separate execution/response safety envelope.

## How to interpret coverage

- `index.total_source_files`: eligible source inventory for this locator.
- `indexed_source_count`: source blobs available in the search index.
- `skipped_source_count`: currently unavailable sources, including unsupported
  blob encodings or deferred chunks due to the current work budget.
- `index.pending_segments`: partitions that need additional work, including
  partially indexed partitions with deferred `BUDGET` files.
- `index.complete`: source indexing is complete only when none of those gaps
  remain; this is **not** semantic, compiler, runtime, or test completeness.
- `coverage.status=PARTIAL`: lexical matches and any observed references are
  still advisory, even after indexing is complete.

Neither a zero-result query nor a successful GitHub Actions workflow can prove
an implementation is absent.

## Authenticated streaming substrate

The Git adapter now provides two bounded streaming entry points for consumers:
\`BlobReader::stream_verified\` yields fixed-sized byte windows and incrementally
checks the *whole* original Git object's SHA-1 identity, including its
length-prefixed header. \`stream_text_chunks_verified\` turns the stream into
UTF-8-safe text windows with original byte and line offsets. Both refuse
malformed framing and invalid source data.

A callback is **provisional** until the whole blob is verified. Consumers
must stage index writes in an atomic transaction and commit only when the
stream returns successfully. The text windows concatenate without gaps or
duplication; an FTS producer must separately handle token continuation at
a very long line split, and build paged search results.

The locator now stores authenticated text windows as persistent FTS chunks,
checks every returned window against the full pinned Git blob, and resumes
incomplete files on subsequent invocations. **The per-invocation time budget is
not yet resumable across machines or temporary GitHub workers.** Follow-up
#280 addresses portable continuation receipts and snapshot-bound pagination.

## Existing boundaries outside source indexing

| Concern | Current behavior | Required next improvement |
| --- | --- | --- |
| Query results | `--limit` accepts 1–64; `reading_plan` contains at most 12 entries | Deterministic, snapshot-bound page cursors; no huge single response |
| Agent context | Up to 16 focus paths and 64 relationships per response | Continuation across pages with exact source and evidence identities |
| GitHub-native execution | 240-second native process and 8 MiB report caps; 30-day artifact retention | Explicit supported budgets, per-partition artifact checkpoints and continuation |
| Very large Git blobs | Source chunks are indexed and checkpointed within bounded buffers; complete re-streaming may still be expensive | Add bounded-time incremental verification and avoid replaying the entire blob to resume a chunk |
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
executed in the host process. `NON_UTF8`, `BUDGET`,
`SEGMENT_INDEX_INCOMPLETE` and other gaps must remain visible.
