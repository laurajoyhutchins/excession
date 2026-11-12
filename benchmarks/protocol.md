# Benchmark contract: observable relational information, not agent productivity

The primary question is whether reusable, Git-bound repository relations justify
their total cost against a conventional indexed graph and ordinary Git tools.
The primary experiments use deterministic outputs and an independently authored
relationship specification. There is **no task-completion score**, preference
rating, qualitative answer assessment or combined "winner" score.

## Four questions

| Axis | Directly observable quantities | Required control |
| --- | --- | --- |
| Relationship retrieval | Exact edges, modules and file sets; missing and extraneous edges; output bytes, process count, CPU, elapsed time | Separate source-written oracle; both arms use the same immutable revision and index |
| Reuse across queries/revisions | New index time and disk growth at each revision; aggregate time for many repeated queries, warm/cold observations | Include every index/re-index in cost, keep revision commits pinned |
| Evidence integrity | True-positive corruption refusals, false-positive refusals, hash audit work, elapsed and CPU costs | Compare only shared verification guarantees; distinguish Git-path checks from full view replay |
| Scaling | Costs against modules, fanout, path depth, revision count and number of related queries | Vary dimensions independently and retain per-observation source, tool and runner identities |

An experiment failing coverage or refusing a query stays in the denominator.
A source-set match is not proof of semantic correctness unless the independent
oracle matches too. Unsupported operations are labeled unsupported, not zero
cost. Same-index comparison measures projection overhead; it cannot establish
whether the index describes all language-level relations.

## Measurement rules

1. Report wall-clock, CPU, bytes emitted, number of process launches, index
   construction and update cost, index disk bytes, failures, and revision/tree
   identities **separately**. Do not invent model-token or peak-RSS values.
2. Evaluate exact relations against oracle data defined *before* indexing. For
   natural repositories, use an independently reviewed fixed source/edge set;
   don't generate ground truth from Excession's tables or manifests.
3. Keep the retrieval contract identical between competitors. A multi-relation
   bundle implemented as one view invocation versus multiple graph invocations
   is a call-overhead experiment, not a pure algorithmic comparison.
4. For integrity, list the invariants actually checked by each method. Git
   object/path authentication, SQL consistency, and replay of a canonical
   derived-view commitment are different guarantees. Do not equate them.
5. Produce source bundles, command/version identities, machine-readable
   per-observation receipts, and explicit missing/extra relationships.
6. Compare cold setup and repeated queries as a cost surface, not a single
   latency ranking. Report total time for one, five, twenty and one hundred
   queries only where the workload actually measures those counts, without
   extrapolating cached behavior.

## Experimental status

- **Executable:** `excession benchmark run-relational` (Rust) constructs pinned multi-revision
  Git histories from a source-written relationship oracle, independently checks
  the indexed edge set and times paired graph/compact-view source queries.
  Size, fanout, revision count and repetition count are configurable. Index
  setup is measured per revision. Both query arms run in the same Rust CLI.
- **Existing microbenchmarks:** `micro/` preserves exact-file lookup, LSP
  definition navigation, native source-set parity and authenticated projection
  phases. Their results are calibration, not acceptance evidence.
- **Partial evidence controls already present:** `experiments/checked_tree_ab.py`
  compares authenticated before/after outputs and selected tamper refusals;
  `experiments/real_fixtures.py` independently checks Git source bindings.
- **Independent full indexed-query replay implemented:**
  `experiments/independent_audit.py` compares a separate Git/SQLite verifier
  against native replay for intact and corrupted h11 evidence.
- **Still incomplete:** true cached incremental view maintenance,
  complete real-repository runtime relationships, portable peak memory
  measurement, and comprehensive adversarial coverage beyond the bounded cases.

The first experiment does not prove that Excession reuses computation between
queries. It only records a workload where such reuse could be observed. Evidence
integrity should be independently tested before treating faster unverified
retrieval as equivalent.

## Versioned reporting

`benchmarks/schemas/benchmark-report.v1.schema.json` is the normative
Draft 2020-12 schema for normalized reports. Producers must identify the
executable by SHA-256 and source revisions by Git commit/tree IDs, retain
raw artifacts with byte counts and SHA-256 hashes, and report metric unit,
statistic, sample count, and status. Unmeasured or unsupported values require
an explicit reason, never a fabricated zero.

The validator checks structure, report identity, referenced artifacts and
passed/failed check consistency. The report ID digests canonical JSON contents
excluding itself and is **not** a signature. An independently authored oracle
and a shared-index comparison must never be conflated. Validation cannot
establish the semantic truth of supplied source or claims.

## Independent full-query audit control

`experiments/independent_audit.py` is a second, separately implemented
Git/SQLite verifier. It parses a view manifest strictly, re-enumerates the
complete pinned Git tree, compares every indexed tree row, authenticates
module path/mode/blob claims, recomputes dependency reachability and roles,
and verifies the full canonical commitment. Its behavior is compared
against `excession view replay deps` on matching evidence and
corrupted manifests/records. This establishes source-bound **indexed query
replay**, not semantic edge truth, independent code execution, or a
cryptographically signed worker identity. Both trust Git; Python's
interpreter startup is a measurement confound.
