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

## First independently parsed source result

[GitHub Actions run 44001137658](https://github.com/laurajoyhutchins/excession/actions/runs/44001137658)
passed at source commit `fcd59d54aaf6ab5f96be12c4b0430a0eccf91363`.
Retained artifact: `independent-dependency-recall-v1` (GitHub artifact
`14292193299`, ZIP SHA-256
`ceadf79570827b86bbe5067d6113df1e4eb653374c999f00451870c594bfec45`).
All four decoded SCIP index SHA-256 values match their source-bound result
records; the exact commit/tree identities and observed edge receipts passed.
The independent CPython and TypeScript oracle unit tests and full Rust
verification also passed.

| Source | Previous static import hits | Distinct-native-first hits | Opportunities |
|---|---:|---:|---:|
| h11 | 13 | 22 | 23 |
| Click | 0 | 28 | 30 |
| Requests | 2 | 39 | 45 |
| Axios | 3 | 44 | 67 |
| **Total** | **18** | **133** | **165** |

Static import-pair recall across **12 preregistered source contexts**, with
16 emitted relationships per context: **10.91% → 80.61%**. These are
per-focus opportunities; paths counted in multiple focus contexts are not
globally deduplicated.

The language-native SCIP indexers produced 485, 892, 820 and 909 matched
cross-file observations respectively. They also reported **692, 4,493,
2,003 and 1,850 unresolved reference occurrences**, respectively, which
remain unresolved. The independent Python import oracle also recorded
external or unresolvable targets, and Axios's JavaScript oracle intentionally
excluded nonliteral and package-resolution semantics. The scorer's execution
excluding the initial release build took **1:18.82 wall-clock** with peak
resident memory reported as **377,056 KiB**; no comparison against a
controlled latency baseline was made.

**Supported conclusion:** the policy from Excession PR #268 returns more
*independently confirmed direct static import source pairs* at fixed context
budget in all four pinned fixtures. The graph includes many non-import
observations that are unclassified by this test. This does **not** establish
overall dependency precision, transitive or runtime recall, independent
human-adjudicated correctness, statistical generalization or safe test
omission. The prior negative graph-first BM25 ranking result remains
unchanged; this study did not rerank lexical search.

## Conservative direct-call and type-reference extension

The `--semantic` mode extends the **same** pinned four-repository, 16-slot
comparison without regenerating a second set of SCIP indexes. It introduces
`semantic_symbol_oracle.py` for Python and `ts_symbol_oracle.cjs` for
JavaScript/TypeScript. Both read the original Git checkout, independent of
Excession's Rust graph and its native SCIP index.

Python's CPython AST oracle only labels direct calls to unshadowed imported
top-level functions/classes and imported class names in type annotations or
class bases. A name stored or redefined anywhere in its source file is
excluded. Attribute calls on instances, re-exports, alias flow, monkey
patching, dynamic imports and reflection are not considered resolved.

The TypeScript 5.8.3 compiler oracle identifies direct identifiers,
constructors, namespace-imported properties, and type-reference declarations,
only when the source declaration resolves into the pinned tracked checkout.
It refuses general receiver-method dispatch. **The TS compiler family is also
used by the SCIP indexer**, so this is an independently implemented
*extraction and source-site comparison*, not an independent semantic engine
or separate runtime witness. Axios is JavaScript, so it may have zero
TypeScript type-reference labels.

For a labeled source-only call/type site, native evidence must match its
**source file, target declaration file and one-based source line**. A mere
file-pair coincidence on another line is not counted. This is still not an
identity proof for the specific symbol when two expressions share a line:
`source_target_line` is a bounded, conservative surrogate. We report three
distinct measures per kind: fraction found anywhere in the *full native
graph*, fraction retained in the previous bounded context, and fraction
retained in the revised bounded context. Missing or unresolvable sites
remain visible as unsupported.

A zero-label kind is **not** assigned perfect recall. Cross-language symbol
resolution, dynamic dispatch and behavioral call targets remain entirely
unsupported by this experiment. No claims of cross-language or dynamic-call
precision or recall should be derived from its green checks. The eventual
acceptance test for those domains needs independently held-out, reviewed
source/runtime observations.

## Direct-call/type results on pinned source, November 16, 2026

[Successful full run 44004799557](https://github.com/laurajoyhutchins/excession/actions/runs/44004799557)
at exact Excession head `2fe2bd79e82cdb524a0b04f6b22803570d0942b1`,
artifact `independent-dependency-recall-v1` (ID `14293343500`),
ZIP SHA-256 `2e464c15268e4defbf2308593565a11f71ba59f23d510cbb5b877db6b41706f3`.
Each of the four original decoded SCIP index SHA-256 values was independently
matched to its retained source report; commit and tree identities are pinned.
All eight independent oracle unit tests passed at this head.

| Independently labeled site kind | Full native SCIP hits | Previous first-16 hits | New distinct-pair-first hits |
|---|---:|---:|---:|
| Direct imported function/class calls, and TS direct constructors | 262 / 267 (98.13%) | 4 / 267 (1.50%) | 25 / 267 (9.36%) |
| Imported class type and TS type-reference sites | 115 / 115 (100%) | 6 / 115 (5.22%) | 17 / 115 (14.78%) |

The denominator is a **per-focus static symbol-site opportunity** among
12 preregistered source files. Every hit must match the independent
**source path, target declaration path and one-based line**, not merely
the two file names. Source-only oracle labels were collected without reading
SCIP or Excession. TypeScript's compiler family is shared with the upstream
SCIP producer, and no dynamic method dispatch is claimed.

The source-reported per-repository oracle populations were:
h11 73 call / 86 type sites, Click 123 / 56, Requests 129 / 47,
and Axios 147 / 1. The measured per-focus populations differ because the
12 selected contexts cover only a subset of each repository. Unsupported
calls and types remain explicit. No true-negative universe was established,
so **precision, F1 and false-positive rate are not computed**.

The full graph contains nearly all the conservatively labeled static
call/type observations. The much lower bounded result means the current
agent context is **losing discoverability**, rather than predominantly
failing to index these known sites. This is an actionable negative result:
one source-pair representative per relationship budget does not retain the
large number of distinct callsite lines within a heavily connected pair.
A dedicated symbol/line query or explicit pagination is preferable to
claiming that a 16-slot graph-neighbor list is complete.

The original import test in this run scored **18/165 → 132/165**, against
**18/165 → 133/165** in the earlier accepted run. We independently compared
the original decoded SCIP JSON from both runs: after normalizing only the
checkout-specific `metadata.project_root` URI, the decoded indexes were
otherwise identical across all four repositories. The one Axios
`lib/adapters/http.js` difference therefore indicates a
**source-root/receipt-order-sensitive bounded selection**. This does not
change any labeled source or external SCIP occurrence, but it is an
unresolved reproducibility defect. Do not report the bounded import result
as bit-for-bit stable across independent executions.

Measured scorer execution excluding the release build: **1:25.92** wall time,
peak RSS **370,976 KiB**. No exact runtime-call semantics, cross-language
link equivalence or dispatch target correctness was established.
