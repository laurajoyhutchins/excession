# History reorganization: evidence-preserving workflow

This is a procedure for building a new target architecture and *then* reorganizing Git ancestry around it. There is intentionally no command that moves refs or rewrites commits in this first slice.

## Implemented, read-only commands

Run against a local Git SHA-1 checkout:

```sh
excession history plan --repo . --rev HEAD --max-commits 10000 > /tmp/excession-history-plan.json
excession history audit --repo . --plan /tmp/excession-history-plan.json
```

Use --base BASE to exclude everything reachable from an ancestral commit. Omit it for the complete reachable ancestry of the selected tip.

Additional read-only inspection commands:

```sh
excession history refs --repo . --max-refs 10000
excession history verify-target --repo . --target architecture-target --replacement reconstructed
```

The refs report inventories *locally available* Git refs and explicitly refuses
to claim external GitHub PR heads, unfetched tags or third-party receipts are
complete. verify-target checks the exact Git tree identity of two pinned commits.
Even an identical tree does not prove an equivalent historical process, build,
review or evidentiary chain.

The plan includes the original tip and tree SHA-1, original parents and tree SHA-1 per commit, declared subject scope, first-parent unchanged-tree signals where known, merge boundaries and adjacent same-scope groups. The canonical report_id binds the report contents. The audit re-reads the exact pinned ancestry and refuses modified or inconsistent plans, even if their report_id is recomputed.

Scope grouping is a *review hint* based on commit subjects and adjacency, not evidence that two commits can be squashed safely. It will not treat unscoped messages as an architectural group. Truncation and shallow-clone ancestry are explicit. A scan of 100,000 commits is allowed, but a truncated or shallow scan cannot authorize a rewrite. Commit metadata and report output are bounded to 64 MiB. The command does not inspect working-tree edits.

## Full history-rewrite acceptance contract (next phase)

1. Freeze and independently verify the target source tree. Record its exact tree ID and proof receipts.
2. Inventory all relevant refs: main, release tags, active PR heads, annotated notes, archival refs, nested/submodule references and external source/artifact receipts. Do not claim these are exhausted from a local branch-only scan.
3. Generate a plan from the entire source DAG with original commit IDs and parent edges. Identify duplicate tree states, empty commits, merge commits, supersession, disconnected branches and cross-feature dependencies. Subject/path similarity alone is not a safe rewrite rule.
4. Propose a dependency-respecting topological schedule. Rewrite in a detached, isolated repository or worktree, never in the user's source checkout. Preserve author identity and authorship provenance; do not fabricate review or signatures. Every new commit gets an old-to-new identity mapping and its exact old/new source-tree relationship.
5. Reject any change that silently changes final source tree, drops a reachable change, creates a cycle, loses a merge dependency or makes a historical evidence receipt appear valid against a different commit ID. Merge conflict resolution must be independently verified.
6. Check exact final tree equality, replay enabled contracts, run independent upstream fixtures, compare behavior and cost, and preserve an immutable archive namespace for the original DAG.
7. The maintainer separately reviews the mapping and decides whether to publish a curated branch. Ref updates, if ever enabled, must be guarded by expected old SHA, be explicitly authorized and never default to main.

## Limits of the first slice

The current Rust history planner checks Git source-DAG identity and proposed contiguous same-scope batches. It does not yet inspect PR review provenance or native code semantics, perform an actual topological rewriting, reconcile external receipts, or prove replay equivalence. Its machine output says rewrite_authorized=false. That is a deliberate safety boundary, not an implicit promise.

## Scale and storage direction

Use streaming Git revision traversal and batched object reads for future DAG/patch-ID indexes. Cache derived graph observations under immutable source identities in SQLite. Keep a bounded working set and emit paginated plans, with explicit coverage and continuation IDs. Cost is expected to be near O(V+E) for ancestry traversal; substantive semantic verification depends on the independent producers and cannot be assumed cheap.

Git remains the immutable source authority. SQLite can record proposals and their proof obligations, but no SQLite entry changes Git history on its own.
