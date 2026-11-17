> **Implemented building blocks for investigations.** The
> [investigation contract](INVESTIGATIONS.md) describes the target
> product artifact; these current commands return bounded source
> candidates, reading plans, context, and revision evidence, not an
> automatically generated behavioral explanation. See the
> [practical workflow](CODEBASE_UNDERSTANDING.md).

# Agent-facing CLI v1

## Locate source from a task (Git-pinned, heuristic)

```sh
excession agent locate --repo /work/project \
  --task "Fix worker lease recovery" --rev HEAD --limit 20 --json
```

`agent locate` accepts natural-language tasks without requiring prior exact
file paths. It maintains a **persistent, incrementally updated SQLite FTS5 index** from
authenticated regular Git source blobs and ranks candidates using BM25.
Each candidate carries an exact commit, tree, path, blob, file mode, and a
1-based **heuristic** source line window.

The response also contains `reading_plan`: at most 12 distinct Git-pinned
files ordered to inspect task matches first, then connected possible callers
and dependencies, then filename-suggested tests. Each step contains its
original array reference (`candidates`, `relationship_candidates`, or
`context.test_candidates`), observation basis, and a concrete inspection
question. Nothing in the plan is labeled verified behavior. The plan does not
perform source interpretation, resolve dynamic calls, or replace an agent's
inspection of actual source. See [codebase understanding](CODEBASE_UNDERSTANDING.md). The result also contains the
existing `agent context` report for up to the first 16 candidates, including
its native relationships, test suggestions, and coverage gaps. Empty results
do **not** prove the requested behavior is absent.

When the existing evidence database contains source-bound SCIP observations or
configured graph evidence for the exact commit, `agent locate` now consumes
these through the default `agent context` analysis path. Direct, source-bound
SCIP references and static import candidates connected to a lexical hit are
reported separately in `relationship_candidates`, with native producer
receipts and `OBSERVED` versus `CANDIDATE` assessments preserved. They are
**not** folded into the BM25 ranking or promoted to verified program behavior.
Configured graph membership retains its own source/entity meaning in context;
it is not silently treated as a source-to-source edge.

For very large, mixed-language repositories, use `--roots fs` (or another
specific source root) to bound the auxiliary relationship analysis. The
segmented lexical index still sees the full tracked Git source inventory,
subject to explicit index coverage and size limits.

Source paths are partitioned deterministically into **256 SHA-256(path)
segments**. The *default processing budgets* are 6000 newly read files per
segment refresh, 1 MiB maximum per text chunk (actual windows at most 64 KiB),
and 32 MiB of newly indexed text per
segment refresh. These are caller-configurable with `--budget-file` or
`--max-file-bytes`, `--max-segment-bytes` and `--max-segment-files`.
A budget-exhausted segment is checkpointed and continues on subsequent runs;
the file-read cap still excludes larger blobs until the caller raises it.
See [resource budgets and remaining limitations](RESOURCE_BUDGETS.md).
By default, an invocation refreshes up to eight changed or incomplete
segments. To process up to 256 segments in one foreground run:

```sh
excession agent locate --repo /work/project \
  --task "worker lease recovery" --index-segments 256 --json
```

A cold or revision-switched index may be **partial**. The `index` output
records populated, ready and pending segments and whether source indexing is
complete. `SEGMENT_INDEX_INCOMPLETE` is reported when additional calls are
needed. Stale segments are excluded from results, and an empty candidate set
never proves that behavior is absent. A fully refreshed index still reports
unsupported source, non-UTF-8, oversized, and semantic verification gaps.

The cache lives outside the source checkout in
`agent-locate-segments.sqlite` alongside Excession's existing SQLite
cache. It reuses unchanged Git blobs, updates changed/deleted paths within
transactions and verifies the blob identity of returned evidence.
`cache` records local read/reuse/refreshed counts; these incidental values
are excluded from `report_id`. The working tree remains unread and
unmodified. No neural inference is performed. `--roots` and `--db`
affect existing relationship/context analysis, not the whole-tree lexical
inventory. See `contracts/agent/locate.schema.json` for the output contract. The output
contract is `contracts/agent/locate.schema.json`.


Excession exposes two bounded JSON commands for coding agents. Both reuse the
existing Git, SQLite, static analysis and impact computation. They do not start
agents, execute tests or builds, perform semantic ontology inference, or merge
candidate revisions.

## Context

```sh
excession agent context --repo /work/project --rev BASE_COMMIT \
  --focus src/service.py,tests/test_service.py \
  --task "Change the service retry policy" --limit 32
```

`--focus` requires 1–16 exact, tracked Git paths, not natural-language
search. The optional `--task` is metadata only. The command authenticates
source commit/tree/path/blob/mode, selects directly touching static import
candidates from the existing analyzer, and suggests tests using an explicitly
unverified path-name heuristic. The full native relationship evidence is
preserved. `--roots` and `--db` pass through to the normal analyzer.

