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

For attribution, the v2 evaluator additionally runs **SCIP-only** and
**static-only** graph-first variants, each under the same five-file limit.
Only edges whose original producer assessment is `OBSERVED` enter SCIP-only;
`STATIC_IMPORT` remains a heuristic and enters static-only. This avoids
misrepresenting the effects of static imports as an SCIP accuracy result.

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


## First execution: pinned h11, 2026-11-14

The exact-head [GitHub Actions benchmark run](https://github.com/laurajoyhutchins/excession/actions/runs/43660179881) **passed**, and the recorded result is negative for graph-first ranking. The full source-bound [evidence ZIP](https://github.com/laurajoyhutchins/excession/actions/runs/43660179881) contains per-query paths and native receipt digests.

- Upstream h11 commit: `62c5068c971579d61fa1b55373390e12f25fd856`
- SCIP producer: `scip-python 0.6.6` via the official `scip 0.10.0` decoder
- Git-bound SCIP ingestion: 2,829 matched cross-file references, 1,143 unresolved; retained snapshot `657fef9b94a6412dd9f166c15b26b6c1685402e7e265c07ab1623234bfe4b583`
- 16 positive, two negative author-seeded queries; 24 relevant paths total
- **BM25 recall@5: 0.750000**, **graph-first recall@5: 0.541667**
- **BM25 MRR@5: 0.528125**, **graph-first MRR@5: 0.443750**
- Negative cases producing a nonempty retrieval result: 1/2 under both policies
- Only **one of 18 queries** exposed native SCIP relations as additional direct neighbors, totaling four native edges. Other `relationship_candidates` arose from static import candidates. Consequently the graph-first column measures a **mixed static/SCIP expansion policy**, not SCIP-only gain.
- The Rust evaluator ran for **5:12.20** wall-clock, 82.78 user CPU seconds, and reported 47,448 KiB peak resident memory for the evaluation process. Its maximum single-query time was 18,434 ms. This is end-to-end query overhead including source-bound native evidence replay, not a neural model inference time.

**Interpretation:** Do **not** promote the graph-first ranking. The negative result does not disprove the value of SCIP in other corpora, but it demonstrates that simply placing relationship neighbors above lexical matches can degrade retrieval. The evaluation labels are also unreviewed and too few to support a general accuracy claim.

**Next experiment:** Quantify native SCIP-only neighbors independently from static-import candidates, use externally adjudicated queries and multiple repositories, and compare identical source-path inspection budgets. Performance work should eliminate repeated Git subprocesses for the same already-authenticated path inside one exact-revision replay, without weakening source identity checks. Measure before and after, keeping this run as the baseline.

The first recorded run used evaluator v1 (mixed graph expansion only). The v2 arms are a follow-up diagnostic with **unchanged queries and gold anchors**, not a replacement of the archived result.
