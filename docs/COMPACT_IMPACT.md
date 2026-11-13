# Compact source-impact witness forest

The original `excession impact` report enumerates individual reverse-dependency
paths. Its eight-hop and 512-path limits are useful for bounding explanatory
output, but they are not sound bounds on *which sources are affected*. Layered
dependency graphs can have exponentially many simple paths and only linearly
many distinct sources. Stopping the path walk also used to stop processing
later revisions.

This implementation keeps the legacy `effects` and their replayable claim
IDs, and adds a separately calculated `candidate_source_impacts` inventory.
There is **one witness per distinct non-seed source per revision**.

## Witness interpretation

Each source impact record contains:

- `entity`: Git-pinned source identity at the reported revision.
- `next_hop`: the original producer-provided observed edge, source identity,
  target identity, relationship kind, and evidence. No evidence is invented.
- `distance_to_change`: the shortest observed path length to a changed seed.
- `assessment: CANDIDATE_PATH`: source reachability is not behavioral failure.

A changed seed has implicit rank zero. A witness of rank `d > 0` points to a
changed seed if `d = 1`, otherwise to a witnessed source of rank `d - 1` at
the *same revision*. This decreasing rank is an independently checkable
structural reachability certificate. The report is bound by its existing
`report_id` and retains its original revision-specific producer evidence.

The reverse graph search is multi-source BFS. For each candidate source it
records the first parent hop, and never visits that source again. Its
deterministic ordered-map/set implementation takes
`O((|V| + |E|) log |V|)` time for the graph traversal and `O(|V| + |E|)`
working memory, excluding native indexing and report serialization. The
witness *forest* itself has at most `|V|` edges, not one object per possible
path. This is a standard graph algorithm used to correct a product-level
path-enumeration failure, not a claimed new graph-theory theorem.

The inventory has an independent 10,000-entry output cap.
`candidate_source_count` is the exact (base-plus-head) count prior to
bounding. `candidate_source_enumeration_complete` is **true only if every
discovered source is present in the emitted inventory**. If output is capped,
the JSON report marks this false and records a `TRUNCATED` coverage gap.
A legacy path truncation can occur while source enumeration remains complete.

## Proof boundary

**Soundness relative to the observed graph:** every emitted record links to a
changed seed through strictly decreasing ranks and authentic original
observation hops, assuming the analysis reports themselves passed Excession's
existing identity/evidence checks.

**Completeness relative to the observed graph:** if a source has any
directed dependency path to a changed seed, induction on its shortest distance
shows that BFS eventually visits it. This guarantee holds before the optional
10,000-entry *output* cap.

**Not proved:** the observed graph is a complete model of the program;
the imports are runtime-executed; source changes will break a dependent;
build/test receipts are independently authentic; or unlisted tests are safe
to skip. Excession continues to report `coverage.status=PARTIAL` and
`safe_to_skip_unlisted_tests=false`.

## Regression and independent acceptance

The Rust integration regression constructs seven 3-wide layers of Python
modules, with every module in a layer importing every module in the previous
layer. It changes the root between two Git commits. Each revision has only
21 distinct dependent source files but 3,279 simple paths, all within the
legacy eight-hop bound. The old `effects` list stops at 512; the compact
inventory must retain all 42 revision-specific source witnesses. The test
also independently checks each witness's strictly decreasing rank and its
Git revision identity.

This is deliberately a *graph adversary*, not an independently measured
large-repository advantage. Hold-out real-repository validity, operational
performance, and user-visible prevention of coding-agent mistakes remain the
separate gates under issue #84. Do not market this result as a completed
independent codebase evaluation.