The response has schema `excession.agent-context.v1`, maximum 64 returned
relationships and 64 test candidates, original counts, truncation and coverage
gaps. The relationship budget now retains producer-observed `SCIP_REFERENCE`
source edges **before** static-import candidates and retains one native
observation from each distinct directed source-to-source pair before repeating
a pair. This prevents the analyzer's static-first emission order, as well
as repeated references between the same two files, from consuming every
bounded result slot before other native source dependencies are surfaced. It does not rank a source
match as semantically correct or infer an unobserved dependency.
`observed_reference_count` and `returned_observed_reference_count` separately
report the native reference total and the retained subset. Consumers must still
inspect truncation and original producer receipts.

The independent external [h11 native-reference benchmark](../tools/benchmark_dependency_retention.py)
compares old static-first and new distinct-native-first retention at exactly
the same 16-relationship budget and five preregistered source paths. It reports
both native observation retention and **distinct directed source-pair retention**.
These are source-authenticated SCIP observations, **not** semantic dependency
accuracy.
The deep `native-dependency-discovery.yml` workflow preserves the producer and
Git identity receipts. A Rust-only project currently has an exact source inventory but no
Python/JavaScript static-module relationships; that is reported as
`NO_SUPPORTED_STATIC_MODULE_INDEX`, not as "no dependencies".

## Verify an isolated candidate commit

```sh
excession agent verify --repo /work/project \
  --base BASE_COMMIT --candidate CANDIDATE_COMMIT \
  --receipts /tmp/agent-receipts.json --limit 32
```

Both revisions must be retained Git commits. No checkout, branch, worktree,
build or test is created. The command recomputes the source-only v3 impact
report against the two pinned commits, cross-checks the change inventory
digest, carries one rank-certified witness per observed dependent source plus
coverage gaps, and preserves all unresolved obligations. It reports the source recomputation separately from
semantic correctness. The optional `--roots` and `--db` flags select the
same underlying analyzer as `excession impact`.

**External receipts are observations, not attestations.** They must come from
an independently run test/build process. A plain JSON receipt cannot
authenticate its producer or prove execution isolation. It is rejected if it
refers to the wrong commit/tree, is malformed, reports `PASS` with a nonzero
exit code, or duplicates another record. The declared log SHA-256 is not
verified against log bytes by this command. Even accepted passing receipts
leave `RECEIPT_AUTHENTICITY_UNVERIFIED` and semantic/coverage obligations
open. The command never emits `ready_to_merge: true`.

Example `/tmp/agent-receipts.json` (replace the placeholder hashes and
revision identities with real ones):

```json
{
  "schema": "excession.execution-receipts.v1",
  "receipts": [
    {
      "kind": "TEST",
      "status": "PASS",
      "source_commit": "0123456789012345678901234567890123456789",
      "source_tree": "0123456789012345678901234567890123456789",
      "producer": "isolated-worker",
      "command": "cargo test --locked",
      "exit_code": 0,
      "log_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }
  ]
}
```

`log_sha256` is a digest declared by the external producer, not a signature.
Use the exact candidate commit/tree values from `excession changes` or Git.
No command in a receipt is executed by Excession.

## Working-tree tests without commits

For a locally edited project, use the native-test workflow rather than
inventing a commit or running tools directly against the agent's checkout:

```sh
excession tests --doctor --json
excession tests --repo /work/project --json
```

Check `schema=excession.test-workflow.v2`, `status=READY`,
`safe_to_skip_unlisted_tests=false`, and the `working_tree` object.
`next_argv` contains a source fingerprint and an immutable, preinstalled
sandbox image ID. **Execute the complete returned argument vector**, not a
reconstructed shell command, only when execution is authorized. If the
fingerprint changed, re-plan and review before running anything.

`--doctor` reports setup blockers without requiring a repository or
executing project code. A developer can explicitly provision the
sandbox once with `sh tools/bootstrap-test-sandbox.sh` from an Excession
checkout; that opt-in setup uses the network. Normal test runs never pull
images. When no explicit image is supplied, Excession only recognizes the
labeled local sandbox and pins its digest. Missing runtime, image or
dependencies cause a refusal; there is no trusted-host fallback for
uncommitted code. A missing Python testmon baseline is generated from
the committed base **inside the sandbox**, not in the developer checkout.

The resulting snapshot and test receipts are ephemeral local evidence,
not persisted commits or independently attested receipts. Do not pass
their ephemeral commit ID to `agent verify` after the temporary clone is
deleted. `agent context` and `agent verify` still operate on durable
committed Git revisions.

## Protocol boundaries

- Context and verification both accept `--limit` in 1–64 (default 32).
  `--limit` bounds emitted candidate arrays. `agent verify` always
  includes total change and impact counts; an over-limit result is truncated,
  never silently complete.
- At most 32 receipts in a 1 MiB JSON input. Files are never overwritten.
  Git source is read from committed objects, not working tree edits.
- Unknown static relationships, dynamic calls, native tool validity and
  behavioral equivalence remain unresolved. No candidate path implies a
  test failure, and no omitted test is safe to skip on this basis.
- The v1 JSON contracts are in `contracts/agent/`. Consumers must check
  `schema`, pinned Git identities, `coverage` and `obligations` instead
  of interpreting a zero-length candidates array as success.

The implementation lives in `core/src/agent.rs` and calls the existing
`analyze::run_if_supported`, `change_inventory::inventory`, and
`impact_query::run`. Future native SCIP, test ownership and stronger
attestation can enrich this envelope without changing source authority.
