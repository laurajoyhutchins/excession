# Independent import dependency recall

The preceding h11 experiment improved the fraction of **already-observed SCIP
source pairs** returned by `agent context` from 18/59 to 54/59 at a
16-relationship budget. That was an evidence-delivery improvement, not an
independent dependency-correctness benchmark.

This experiment measures a distinct question: **of the direct static import
relationships independently identified in source, how many appear among the
native SCIP references returned to an agent at the same budget?**

## Repositories and ground truth

| Repository | Exact source identity | Language | Preregistered source files |
|---|---|---|---|
| python-hyper/h11 | Pinned Git bundle, exact commit emitted | Python | `h11/_connection.py`, `_events.py`, `_headers.py` |
| pallets/click | `8abb77a8432e03ed14a95f972b44f905279415a7` | Python | `src/click/core.py`, `parser.py`, `decorators.py` |
| psf/requests | `3c7baaa05ab4aa49cc819ce3a819d260de19c157` | Python | `src/requests/sessions.py`, `models.py`, `adapters.py` |
| axios/axios | `4ee8d46fc463770b8f5da718d5449a5b027b2c08` | JavaScript | `lib/core/Axios.js`, `adapters/http.js`, `utils.js` |

The exact manifests are retained in
[`benchmarks/fixtures/real-repositories.json`](../fixtures/real-repositories.json).
These cases were selected **before** scoring this experiment. They are not
randomized, human-adjudicated samples of all possible dependencies.

The ground-truth producer is separate from Excession's Rust graph:

- Python: CPython `ast.parse` resolves direct absolute and relative module
  imports to tracked source files under known package roots. No code executes.
- JavaScript/TypeScript: `typescript@5.8.3` compiler AST plus
  `resolveModuleName` resolves literal relative imports, reexports and
  CommonJS `require` calls to tracked source files. No app code executes.
- The independent oracle reads the pinned checkout only after verifying each
  file against its actual Git blob. Unknown imports are counted separately,
  not quietly treated as absent edges.
- An external language-native SCIP indexer runs on a *disposable copy*.
  Excession ingests its decoded index into the existing immutable SQLite
  evidence store and presents source-bound `SCIP_REFERENCE` relationships.
  Neither the SCIP index nor Excession generates the oracle's labels.

## Exact metrics

For every selected focus file, query Excession's real `agent context` with
`--limit 16`. Compare two policies against the **same** complete, native
relationship ledger and the same independently parsed source pairs:

1. Previous: first 16 touching relationships, including static candidates.
2. Current: actual Excession context result with observed, distinct pairs
   prioritized and remaining slots filled by repeated references and static
   candidates.

An independent scorer checks the current output's exact sequence and
Git-bound endpoints. It compares each policy's set of distinct observed
directed file pairs to the CPython/TypeScript import-pair set.

`static_import_pair_recall = confirmed_native_pairs / independently_resolved_import_pairs`

We sum per-focus opportunities, just as the previous h11 retention benchmark
did. Duplicated import pairs across multiple focus files are **not**
repository-global unique pairs; the artifact includes every per-focus result.

We also report native pairs not confirmed by the import oracle as
**unclassified**, not false-positive dependencies. SCIP references can denote
legitimate calls, types and symbols that are not direct static imports.
Therefore, the experiment does not establish precision of all dependency
edges, the absence of dynamic links or completeness of the indexer.

## Execution

The deep [`independent-dependency-recall.yml`](../../.github/workflows/independent-dependency-recall.yml)
job runs only when the evaluator or workflow changes, or manually. It builds
Excession from locked Rust dependencies, uses independently version-pinned
SCIP indexers for both languages, verifies the official decoder checksum,
runs independent-oracle unit tests, and retains:

- Each pinned Git commit and tree, input index digest, native snapshot and
  unresolved-reference counts.
- Each independent oracle's import counts and explicit unsupported cases.
- Every focus file's previous and current true import hits, unknown native
  neighbors and bounded context counts.
- The original decoded index bytes and a digest of the external toolchain.

The job **does not require an accuracy gain to pass**. A correct negative
result is an accepted scientific outcome. It does fail on stale source
identity, forged observed receipts, missing preregistered sources,
inconsistent counts, missing parser coverage or mismatched bounded selection.

Do not promote a local or tiny-fixture improvement to general dependency
accuracy. The next step, if these results are useful, is a separately
preregistered held-out cohort with independent reviewer adjudication of
non-import cross-file relationships.
