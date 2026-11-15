# Source-bound retrieval evaluation

This is a **research acceptance benchmark**, not a production ranking path. It compares Excession's existing **Git-backed FTS5/BM25** file retrieval with **direct, producer-observed SCIP graph expansion** on an external repository at an exact commit.

## Fixed corpus

- Upstream: `python-hyper/h11`
- Commit: `62c5068c971579d61fa1b55373390e12f25fd856`
- Producer: `@sourcegraph/scip-python@0.6.6`
- SCIP decoder: `scip-code/scip` CLI release `v0.10.0`, using official `scip print --json`
- Source identities: the Git tree's exact commit, blob, file mode, and source anchors, checked independently of SCIP before scoring.
- Labels: 16 positive, 2 negative queries in [h11-gold.json](h11-gold.json). They are **author-seeded, not independently reviewed**, and are not derived from observed SCIP relations. The benchmark therefore reports descriptive numbers only, never a statistically or semantically validated retrieval improvement.

The pinned indexer versions are fixed at top level. The deep workflow records SHA-256 of the downloaded SCIP release, npm lockfile, and resulting source index. This is sufficient to *audit the run*, but not a hermetic dependency lock across environments.

## Retrieval comparison

For a query, both approaches consume the **same existing** `agent locate` output at the same exact Git commit and source inventory:

- **Lexical:** first five BM25-ranked source paths.
- **Graph-first hybrid:** top BM25 path; then up to two distinct direct source neighbors from the existing Git-bound `relationship_candidates`; then remaining BM25 paths, with a hard total budget of five.

The hybrid does **not** score language-model similarity, invent edges, claim semantic correctness, or change the core candidate ranking. Source-bound `SCIP_REFERENCE` observations retain `OBSERVED` assessments and their original native receipts; static import edges remain `CANDIDATE`. A producer observation is not automatically a correct semantic dependency.

The Rust evaluator in `core/examples/retrieval_scip_benchmark.rs` checks the gold anchors against raw Git blobs and evaluates path-level recall@5, reciprocal rank@5, per-query hit counts, and non-answer false retrieval. It fails if a real native SCIP observation does not reach at least one evaluated query. The negative cases measure whether unsupported queries still return candidates, not whether the ranking model can correctly abstain. All per-task results are retained, including cases where graph expansion reduces recall.

## CI

[real-scip-retrieval.yml](../../.github/workflows/real-scip-retrieval.yml) obtains the exact upstream commit, executes `scip-python`, converts via the official SCIP CLI, **ingests through Excession's native evidence store**, verifies the snapshot is auto-discoverable, then runs the Rust scorer. If ingestion is unsupported or exceeds its 4 MiB input cap, the job refuses to report an accuracy result. The workflow uploads evidence and timing receipts, and does not alter the upstream source tree.

## Promotion gates

This initial dataset is far too small and has not been independently adjudicated. Before adding graph expansion to default ranking, require:

1. Independent human adjudication of query intent and gold paths without viewing model results or SCIP edges, with disagreements recorded.
2. Larger unseen queries from at least three distinct repositories and two source languages.
3. Equal retrieval budget with meaningful improvements in recall and MRR, without material regression on negative/adversarial cases.
4. Repeated cold/warm measurements with explicit corpus, version, CPU, peak RSS, evidence-size, and latency costs.
5. No stale Git identity, forged native receipt, cross-revision leakage, or silent semantics promotion.

Keep the evidence-first retrieval output in Excession regardless of whether the hybrid wins. A negative result is a useful result.
